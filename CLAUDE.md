# CLAUDE.md : ArIès 2.0

Assistant personnel agentique « type Jarvis », en Rust, sur un VPS sans GPU
(8 vCPU partagés, 16 Go RAM, Ubuntu 26.04). L'architecture et la feuille de route sont
dans `README.md` : c'est la source de vérité. Le mettre à jour quand une décision change.

Langue : réponses, commentaires et docs en **français** ; identifiants de code en **anglais**.

## 1. Réutiliser avant d'écrire

Avant d'écrire une fonction ou une brique, vérifier dans cet ordre :

1. `std` (itérateurs, `Option`/`Result` combinators, `collections`, `fmt`, `time`…).
2. Les crates déjà présentes dans `Cargo.lock` (`grep` dessus avant d'en ajouter une).
3. Une crate établie (crates.io / lib.rs / GitHub) ou un serveur MCP existant.
4. Sinon seulement, du code maison.

Pour adopter un projet externe, appliquer les critères du README §7 (licence permissive,
GPL/AGPL seulement en service séparé ; maintenu ; compatible `tokio` et CPU seul),
puis l'ajouter à `docs/dependencies.md`. Ne jamais inventer une API de crate : vérifier
sur docs.rs la version utilisée.

## 2. Conventions Rust

- Workspace Cargo ; crates `aries-<domaine>` en kebab-case (`aries-core`, `aries-llm`…).
- Nommage standard : `snake_case` (fonctions, variables, modules, fichiers),
  `UpperCamelCase` (types, traits, variantes), `SCREAMING_SNAKE_CASE` (constantes).
- Erreurs : `thiserror` dans les bibliothèques, `anyhow` seulement dans les binaires.
  Pas de `unwrap()` / `expect()` hors tests et invariants prouvés.
- Async : `tokio` partout ; jamais de code bloquant dans une tâche async
  (`spawn_blocking` pour le CPU lourd).
- Logs : `tracing` (pas de `println!`). Config : `config/aries.toml` + variables d'env ;
  secrets uniquement dans `.env` (gitignoré).
- Abstractions par trait aux frontières (`LlmProvider`, `Tool`, `MemoryStore`), pas
  au-delà du besoin.
- Avant de rendre la main : `cargo fmt`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test`. Signaler honnêtement tout échec.

## 3. Conventions web / API

- REST versionné : `/api/v1/...`, ressources au pluriel en kebab-case
  (`/api/v1/memory-facts/{id}`), verbes HTTP et codes de statut corrects.
- JSON en `snake_case` ; erreurs au format unique `{ "error": { "code", "message" } }`.
- Temps réel (chat, voix) en WebSocket sur `/api/v1/ws`.
- L'Apache du serveur (qui sert aussi `antoine-vivian.fr` et `codex`) fait HTTPS +
  mot de passe via `deploy/apache/aries.conf`. Dans Docker Compose, seul
  `127.0.0.1:3050` est publié (Docker contourne ufw). Ne jamais modifier les vhosts,
  le certificat ni le service `codex` ; toujours `apachectl configtest` avant un reload.

## 4. Gouvernance (non négociable)

- Tout outil déclare un niveau `read` / `action` / `danger` ; `danger` exige une
  validation humaine. Tout appel d'outil passe par l'audit.
- Les données marquées privées ne partent jamais vers un LLM cloud.

## 5. Économie de tokens

- Chercher avec Grep/Glob, lire seulement les lignes utiles (`offset`/`limit`),
  ne pas relire un fichier qu'on vient d'éditer.
- Modifier par `Edit` ciblé plutôt que réécrire un fichier entier.
- Code concis : préférer les fonctions existantes (std, itérateurs, macros
  `derive`) au code manuel ; pas de commentaires qui paraphrasent le code.
- Réponses courtes : ce qui a changé, ce qui a été vérifié, ce qui reste. Pas de récap
  du code déjà visible.
- Déléguer à un sous-agent seulement pour les recherches larges (veille GitHub d'une phase).

## 6. Environnement

- Dev sur PC Windows (`e:\aries2.0`), exécution sur le VPS Linux : viser
  `x86_64-unknown-linux-gnu`, déployer via Docker Compose.
- Pas de GPU : tout composant d'inférence doit tourner sur CPU.
