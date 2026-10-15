# Dépendances adoptées

Critères : voir README §7. Une ligne par adoption.

| Projet | Version | Licence | Rôle / pourquoi |
|---|---|---|---|
| llama.cpp (image `ghcr.io/ggml-org/llama.cpp:server`) | dernière | MIT | Inférence locale sur CPU, API compatible OpenAI. Service séparé |
| Qwen3-8B GGUF Q4_K_M | — | Apache-2.0 | Modèle local par défaut (à comparer en phase 0 via la vitesse mesurée) |
| `tokio` | 1 | MIT | Runtime async |
| `axum` | 0.8 | MIT | Serveur HTTP + SSE |
| `tower-http` | 0.6 | MIT | Fichiers statiques, traces HTTP |
| `reqwest` | 0.12 | MIT/Apache-2.0 | Client HTTP vers les LLM (rustls, pas d'OpenSSL) |
| `eventsource-stream` | 0.2 | MIT/Apache-2.0 | Décodage des flux SSE des LLM |
| `futures` | 0.3 | MIT/Apache-2.0 | Combinateurs de flux |
| `async-trait` | 0.1 | MIT/Apache-2.0 | Trait `LlmProvider` utilisable en `dyn` |
| `serde`, `serde_json`, `toml` | 1 / 1 / 0.8 | MIT/Apache-2.0 | (Dé)sérialisation, configuration |
| `thiserror`, `anyhow` | 2 / 1 | MIT/Apache-2.0 | Erreurs (lib / binaire) |
| `tracing`, `tracing-subscriber` | 0.1 / 0.3 | MIT | Logs structurés |

| bge-m3 GGUF Q8_0 (`ggml-org/bge-m3-Q8_0-GGUF`) | — | MIT | Embeddings multilingues (1024 dim), servis par un 2e llama.cpp `--embeddings` |
| Qdrant (image `qdrant/qdrant`) | dernière | Apache-2.0 | Index vectoriel des faits et souvenirs. Service séparé |
| `sqlx` (SQLite) | 0.8 | MIT/Apache-2.0 | Base de la mémoire, migrations intégrées au binaire |
| `jiff` | 0.2 | MIT/Unlicense | Date et heure locales (fuseau `TZ`) injectées dans le contexte |
| SearXNG (image `searxng/searxng`) | dernière | AGPL-3.0 | Recherche web sans clé API (outil `web_search`). Service séparé, aucun port publié |
| `dom_smoothie` | 0.18 | MIT | Extraction du texte principal d'une page (port de Readability, Firefox) pour `read_web_page` |

Écarté : `qdrant-client` (gRPC/tonic, compilation lourde). 5 appels REST via `reqwest`
suffisent (`crates/aries-memory/src/vectors.rs`). Mem0 (Python) a servi de modèle pour la
consolidation des faits, sans être intégré.

Écarté pour l'instant : `async-openai` et `rig`. Le chat en streaming et les appels
d'outils (phase 2a) tiennent en ~150 lignes sur `reqwest` (`crates/aries-llm`), et la
boucle d'agent avec gouvernance en ~250 (`crates/aries-core`). `rig` sera réévalué en
phase 3 (planification, sous-agents).

Écarté : `html2text` (garde menus et pieds de page) au profit de `dom_smoothie`, qui
isole le contenu principal.
