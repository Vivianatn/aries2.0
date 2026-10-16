# Dépendances adoptées

Critères : voir README §7. Une ligne par adoption.

| Projet | Version | Licence | Rôle / pourquoi |
|---|---|---|---|
| llama.cpp (image `ghcr.io/ggml-org/llama.cpp:server`) | dernière | MIT | Inférence locale sur CPU, API compatible OpenAI. Service séparé |
| Qwen3-8B GGUF Q4_K_M | — | Apache-2.0 | Modèle local par défaut (à comparer en phase 0 via la vitesse mesurée) |
| `tokio` | 1 | MIT | Runtime async |
| `axum` | 0.8 | MIT | Serveur HTTP + SSE |
| `tower-http` | 0.6 | MIT | Fichiers statiques, traces HTTP |
| `reqwest` | 0.13 | MIT/Apache-2.0 | Client HTTP vers les LLM (rustls, pas d'OpenSSL). Passé de 0.12 à 0.13 pour partager la version de `rmcp` |
| `eventsource-stream` | 0.2 | MIT/Apache-2.0 | Décodage des flux SSE des LLM |
| `futures` | 0.3 | MIT/Apache-2.0 | Combinateurs de flux |
| `async-trait` | 0.1 | MIT/Apache-2.0 | Trait `LlmProvider` utilisable en `dyn` |
| whisper.cpp (`whisper-server`, compilé dans `deploy/whisper` : l'image officielle plante, AVX-512) | v1.9.4 | MIT | Reconnaissance vocale sur CPU, `--convert` (ffmpeg) pour l'audio du navigateur. Service séparé |
| Whisper `ggml-small-q5_1` | — | MIT | Modèle STT multilingue (~190 Mo), bon compromis vitesse/qualité en français sur CPU |
| Piper (`piper-tts[http]`, OHF-Voice/piper1-gpl) | 1.8.0 | GPL-3.0 | Synthèse vocale locale et rapide. GPL : conteneur à part (`deploy/piper`), appelé en HTTP |
| Voix Piper `fr_FR-siwis-medium` | — | MIT (modèle), données CC-BY 4.0 | Voix française par défaut (une locutrice) ; `fr_FR-upmc-medium` (locuteurs jessica/pierre) en alternative |
| `reqwest` (fonction `multipart`) | 0.13 | MIT/Apache-2.0 | Envoi de l'audio à `whisper-server` |
| `bytes` | 1 | MIT | Audio transmis sans copie entre axum et reqwest |
| Source Sans 3 (woff2 latin 400 et 600, via `@fontsource/source-sans-3`) | 5.2.9 | OFL-1.1 (`web/fonts/OFL.txt`) | Police de la DA, auto-hébergée : pas de Google Fonts (transfert d'IP à un tiers, RGPD) |
| marked (`web/vendor/marked.umd.js`) | 18.0.14 | MIT | Markdown des réponses dans le chat (GFM : listes, tableaux, liens). Servi par ArIès, sans CDN |
| DOMPurify (`web/vendor/purify.min.js`) | 3.4.16 | MPL-2.0 ou Apache-2.0 | Nettoie le HTML issu du Markdown : une page lue par ArIès ne peut pas injecter de script ; images interdites (pas de requête vers un tiers) |
| `tower-http` (fonction `set-header`) | 0.6 | MIT | En-têtes de sécurité : CSP, Referrer-Policy, Permissions-Policy |
| `serde`, `serde_json`, `toml` | 1 / 1 / 0.8 | MIT/Apache-2.0 | (Dé)sérialisation, configuration |
| `thiserror`, `anyhow` | 2 / 1 | MIT/Apache-2.0 | Erreurs (lib / binaire) |
| `tracing`, `tracing-subscriber` | 0.1 / 0.3 | MIT | Logs structurés |

| bge-m3 GGUF Q8_0 (`ggml-org/bge-m3-Q8_0-GGUF`) | — | MIT | Embeddings multilingues (1024 dim), servis par un 2e llama.cpp `--embeddings` |
| Qdrant (image `qdrant/qdrant`) | dernière | Apache-2.0 | Index vectoriel des faits et souvenirs. Service séparé |
| `sqlx` (SQLite) | 0.8 | MIT/Apache-2.0 | Base de la mémoire, migrations intégrées au binaire |
| `jiff` | 0.2 | MIT/Unlicense | Date et heure locales (fuseau `TZ`) injectées dans le contexte |
| SearXNG (image `searxng/searxng`) | dernière | AGPL-3.0 | Recherche web sans clé API (outil `web_search`). Service séparé, aucun port publié |
| `dom_smoothie` | 0.18 | MIT | Extraction du texte principal d'une page (port de Readability, Firefox) pour `read_web_page` |
| `rmcp` | 3.5 | Apache-2.0 | SDK MCP officiel, côté client seulement, transport HTTP « streamable » : outils de n'importe quel serveur MCP |
| Playwright MCP (image `mcr.microsoft.com/playwright/mcp`) | dernière | Apache-2.0 | Navigateur headless (Chromium) piloté par MCP. Remplace `chromiumoxide` : rien à coder. Service séparé, réseau isolé |
| `@notionhq/notion-mcp-server` (image `node:22-alpine`, version figée) | 2.5.2 | MIT | Outils Notion (API officielle) en MCP HTTP. Le dépôt se dit « plus maintenu » au profit du MCP hébergé de Notion (OAuth), mais publié le 2026-09-20. Service séparé |
| `google_workspace_mcp` (taylorwilsdon, construit depuis le commit `7897304`) | 2026-10-09 | MIT | Gmail et Agenda en MCP HTTP, mono-utilisateur, droits par niveau (`--permissions`). Service séparé, jeton OAuth dans le volume `google_creds` |
| `croner` | 4 | MIT | Expressions cron et prochaine échéance, avec `jiff` (gère l'heure d'été). `tokio-cron-scheduler` (sans SQLite) et `apalis` (exige sqlx 0.9) écartés : stockage et boucle maison (~150 lignes) |
| socat (image `alpine/socat`) | dernière | GPL-2.0 | Relais TCP entre ArIès et Playwright (isolation réseau). Service séparé |

Écarté : `qdrant-client` (gRPC/tonic, compilation lourde). 5 appels REST via `reqwest`
suffisent (`crates/aries-memory/src/vectors.rs`). Mem0 (Python) a servi de modèle pour la
consolidation des faits, sans être intégré.

Écarté pour l'instant : `async-openai` et `rig` (réévalué en phase 3 : `rig-rmcp` dépend de rmcp 2, incompatible avec notre rmcp 3.5). Le chat en streaming et les appels
d'outils (phase 2a) tiennent en ~150 lignes sur `reqwest` (`crates/aries-llm`), et la
boucle d'agent avec gouvernance en ~250 (`crates/aries-core`). `rig` sera réévalué en
phase 3 (planification, sous-agents).

Écarté : `chromiumoxide` (navigateur piloté depuis le binaire) au profit de Playwright MCP :
même résultat sans code, et le navigateur reste dans un conteneur à part.

Écarté : `html2text` (garde menus et pieds de page) au profit de `dom_smoothie`, qui
isole le contenu principal.
