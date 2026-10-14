//! Consolidation en tâche de fond : quand une conversation se calme, un seul appel au
//! LLM en tire un souvenir (résumé) et met à jour les faits (ajout, correction, oubli).
//! Approche inspirée de Mem0. Ne tourne que si aucune réponse n'est en cours, pour ne
//! pas ralentir le chat sur un CPU partagé.

use std::{collections::HashSet, sync::Arc, time::Duration};

use aries_llm::{CompletionOptions, LlmProvider, Message};
use serde::Deserialize;
use serde_json::json;
use tokio::{task::JoinHandle, time::MissedTickBehavior};

use crate::{
    ActivityGauge, Fact, Memory, MemoryError, StoredMessage, ids_u64, truncate_chars,
    vectors::{EPISODES, FACTS},
};

const POLL_INTERVAL: Duration = Duration::from_secs(30);
const MAX_MESSAGES: i64 = 30;
const MESSAGE_MAX_CHARS: usize = 1_500;
const QUERY_MAX_CHARS: usize = 2_000;
const RELATED_FACTS: usize = 15;
const RELATED_FACTS_MIN_SCORE: f32 = 0.3;

const SYSTEM_PROMPT: &str = r#"Tu es le module de mémoire d'ArIès, un assistant personnel.
On te donne des faits déjà connus sur l'utilisateur (avec leur id) et un extrait de conversation.
Réponds UNIQUEMENT avec un objet JSON de cette forme :
{"episode": "...", "add": ["..."], "update": [{"id": 0, "content": "..."}], "delete": [0]}

Règles :
- episode : résumé factuel de l'extrait en 1 à 3 phrases (« L'utilisateur a demandé… »).
  Attribue chaque propos au bon interlocuteur : les lignes « ArIès : » sont dites par
  l'assistant, pas par l'utilisateur. Chaîne vide si l'échange est anodin (salutations, tests).
- Écris toujours « l'utilisateur », jamais « il » ou « elle » : son genre n'est pas connu.
- add : faits DURABLES et utiles sur l'utilisateur : identité, proches, préférences, goûts,
  projets, habitudes, objectifs, contraintes. Chaque fait est une phrase courte et autonome
  (« L'utilisateur s'appelle Vivian. »). Rien d'éphémère, rien sur ArIès, pas de doublon
  d'un fait connu.
- update : un fait connu est précisé ou contredit → son id et sa nouvelle formulation complète.
- delete : ids des faits connus devenus faux, ou que l'utilisateur demande d'oublier.
- N'invente rien : seulement ce que l'utilisateur a dit ou confirmé lui-même.
- Listes vides si rien à faire."#;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Consolidation {
    episode: String,
    add: Vec<String>,
    update: Vec<FactUpdate>,
    delete: Vec<i64>,
}

#[derive(Debug, Deserialize)]
struct FactUpdate {
    id: i64,
    content: String,
}

impl Memory {
    pub fn spawn_consolidation(
        self: Arc<Self>,
        llm: Arc<dyn LlmProvider>,
        activity: ActivityGauge,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(POLL_INTERVAL);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                if let Err(err) = self.consolidate_pending(llm.as_ref(), &activity).await {
                    tracing::warn!(%err, "consolidation de la mémoire échouée, nouvel essai plus tard");
                }
            }
        })
    }

    async fn consolidate_pending(
        &self,
        llm: &dyn LlmProvider,
        activity: &ActivityGauge,
    ) -> Result<(), MemoryError> {
        let conversation_ids = self
            .store
            .conversations_to_consolidate(
                self.config.consolidation_idle_secs,
                self.config.consolidation_batch,
            )
            .await?;
        for conversation_id in conversation_ids {
            if !activity.is_idle() {
                break;
            }
            self.consolidate(llm, conversation_id).await?;
        }
        Ok(())
    }

    async fn consolidate(
        &self,
        llm: &dyn LlmProvider,
        conversation_id: i64,
    ) -> Result<(), MemoryError> {
        let pending = self
            .store
            .pending_messages(conversation_id, MAX_MESSAGES)
            .await?;
        let (Some(first), Some(last)) = (pending.first(), pending.last()) else {
            return Ok(());
        };

        let transcript = pending
            .iter()
            .map(|message| {
                let speaker = if message.role == "user" {
                    "Utilisateur"
                } else {
                    "ArIès"
                };
                format!(
                    "{speaker} : {}",
                    truncate_chars(&message.content, MESSAGE_MAX_CHARS)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let query = self
            .embedder
            .embed_one(truncate_chars(&transcript, QUERY_MAX_CHARS))
            .await?;
        let known_ids = self
            .vectors
            .search(FACTS, &query, RELATED_FACTS, RELATED_FACTS_MIN_SCORE, None)
            .await?;
        let known = self.store.facts_by_ids(&known_ids).await?;
        let known_list = if known.is_empty() {
            "(aucun)".to_owned()
        } else {
            known
                .iter()
                .map(|fact| format!("[{}] {}", fact.id, fact.content))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let raw = llm
            .complete(
                vec![
                    Message::system(SYSTEM_PROMPT),
                    Message::user(format!(
                        "Faits connus :\n{known_list}\n\nExtrait de conversation :\n{transcript}"
                    )),
                ],
                CompletionOptions {
                    max_tokens: Some(512),
                    temperature: Some(0.2),
                    json: true,
                },
            )
            .await?;

        match parse_consolidation(&raw) {
            Some(result) => {
                self.apply(conversation_id, first, last, &known, result)
                    .await?
            }
            None => tracing::warn!(raw = %raw, "sortie de consolidation illisible, extrait ignoré"),
        }
        self.store.mark_consolidated(conversation_id, last.id).await
    }

    async fn apply(
        &self,
        conversation_id: i64,
        first: &StoredMessage,
        last: &StoredMessage,
        known: &[Fact],
        result: Consolidation,
    ) -> Result<(), MemoryError> {
        // Le LLM ne peut toucher qu'aux faits qu'on lui a montrés.
        let known_ids: HashSet<i64> = known.iter().map(|fact| fact.id).collect();
        let deleted: Vec<i64> = result
            .delete
            .into_iter()
            .filter(|id| known_ids.contains(id))
            .collect();
        let updates: Vec<(i64, String)> = result
            .update
            .into_iter()
            .filter(|update| known_ids.contains(&update.id) && !deleted.contains(&update.id))
            .map(|update| (update.id, update.content.trim().to_owned()))
            .filter(|(_, content)| !content.is_empty())
            .collect();
        let additions: Vec<String> = result
            .add
            .iter()
            .map(|content| content.trim().to_owned())
            .filter(|content| !content.is_empty())
            .collect();
        let episode = result.episode.trim();

        // Tous les vecteurs d'abord, en un appel : si l'embedding échoue, rien n'est écrit
        // et l'extrait sera retraité au prochain passage.
        let texts: Vec<&str> = updates
            .iter()
            .map(|(_, content)| content.as_str())
            .chain(additions.iter().map(String::as_str))
            .chain((!episode.is_empty()).then_some(episode))
            .collect();
        let mut vectors = self.embedder.embed(&texts).await?.into_iter();

        for id in &deleted {
            self.store.delete_fact(*id).await?;
        }
        self.vectors.delete(FACTS, &ids_u64(&deleted)).await?;

        let mut fact_points = Vec::new();
        for ((id, content), vector) in updates.iter().zip(vectors.by_ref()) {
            if self.store.update_fact(*id, content).await? {
                fact_points.push((*id as u64, vector, json!({})));
            }
        }
        for (content, vector) in additions.iter().zip(vectors.by_ref()) {
            let id = self.store.insert_fact(content, conversation_id).await?;
            fact_points.push((id as u64, vector, json!({})));
        }
        self.vectors.upsert(FACTS, fact_points).await?;

        if let Some(vector) = vectors.next().filter(|_| !episode.is_empty()) {
            let id = self
                .store
                .insert_episode(
                    conversation_id,
                    episode,
                    &first.created_at,
                    &last.created_at,
                )
                .await?;
            self.vectors
                .upsert(
                    EPISODES,
                    [(
                        id as u64,
                        vector,
                        json!({ "conversation_id": conversation_id }),
                    )],
                )
                .await?;
        }

        tracing::info!(
            conversation_id,
            added = additions.len(),
            updated = updates.len(),
            deleted = deleted.len(),
            episode = !episode.is_empty(),
            "mémoire consolidée"
        );
        Ok(())
    }
}

/// Extrait l'objet JSON même s'il est entouré de texte ou de balises Markdown.
fn parse_consolidation(raw: &str) -> Option<Consolidation> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    serde_json::from_str(raw.get(start..=end)?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_wrapped_in_markdown() {
        let raw =
            "```json\n{\"episode\": \"Test.\", \"add\": [\"L'utilisateur aime le café.\"]}\n```";
        let result = parse_consolidation(raw).expect("JSON valide");
        assert_eq!(result.episode, "Test.");
        assert_eq!(result.add.len(), 1);
        assert!(result.update.is_empty() && result.delete.is_empty());
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_consolidation("pas de JSON ici").is_none());
    }
}
