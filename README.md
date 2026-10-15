# ArIès 2.0

> Un assistant personnel généraliste et agentique, inspiré de J.A.R.V.I.S. :
> il écoute, se souvient, agit et prend des initiatives, sur un serveur qui t'appartient.

ArIès 2.0 réécrit entièrement ArIès 1 (Python, `F:\aries`). On garde ce qui a fait
ses preuves (outils, gouvernance, voix, mémoire) et on change le moteur :
**Rust**, des **modèles locaux** sur le serveur et un **vrai agent autonome**
qui peut naviguer sur Internet.

---

## 1. Pourquoi une réécriture

| Limite d'ArIès 1 (Python) | Réponse dans ArIès 2.0 |
|---|---|
| Interpréteur lent, GIL : une seule tâche lourde à la fois | Rust + `tokio` : des milliers de tâches asynchrones sur les 8 vCPU |
| Démarrage lent, consommation mémoire élevée | Binaire natif unique (~20–40 Mo de RAM pour le cœur) |
| Erreurs découvertes à l'exécution | Typage strict : la plupart des bugs sont arrêtés à la compilation |
| Dépend d'API cloud (Gemini, Deepgram, ElevenLabs) | Inférence locale par défaut (LLM, STT, TTS, embeddings), cloud en option |
| Assistant réactif (questions → réponses) | Agent : planifie, enchaîne les outils, travaille en tâche de fond, notifie |
| Python « embeddable » capricieux sur le PC | Plus de Python dans le cœur, déploiement Docker sur le serveur |

### Pourquoi Rust (et pas Go, C++ ou Python)

- **Performance native** comparable au C++, sans ramasse-miettes : la latence reste
  stable, ce qui compte pour une voix fluide.
- **Sécurité mémoire et concurrence** garanties par le compilateur. C'est essentiel
  pour un démon qui tourne 24 h/24 et exécute des actions réelles.
- **Écosystème IA et assistant mûr** : `llama.cpp` (bindings), `whisper-rs`,
  `candle` (Hugging Face), `qdrant` (écrit en Rust), `rmcp` (SDK MCP officiel),
  `teloxide` (Telegram), `axum` (web).
- Go serait plus simple à écrire mais moins bon pour l'inférence embarquée.
  C++ est aussi rapide, mais bien plus risqué à maintenir.

Les composants lourds (LLM, STT) restent des moteurs C/C++ éprouvés
(`llama.cpp`, `whisper.cpp`). Rust les **orchestre** : c'est là que se joue la vitesse
perçue.

---

## 2. Le matériel cible

**`tharros-server`** : VPS PulseHeberg, gamme **Economy Cloud « Silver G1 »**
(lancée en octobre 2025), hébergé à Marseille, sous **Ubuntu 26.04**.

| Ressource | Capacité | Usage prévu |
|---|---|---|
| RAM | 16 Go | LLM local quantifié (~6 Go) + STT + embeddings + services |
| Disque | 160 Go NVMe (RAID 1) | Modèles (~20–30 Go), mémoire vectorielle, journaux |
| CPU | 8 vCPU | Inférence CPU (`llama.cpp`) + cœur Rust |
| GPU | **aucun** (VPS cloud) | Limite les modèles locaux à ~8 B paramètres |
| Réseau | IPv4 + IPv6 publiques, pare-feu côté hébergeur | Exposition à sécuriser (voir §4.8) |
| Sauvegarde | Instantanés (snapshots) depuis l'espace client | Un instantané avant chaque grosse mise à jour |

### Ce qu'il faut savoir sur les LLM locaux sans GPU

Sur 8 vCPU sans GPU, un modèle **7–9 B paramètres quantifié en Q4** produit environ
**5 à 12 tokens/s**. C'est fluide pour discuter, mais lent pour raisonner sur une
longue tâche. Au-delà de ~14 B, ça devient trop lent et la RAM est saturée.

Sur une gamme « Economy », les vCPU sont probablement **partagés** avec d'autres
clients : il faut s'attendre plutôt au bas de la fourchette, avec des variations
de vitesse. Les instructions CPU disponibles (AVX2, AVX-512) changent aussi
beaucoup la vitesse de `llama.cpp`. On le vérifie en phase 0 :

```bash
lscpu | grep -E "Model name|^CPU\(s\)|Flags" | grep -oE "Model name.*|CPU\(s\).*|avx512[a-z_]*|avx2|fma"
```

Puis on mesure la vitesse réelle avec `llama-bench` avant de choisir le modèle.

D'où le choix d'une **architecture hybride avec routeur** :

- **Local** (privé, gratuit, toujours disponible) : conversation courante, tri des
  demandes, résumés, extraction de mémoire, embeddings, voix.
- **Cloud** (en option, à la demande) : raisonnement complexe, code, longues
  tâches agentiques. Exemples : Claude Opus 5.5 / Sonnet 5.5, Gemini, GPT.
- Le **routeur** choisit le modèle selon la difficulté, la sensibilité des données
  (une donnée privée reste en local) et le budget.

---

## 3. Architecture

```
            ┌──────────────── Interfaces ────────────────┐
            │  Web UI (WebSocket)  ·  Telegram  ·  Voix  │
            │  (micro / wake word "ArIès")  ·  API REST  │
            └──────────────────────┬─────────────────────┘
                                   │
                        ┌──────────▼──────────┐
                        │   aries-gateway     │  axum, auth, sessions
                        └──────────┬──────────┘
                                   │
   ┌───────────────────────────────▼───────────────────────────────┐
   │                        aries-core (agent)                     │
   │  Boucle ReAct : Percevoir → Planifier → Agir → Observer → …   │
   │  · planificateur de tâches longues  · sous-agents parallèles  │
   │  · réflexion / auto-correction      · file de tâches de fond  │
   └───┬──────────────┬──────────────┬──────────────┬──────────────┘
       │              │              │              │
┌──────▼─────┐ ┌──────▼─────┐ ┌──────▼──────┐ ┌─────▼──────────┐
│ aries-llm  │ │aries-memory│ │ aries-tools │ │ aries-governance│
│ routeur    │ │ court terme│ │ natifs +    │ │ permissions     │
│ local/cloud│ │ épisodique │ │ clients MCP │ │ audit, budget   │
│            │ │ sémantique │ │             │ │ validation      │
└──────┬─────┘ └──────┬─────┘ └──────┬──────┘ └─────────────────┘
       │              │              │
 llama.cpp       Qdrant +       SearXNG, navigateur headless,
 server          SQLite         Gmail, Calendar, Notion, fichiers,
 (+ API cloud)                  shell en bac à sable, domotique…

   aries-voice : whisper.cpp (STT) · Piper (TTS) · openWakeWord
   aries-proactive : planificateur, déclencheurs, briefings
```

### Organisation du code (workspace Cargo)

```
aries2.0/
├── Cargo.toml                 # workspace
├── crates/
│   ├── aries-core/            # boucle agent, planificateur, sous-agents
│   ├── aries-llm/             # trait LlmProvider + routeur (llama.cpp, Claude, Gemini, OpenAI)
│   ├── aries-memory/          # mémoire à 3 niveaux + extraction automatique
│   ├── aries-tools/           # outils natifs + client MCP (rmcp)
│   ├── aries-governance/      # permissions read/action/danger, audit JSONL, budget
│   ├── aries-voice/           # STT, TTS, wake word, streaming audio
│   ├── aries-proactive/       # cron, déclencheurs, briefing du matin
│   ├── aries-gateway/         # serveur HTTP/WebSocket (axum)
│   └── aries-telegram/        # bot Telegram (teloxide)
├── web/                       # interface (Svelte ou HTMX, légère)
├── models/                    # modèles GGUF / Whisper / Piper (gitignorés)
├── config/
│   ├── aries.toml             # configuration principale
│   └── persona.md             # personnalité, ton, règles d'ArIès
├── deploy/
│   ├── apache/                # virtual host + installation (HTTPS, mot de passe)
│   └── download-models.sh     # téléchargement des modèles GGUF
├── docs/dependencies.md       # projets externes adoptés et pourquoi
├── data/                      # SQLite de la mémoire (gitignoré)
├── docker-compose.yml         # aries, llama.cpp ×2, qdrant, searxng, playwright (+ relais)
├── Dockerfile
└── README.md
```

---

## 4. Les briques en détail

### 4.1 Cerveau : `aries-llm`

- Un trait `LlmProvider` unique (chat, streaming, appels d'outils) et une
  implémentation par fournisseur.
- **Local** : `llama.cpp` en mode serveur (API compatible OpenAI), avec **Qwen3-8B
  Q4_K_M** (~5 tokens/s). Choisi après benchmark de 6 variantes (`docs/benchmarks.md`) :
  les modèles plus rapides (LFM2.5 MoE, Qwen3.5-4B) étaient trop faibles en qualité.
  Il sert de secours au chat et fait les tâches de fond (consolidation de la mémoire).
- **Cloud** (`[llm.cloud]`, Gemini 3.5 Flash par défaut ; 2.5 n'est plus ouvert aux nouveaux comptes) : répond au chat dès que sa clé
  est dans `.env` (100 tokens/s ou plus, premier mot en moins d'une seconde).
  Si le cloud échoue avant de répondre (réseau, quota, panne), l'agent (`aries-core`)
  continue le tour en local, avec seulement les outils `local` : la liste complète ne
  tient pas dans les 8k tokens du modèle local. Niveau gratuit de Gemini : environ
  20 requêtes par jour et par modèle (constaté le 15/10/2026).
- **Routeur plus fin** (phase 3) : choisir local ou cloud selon la difficulté et la
  sensibilité de la demande.

### 4.2 Mémoire : `aries-memory`

Trois niveaux, comme un humain :

1. **Court terme** : la conversation en cours, avec résumé glissant quand le
   contexte déborde.
2. **Épisodique** : chaque échange est horodaté, résumé et vectorisé dans
   **Qdrant**. ArIès peut répondre à « qu'est-ce que je t'ai dit mardi sur… ».
3. **Sémantique** : faits durables sur toi (préférences, proches, projets),
   stockés dans **SQLite** et extraits automatiquement après chaque échange,
   avec détection des contradictions.

Les embeddings sont calculés localement (ex. `bge-m3` ou `nomic-embed` via
`candle` / `llama.cpp`) : rien ne sort du serveur.

### 4.3 Agent : `aries-core`

- **Boucle ReAct** avec appels d'outils natifs du modèle.
- **Planification** : une demande complexe est découpée en étapes, exécutées,
  vérifiées puis corrigées si besoin.
- **Sous-agents** : recherche web, analyse de documents et code tournent en
  parallèle (`tokio::spawn`), puis la synthèse est faite par l'agent principal.
- **Tâches de fond** ✅ (`aries-proactive`) : « surveille le prix de X et préviens-moi »
  devient une tâche persistante (`data/tasks.db`) qui survit aux redémarrages. Outils
  `schedule_task` (une fois `at`, ou `cron` en heure locale via `croner`), `list_tasks`,
  `cancel_task`. `reminder` envoie un message tel quel ; `agent` exécute une consigne dans
  sa propre conversation, visible dans l'interface, et ne notifie que si la réponse n'est
  pas « RIEN ». Ces tâches s'exécutent sans surveillance : les actions `action` sont
  autorisées, les actions `danger` refusées. Une échéance manquée est rattrapée une fois.
  Un échec est notifié.

### 4.4 Internet et outils : `aries-tools`

| Outil | Rôle |
|---|---|
| **SearXNG** (auto-hébergé) ✅ `web_search` | Méta-moteur de recherche privé, sans clé API |
| Extraction de contenu ✅ `read_web_page` | HTML → texte propre (Readability via `dom_smoothie`) ; PDF plus tard |
| **Navigateur headless** ✅ `browser_*` | Playwright MCP (conteneur isolé) : pages dynamiques, cliquer, remplir |
| **Notion** ✅ `API-*` | Serveur MCP Notion officiel : chercher, lire, créer une page, ajouter du contenu (pages partagées avec l'intégration seulement) |
| **Gmail / Agenda** ✅ | `google_workspace_mcp` (OAuth Google, un seul compte). Liste des mails (objet, expéditeur, date) et agenda : cloud autorisé. Contenu d'un mail : **privé**, lu seulement en local. Envoi et modification de l'agenda : `danger` |
| Météo, actualités, bourse | API publiques |
| Fichiers et shell **en bac à sable** | Conteneur isolé pour exécuter du code |
| **Client MCP** ✅ (`rmcp`) | Brancher n'importe quel serveur MCP (HTTP) existant par la config, sans code |
| Domotique (en option) | Home Assistant |

### 4.5 Voix : `aries-voice`

- **Mot d'éveil** « ArIès » (openWakeWord), en local.
- **STT** : `whisper.cpp` (modèle `small` ou `medium` quantifié), en streaming.
- **TTS** : **Piper** (voix française locale, très rapide). ElevenLabs reste
  disponible en option pour une voix premium.
- Objectif de latence : **moins de 1,5 s** entre la fin de ta phrase et le début de
  la réponse parlée (streaming du LLM vers le TTS phrase par phrase).

### 4.6 Proactivité : `aries-proactive`

C'est ce qui fait la différence entre un chatbot et Jarvis :

- **Briefing du matin** : agenda, mails importants, météo, rappels.
- **Déclencheurs** : nouveau mail d'un contact important, rendez-vous dans 30 min,
  événement surveillé.
- **Suivi** : « tu m'avais demandé de te rappeler… ».
- Notifications via Telegram, la Web UI ou la voix.

### 4.7 Gouvernance : `aries-governance`

Reprise et renforcée depuis ArIès 1 :

- Trois niveaux de risque : **read** (libre), **action** (journalisée),
  **danger** (ta validation est obligatoire : envoyer un mail, payer, supprimer).
- **Audit immuable** en JSONL de chaque appel d'outil et de chaque décision.
- **Budget** ✅ : plafond de requêtes cloud par jour (`daily_request_limit`) ; au-delà,
  bascule sur le modèle local. Compteur en mémoire (remis à zéro au redémarrage).

Fait en phase 2a (`crates/aries-governance`) : niveaux de risque, audit JSONL
(`data/audit.jsonl`) et validation des actions `danger` depuis l'interface.
- **Données privées** ✅ : un outil MCP listé dans `private` (ex. contenu d'un mail) rend
  la conversation privée dès qu'il répond. Toute la suite, y compris les messages
  suivants, est traitée par le modèle local, avec seulement les outils `private` et
  `local` (contexte court). Les faits et souvenirs qu'elle produit sont privés et ne sont
  jamais rappelés dans une conversation traitée par le cloud. Événement SSE
  `private_mode`. Ouvrir une nouvelle conversation permet de retrouver la vitesse du cloud.

### 4.8 Sécurité du serveur

ArIès est accessible directement sur Internet, à une adresse du type
`https://aries.antoine-vivian.fr`. Comme il aura accès à tes mails et à ton agenda,
une seule porte d'entrée est protégée :

- **Apache** (déjà en place pour `antoine-vivian.fr` et `codex`) sert de reverse proxy :
  un virtual host séparé (`deploy/apache/aries.conf`) ajoute le **HTTPS** (certificat
  Let's Encrypt propre à ArIès, via certbot) et un **mot de passe** (HTTP Basic, hash
  bcrypt dans `/etc/apache2/aries.htpasswd`). Les autres sites ne sont pas modifiés.
- ArIès n'écoute que sur `127.0.0.1:3050` ; `llama.cpp`, Qdrant et SearXNG n'ont
  **aucun port publié** (réseau interne Docker).
- **Plafonds de ressources** Docker : le LLM est limité à 6 vCPU et 8 Go, pour que
  codex, Apache et PostgreSQL gardent toujours de quoi tourner.
- **SSH** par clé, `fail2ban` et `ufw` (22, 80, 443) sont déjà actifs sur le serveur.
- Telegram fonctionne en *long polling* (le serveur appelle Telegram), donc aucun
  port entrant n'est nécessaire.
- Mises à jour de sécurité automatiques (`unattended-upgrades`).

### 4.9 Identité et direction artistique (à concevoir)

L'interface actuelle (`web/index.html`) est un chat fonctionnel, sans identité. ArIès doit
devenir **quelqu'un** : une présence avec une forme physique, et pas seulement une bulle
de texte avec un point lumineux.

#### Qui est ArIès

| Trait | Choix |
|---|---|
| Identité | Une jeune femme d'environ **20 ans** ; on dit « elle » |
| Caractère | Vive, directe, chaleureuse, un peu taquine ; compétente sans être scolaire |
| Langage | Français parlé naturel, tutoiement, quelques tics de langage (voir `config/persona.md`) |
| Présence | Calme et attentive : elle écoute, réfléchit, parle, attend ; elle ne « clignote » pas |

#### Intention visuelle

**Futuriste, simple, peu lumineux, humain.** On évite le cliché J.A.R.V.I.S. (hologrammes
bleus, néons, halos) ; on cherche plutôt un objet de design sobre, dans un intérieur
tamisé, avec un visage humain au centre.

- **Forme physique** : un avatar de **buste**, visage et épaules, **stylisé semi-réaliste**
  (ni photo, ni dessin animé), pour rester humain sans tomber dans la « vallée de
  l'étrange ». Tenue simple et intemporelle (col montant, matières mates), un détail
  futuriste discret (une fine ligne d'interface sur le vêtement, une oreillette).
- **Lumière** : ambiance tamisée. Pas de lueur, pas de `box-shadow` coloré, pas de dégradé
  néon. Un éclairage doux de côté sur l'avatar ; le reste est mat.
- **Couleurs** : des fonds sombres et chauds (anthracite, ardoise), des textes en blanc
  cassé, et **un seul accent** désaturé (bleu acier ou ambre doux), réservé aux états
  et aux actions.
- **Typographie** : une seule famille sans-serif humaniste et lisible, avec deux graisses
  au plus. Pas de police « techno » ni de monospace décorative.
- **Mouvement** : lent et organique (respiration, clignement, regard). Aucune animation
  en boucle rapide. `prefers-reduced-motion` est respecté.

#### Exigences pour la DA

1. **États de l'avatar**, reconnaissables sans lire de texte : *au repos*, *écoute*,
   *réfléchit* (remplace la bulle vide pendant l'attente du LLM), *cherche / utilise un
   outil*, *parle* (synchronisé avec la voix en phase 4), *attend ta validation* (action
   `danger`), *erreur*.
2. **Mise en page** : l'avatar occupe une place fixe (colonne sur ordinateur, bandeau
   compact sur mobile) et la conversation reste l'élément principal. L'interface marche
   à 360 px de large, sans défilement horizontal.
3. **Accessibilité** : contraste WCAG AA au minimum, états doublés d'un libellé texte
   pour les lecteurs d'écran, navigation au clavier.
4. **Légèreté** : rendu dans le navigateur, sans 3D lourde ; moins de 1 Mo d'assets pour
   l'avatar. Formats envisagés : SVG animé en CSS, ou Rive (runtime MIT) si l'animation
   du visage l'exige. Rien n'est calculé sur le serveur (il n'a pas de GPU).
5. **Thème** : sombre par défaut. Toutes les couleurs sont des variables CSS (tokens),
   pour pouvoir ajouter un thème clair plus tard.
6. **Droits** : les assets sont créés pour le projet ou sous licence libre (CC0, CC-BY).
   Le visage ne ressemble à aucune personne réelle.
7. **Cohérence** : la même identité sert à la Web UI, à l'avatar Telegram (phase 5) et à
   l'écran de veille vocal (phase 4).

#### Livrables attendus

- Une planche d'ambiance (moodboard) de 8 à 12 références, avec ce qu'on garde de chacune.
- Une fiche personnage : visage, coiffure, tenue, expressions de base.
- Les 7 états de l'avatar (statiques, puis animés).
- Les tokens : palette, typographie, espacements, rayons.
- Des maquettes ordinateur et mobile : conversation, ligne d'outil, demande de validation,
  panneau Mémoire.

Critère de réussite : en ouvrant l'interface, on a l'impression de parler **à quelqu'un**,
dans une pièce calme, et pas à un tableau de bord.

---

## 5. Budget mémoire estimé (16 Go)

| Composant | RAM |
|---|---|
| Système + Docker | ~1,5 Go |
| Site `codex.antoine-vivian.fr` (réservé) | ~0,5–1 Go (à mesurer) |
| LLM principal 8 B Q4_K_M | ~5,5 Go |
| Petit modèle 1–3 B Q4 | ~1,5–2 Go |
| Whisper small/medium | ~0,5–1,5 Go |
| Embeddings + Piper | ~1 Go |
| Qdrant + SQLite | ~0,5–1 Go |
| SearXNG + navigateur headless | ~1–1,5 Go |
| Cœur ArIès (Rust) | < 0,1 Go |
| **Total** | **~13–15 Go** : ça passe, mais la marge est mince |

Si la mémoire manque, on retire d'abord le petit modèle 1–3 B (le routage se fait alors
par règles seules), puis on passe Whisper en `small`.

---

## 6. Feuille de route

| Phase | Contenu | Résultat |
|---|---|---|
| **0. Socle** ✅ en ligne (3,8 tok/s avec Qwen3-8B) | Workspace Cargo, config, `llama.cpp` + modèle local, gateway axum, chat en streaming | Discuter avec ArIès en local |
| **1. Mémoire** ✅ en ligne | Qdrant, SQLite, extraction de faits, rappel contextuel | ArIès se souvient de toi |
| **2. Outils et web** 🚧 2b en cours | **2a** ✅ : boucle d'outils, SearXNG, lecture de pages, gouvernance (audit, validation). **2b** ✅ client MCP, navigateur headless (Playwright MCP), Notion, Gmail/Agenda, données privées → local, budget cloud | ArIès cherche et agit |
| **3. Agent** 🚧 3a fait | **3a** ✅ : tâches de fond persistantes (rappels, consignes en cron), bascule locale gérée par l'agent. **3b** : notifications Telegram. **3c** : planification, sous-agents, routeur | ArIès mène des tâches complexes |
| **4. Voix** | Wake word, whisper.cpp, Piper (voix féminine jeune), streaming | « ArIès ? » … et elle répond |
| **5. Proactivité** | Briefings, déclencheurs, Telegram | ArIès prend des initiatives |
| **6. Finitions** | Interface selon la DA (§4.9) avec avatar, sauvegardes, monitoring, sécurité | Prêt pour le quotidien |

Avant chaque phase : **veille GitHub** sur les briques de la phase (voir §7).

---

## 7. Réutiliser avant de coder

On n'écrit pas ce qui existe déjà et qui marche bien. Avant chaque brique, on cherche
dans cet ordre :

1. **La bibliothèque standard de Rust** (`std`), puis les crates déjà présentes dans
   `Cargo.lock`.
2. **Une crate ou un projet open source établi** sur crates.io, lib.rs ou GitHub.
3. **Un serveur MCP existant**, pour un outil qu'on branche sans écrire de code.
4. Seulement ensuite : **notre propre code**, en s'inspirant des bons projets.

### Critères pour adopter un projet

| Critère | Seuil |
|---|---|
| Licence | MIT, Apache-2.0, BSD ou MPL. Une licence GPL/AGPL n'est acceptable que pour un **service séparé** (conteneur à part), jamais liée dans le binaire ArIès |
| Activité | Commit de moins de 6 mois, issues traitées |
| Adoption | Utilisé par d'autres projets (téléchargements crates.io, étoiles, dépendants) |
| Compatibilité | Async `tokio`, fonctionne sur Linux x86_64 sans GPU |
| Poids | Pas de dépendances démesurées pour le besoin |

Chaque adoption est notée dans `docs/dependencies.md` (projet, version, licence, pourquoi).

### Candidats repérés (à vérifier au moment de chaque phase)

| Besoin | Candidats | Remarque |
|---|---|---|
| Inférence LLM locale | `llama.cpp` (serveur) · `mistral.rs` (natif Rust) | Comparer avec `llama-bench` sur le VPS |
| Client LLM | `async-openai` | Marche avec `llama.cpp`, Gemini et OpenAI (API compatibles OpenAI) |
| Cadre d'agent | `rig` (rig-core) | Fournisseurs, outils, RAG, Qdrant intégrés. À évaluer face à une boucle maison |
| Protocole MCP | `rmcp` (SDK Rust officiel) | Client et serveur |
| Embeddings | `fastembed-rs` | Modèles d'embedding locaux, sur CPU |
| Mémoire vectorielle | `qdrant` + `qdrant-client` | |
| Base SQL | `sqlx` (SQLite) | Requêtes vérifiées à la compilation |
| Web / API | `axum`, `tower-http` | |
| Navigateur | `chromiumoxide` | Pilote Chrome en headless |
| Recherche web | SearXNG | AGPL, donc dans un conteneur séparé |
| STT | `whisper-rs` (bindings de `whisper.cpp`) | |
| TTS | Piper | Vérifier la licence de la version maintenue (passée en GPL) : à lancer comme processus séparé |
| Mot d'éveil | `rustpotter` · openWakeWord | rustpotter est natif Rust |
| Telegram | `teloxide` | |
| Tâches planifiées | `tokio-cron-scheduler` | |
| Logs | `tracing` | |

À étudier pour s'en inspirer (sans les intégrer) : Open WebUI, le pipeline vocal de
Home Assistant (Assist), OpenHands (boucles d'agent), Leon AI.

---

## 8. Démarrage

Le projet est installé dans `/opt/aries` sur le serveur.

### Une seule fois : brancher ArIès sur Apache

```bash
curl -fsSL https://get.docker.com | sh       # Docker (absent par défaut)
bash deploy/apache/install.sh <utilisateur>  # vhost + mot de passe + certificat HTTPS
```

Prérequis DNS : chez le registrar, créer l'enregistrement
`aries  CNAME  antoine-vivian.fr.` (comme `codex`). Vérifier avec
`nslookup aries.antoine-vivian.fr 1.1.1.1` avant de lancer `install.sh`
(certbot échoue si le nom ne résout pas).

### Lancer ArIès

```bash
cp .env.example .env
sed -i "s/^SEARXNG_SECRET=.*/SEARXNG_SECRET=$(openssl rand -hex 32)/" .env   # clé interne SearXNG
bash deploy/download-models.sh        # ~5 Go (Qwen3-8B Q4_K_M par défaut)
docker compose up -d --build
docker compose logs -f llm            # vitesse réelle : ligne "eval time … tokens per second"
# Interface : https://aries.antoine-vivian.fr (mot de passe demandé par le navigateur)
```

Pour des réponses rapides, activer le chat cloud : voir **[docs/cloud-gemini.md](docs/cloud-gemini.md)**.

### En développement (sans Docker)

```bash
# Rust n'est pas nécessaire sur le PC : formatage, lint et tests dans un conteneur sur le serveur
docker run --rm -v /opt/aries:/src -v aries-cargo-registry:/usr/local/cargo/registry \
  -v aries-cargo-target:/src/target -w /src rust:1-bookworm \
  sh -c "rustup component add clippy rustfmt && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test"
```

### API

| Méthode | Route | Rôle |
|---|---|---|
| `GET` | `/api/v1/health` | État et version |
| `GET` / `POST` | `/api/v1/conversations` | Lister (50 dernières) / créer une conversation |
| `GET` | `/api/v1/conversations/{id}/messages` | Historique d'une conversation |
| `POST` | `/api/v1/conversations/{id}/messages` | `{"content":"…"}` → flux SSE : `token` `{"text"}`, `tool_call` `{"call_id","name","arguments"}`, `approval_required` `{"approval_id","call_id","name","arguments"}`, `tool_result` `{"call_id","name","success","decision"}`, `private_mode` `{}` (passage au modèle local), `error`, `done` `{"message_id"}` |
| `GET` | `/api/v1/memory-facts` | Faits retenus sur l'utilisateur |
| `DELETE` | `/api/v1/memory-facts/{id}` | Oublier un fait |
| `PUT` | `/api/v1/tool-approvals/{id}` | `{"approved": true\|false}` : valider ou refuser une action `danger` (204, ou 404 si expirée) |

### Mémoire : comment ça marche

1. **Avant chaque réponse** : le message est vectorisé (bge-m3), Qdrant retrouve les
   faits et les souvenirs d'autres conversations les plus proches, et ils sont injectés
   avec la date dans le dernier message. Ils ne vont pas dans le prompt système, pour que
   llama.cpp garde le début du prompt en cache.
2. **Historique** : les ~12 000 derniers caractères de la conversation. Au-delà, le
   début est remplacé par les souvenirs déjà tirés de cette conversation.
3. **Après 2 min d'inactivité** (ou 20 messages en attente) et seulement si aucune
   réponse n'est en cours : un seul appel au LLM produit un souvenir résumé, et
   ajoute, corrige ou supprime des faits. Le LLM ne peut modifier que les faits qu'on
   lui a montrés.
4. Les données sont dans `data/aries.db` (SQLite, source de vérité) et dans le volume
   Docker `aries_qdrant_data` (index vectoriel).

### Outils : comment ça marche

1. `aries-core` envoie au LLM le prompt et la liste des outils (`aries-tools`). Le LLM
   répond, ou demande un ou plusieurs appels d'outils (format OpenAI, en streaming).
2. Chaque appel passe par la gouvernance : `read`/`action` s'exécutent, `danger` attend
   ta validation dans l'interface (5 min, sinon refus). Tout est écrit dans
   `data/audit.jsonl`.
3. Les résultats sont renvoyés au LLM, jusqu'à 5 allers-retours (`[tools] max_rounds`) ;
   le dernier tour se fait sans outils pour forcer une réponse.
4. Outils natifs (niveau `read`) : `web_search` (SearXNG) et `read_web_page` (texte
   principal, 6 000 caractères max). `read_web_page` refuse les adresses privées ou
   locales et vérifie chaque redirection, pour que le LLM ne puisse pas atteindre
   llama.cpp, Qdrant ou le réseau Docker.
5. Outils MCP : chaque `[[tools.mcp]]` de `config/aries.toml` est un serveur MCP en HTTP
   (son propre conteneur). Au démarrage, ArIès s'y connecte (30 s d'essais, sinon il est
   ignoré), et expose les outils listés dans `tools`. Niveau de risque : `risks`, sinon
   les annotations MCP (`readOnlyHint` → read, `destructiveHint: false` → action, sinon
   danger). `descriptions` remplace la description d'un outil pour guider le LLM.
   Jeton éventuel : `token_env` (variable de `.env`, envoyée en `Authorization: Bearer`).
6. Navigateur (`browser_navigate`, `browser_snapshot`, `browser_click`…) : Playwright MCP,
   Chromium headless, profil en mémoire, sans compte connecté. Il est sur un réseau Docker
   à part ; seul le relais `browser-gateway` (socat vers Playwright) le relie à ArIès, pour
   qu'une page visitée ne puisse joindre ni l'API d'ArIès ni les services internes.
7. Seule la réponse finale est enregistrée dans l'historique, pas les résultats d'outils
   (un identifiant trouvé au message précédent doit donc être recherché à nouveau).
8. Google : le jeton OAuth est dans le volume `google_creds`. Pour (re)connecter le compte,
   ouvrir un tunnel `ssh -L 8000:<IP du conteneur google>:8000 root@serveur`, appeler un
   outil Gmail pour obtenir le lien d'autorisation, puis l'ouvrir sur le même PC (le lien
   expire en quelques minutes). En mode « Test » de Google, il faut recommencer tous les
   7 jours.

---

## 9. Décisions encore ouvertes

Tranché : VPS PulseHeberg sans GPU (AVX2, sans AVX-512), accès direct par Internet
derrière l'Apache existant (HTTPS + mot de passe), sans VPN (§4.8).

Tranché : chat hybride, avec cloud (Gemini Flash) en priorité et Qwen3-8B local en secours
et pour la mémoire.

Tranché : ArIès est une jeune femme d'environ 20 ans, avec une forme physique (avatar)
et une DA futuriste, sobre et peu lumineuse (§4.9). Personnalité dans `config/persona.md`.

Encore ouvert : canaux d'interaction, niveau d'autonomie, choix de la voix,
réalisation de la DA, intégrations prioritaires, pile technique du site
`codex.antoine-vivian.fr` qui partage le serveur. Ce README sera mis à jour à mesure qu'ils sont tranchés.
