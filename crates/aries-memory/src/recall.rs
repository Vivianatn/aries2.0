//! Construction du prompt : historique récent + souvenirs pertinents.

use aries_llm::{Message, Role};
use jiff::{Zoned, civil::Weekday};
use serde_json::json;

use crate::{
    Episode, Fact, Memory, MemoryError, truncate_chars,
    vectors::{EPISODES, FACTS},
};

/// Messages relus au maximum pour remplir la fenêtre d'historique.
const HISTORY_FETCH: i64 = 60;
const QUERY_MAX_CHARS: usize = 2_000;
const ITEM_MAX_CHARS: usize = 400;

impl Memory {
    /// Prompt complet pour répondre au dernier message de la conversation.
    /// Les souvenirs privés n'y figurent que si `include_private` (modèle local).
    ///
    /// Le contexte mémoire est placé dans le dernier message utilisateur, pas dans le
    /// prompt système : le début du prompt reste identique d'un tour à l'autre et
    /// llama.cpp réutilise son cache (sinon tout l'historique serait relu à chaque fois).
    pub async fn build_prompt(
        &self,
        conversation_id: i64,
        persona: &str,
        include_private: bool,
    ) -> Result<Vec<Message>, MemoryError> {
        let latest = self
            .store
            .latest_messages(conversation_id, HISTORY_FETCH)
            .await?;

        let mut budget = self.config.history_char_budget;
        let mut window = Vec::new();
        for message in &latest {
            let len = message.content.len();
            if !window.is_empty() && len > budget {
                break;
            }
            budget = budget.saturating_sub(len);
            window.push(message);
        }
        let truncated = window.len() < latest.len() || latest.len() as i64 == HISTORY_FETCH;
        window.reverse();

        let mut system = persona.trim().to_owned();
        if truncated {
            let episodes = self.store.conversation_episodes(conversation_id).await?;
            if !episodes.is_empty() {
                system.push_str("\n\nRésumé du début de cette conversation :");
                for episode in &episodes {
                    system.push_str(&format!("\n- {}", episode.summary));
                }
            }
        }

        let mut messages = vec![Message::system(system)];
        messages.extend(window.iter().filter_map(|message| message.to_llm()));

        if let Some(last) = messages
            .last_mut()
            .filter(|message| message.role == Role::User)
        {
            let context = self
                .recall_context(conversation_id, &last.content, include_private)
                .await;
            last.content = format!("{context}\n{}", last.content);
        }
        Ok(messages)
    }

    /// Faits et souvenirs d'autres conversations proches de `query`, par pertinence.
    pub async fn recall(
        &self,
        conversation_id: i64,
        query: &str,
        include_private: bool,
    ) -> Result<(Vec<Fact>, Vec<Episode>), MemoryError> {
        let vector = self
            .embedder
            .embed_one(truncate_chars(query, QUERY_MAX_CHARS))
            .await?;
        let other_conversations = json!({ "must_not": [{ "key": "conversation_id", "match": { "value": conversation_id } }] });

        let (fact_ids, episode_ids) = tokio::try_join!(
            self.vectors.search(
                FACTS,
                &vector,
                self.config.facts_limit,
                self.config.facts_min_score,
                None,
            ),
            self.vectors.search(
                EPISODES,
                &vector,
                self.config.episodes_limit,
                self.config.episodes_min_score,
                Some(other_conversations),
            ),
        )?;

        let mut facts = self.store.facts_by_ids(&fact_ids, include_private).await?;
        facts.sort_by_key(|fact| fact_ids.iter().position(|&id| id == fact.id));

        let episodes = self
            .store
            .episodes_by_ids(&episode_ids, include_private)
            .await?;
        Ok((facts, episodes))
    }

    /// Bloc de contexte injecté avant le message utilisateur. Ne bloque jamais la
    /// réponse : si la mémoire est indisponible, ArIès répond sans souvenirs.
    async fn recall_context(
        &self,
        conversation_id: i64,
        query: &str,
        include_private: bool,
    ) -> String {
        let mut context = format!(
            "[Contexte fourni par ta mémoire, à utiliser sans le citer tel quel]\nNous sommes le {}.",
            now_in_french()
        );
        match self.recall(conversation_id, query, include_private).await {
            Ok((facts, episodes)) => {
                if !facts.is_empty() {
                    context.push_str("\nCe que tu sais de l'utilisateur :");
                    for fact in &facts {
                        context.push_str(&format!(
                            "\n- {}",
                            truncate_chars(&fact.content, ITEM_MAX_CHARS)
                        ));
                    }
                }
                if !episodes.is_empty() {
                    context.push_str("\nSouvenirs de conversations passées :");
                    for episode in &episodes {
                        let date = episode.ended_at.get(..10).unwrap_or(&episode.ended_at);
                        context.push_str(&format!(
                            "\n- ({date}) {}",
                            truncate_chars(&episode.summary, ITEM_MAX_CHARS)
                        ));
                    }
                }
            }
            Err(err) => {
                tracing::warn!(%err, "rappel de la mémoire impossible, réponse sans souvenirs")
            }
        }
        context.push_str("\n[Fin du contexte]\n\nMessage de l'utilisateur :");
        context
    }
}

/// Ex. « mardi 14/10/2026 à 22:51 » dans le fuseau du serveur (variable `TZ`).
fn now_in_french() -> String {
    let now = Zoned::now();
    let weekday = match now.weekday() {
        Weekday::Monday => "lundi",
        Weekday::Tuesday => "mardi",
        Weekday::Wednesday => "mercredi",
        Weekday::Thursday => "jeudi",
        Weekday::Friday => "vendredi",
        Weekday::Saturday => "samedi",
        Weekday::Sunday => "dimanche",
    };
    format!("{weekday} {}", now.strftime("%d/%m/%Y à %H:%M"))
}
