# Veille : accélérer le LLM local (CPU seul)

Veille du 2026-10-16. But : faire répondre plus vite le modèle local (Qwen3-8B Q4_K_M via
`llama.cpp`), qui sert de secours au chat, au mode privé et aux tâches de fond.
Chaque piste est **à mesurer sur le VPS** avant adoption (protocole en fin de document).
Critères d'adoption : README §7 (licence, activité < 6 mois, AVX2 sans GPU, poids raisonnable).

## Point de départ (docs/benchmarks.md)

| Mesure | Valeur | Conséquence |
|---|---|---|
| Lecture du prompt (`pp512`) | ~21 tokens/s | Un prompt de 3 000 tokens (persona, mémoire, outils) prend ~2 min 20 |
| Génération (`tg`) | ~5 tokens/s | Une réponse de 150 tokens prend ~30 s |
| Matériel | 8 vCPU partagés, AVX2 sans AVX-512, 16 Go | 6 threads optimum, 8 s'effondre |

**Le goulot principal est la lecture du prompt**, pas la génération : ce qui évite de
relire le prompt (cache) rapporte plus que ce qui accélère la génération.

---

## 1. À faire d'abord : options de llama.cpp déjà installé (aucun nouveau dépôt)

Dépôt : [ggml-org/llama.cpp](https://github.com/ggml-org/llama.cpp) · MIT · actif (commit du
jour) · 131 k étoiles. Options vérifiées dans la doc actuelle de `llama-server`
(`tools/server/README.md`). Elles s'ajoutent à `command:` du service `llm` dans
`docker-compose.yml`.

| Piste | Options | Gain attendu | Coût / risque |
|---|---|---|---|
| **Cache de prompt** (réutiliser le début du prompt entre deux messages) | `--cache-reuse 256` ; plafonner le cache en RAM avec `-cram 1024` (défaut : 8 192 Mo, trop pour `mem_limit: 8g`) | Le plus gros gain : seuls les nouveaux tokens sont relus. ArIès est déjà conçu pour (mémoire injectée dans le dernier message, README « Mémoire ») | Quasi nul ; vérifier la RAM |
| **Cache KV quantifié** | `-ctk q8_0 -ctv q8_0` | Moins de mémoire lue à chaque token, contexte plus grand possible | Qualité quasi inchangée en q8_0 ; éviter q4_0 |
| **Flash attention** | `-fa on` (défaut `auto`) | Attention plus rapide sur long contexte ; nécessaire pour quantifier le cache V | À mesurer : sur CPU, le gain varie |
| **Décodage spéculatif n-gramme** (devine la suite à partir du texte déjà vu, sans second modèle) | `--spec-type ngram-mod` (ou `ngram-simple`) | Utile quand la réponse recopie le contexte (résumés, citations, code) ; nul sinon | Aucun modèle en plus, aucun coût RAM |
| **Décodage spéculatif avec petit modèle** (un modèle « brouillon » propose, le gros vérifie) | `-md /models/Qwen3-0.6B-Q8_0.gguf --spec-type draft-simple --spec-draft-n-max 4` | Rapporté ~×1,3 en moyenne pour Qwen3-8B + Qwen3-0.6B (étude Intel, OpenVINO, pas llama.cpp) | Le brouillon prend des cœurs aux mêmes 6 vCPU partagés : **peut ralentir**. +0,6 Go de RAM. À mesurer absolument |
| **Version figée de l'image** | `ghcr.io/ggml-org/llama.cpp:server-bXXXX` au lieu de `:server` | Pas un gain, mais des mesures reproductibles et pas de régression surprise | Mise à jour manuelle |
| **Mémoire verrouillée** | `--mlock` | Évite que le modèle soit évincé de la RAM (pages swappées) | Nécessite `ulimits: memlock: -1` dans compose |

Modèle brouillon : [Qwen/Qwen3-0.6B-GGUF](https://huggingface.co/Qwen/Qwen3-0.6B-GGUF)
(même tokenizer que Qwen3-8B, condition indispensable).

## 2. À évaluer : moteurs alternatifs

### ik_llama.cpp ⭐ meilleur candidat

- **Dépôt** : [ikawrakow/ik_llama.cpp](https://github.com/ikawrakow/ik_llama.cpp) · MIT ·
  actif (commit du jour) · 3,3 k étoiles. Fork de llama.cpp par l'auteur de nombreuses
  quantifications de llama.cpp.
- **Pourquoi** : orienté performance CPU. Lecture du prompt annoncée plus rapide que
  llama.cpp ([comparaison CPU](https://github.com/ikawrakow/ik_llama.cpp/discussions/164)),
  quantifications « R4 » à lignes entrelacées pour AVX2, et **re-paquetage au chargement**
  (`-rtr` / `--run-time-repack`) qui les applique à un GGUF existant sans requantifier.
- **Compatibilité** : même format GGUF et même API serveur compatible OpenAI ;
  `aries-llm` n'a rien à changer, seul le service `llm` change.
- **Limites** : les comparaisons publiées datent surtout de 2024-2025 et viennent de
  l'auteur ou de la communauté ; llama.cpp a rattrapé une partie de l'écart depuis.
  Pas d'image Docker officielle connue : compiler dans `deploy/ik-llama/Dockerfile`,
  comme `deploy/whisper` (`-DGGML_NATIVE=OFF -DGGML_AVX2=ON`, sans AVX-512).
- **Test** : `llama-bench -m Qwen3-8B-Q4_K_M.gguf -t 6 -p 512 -n 128`, avec et sans `-rtr 1`,
  à comparer à 21,1 / 5,0.

### OpenVINO GenAI (seulement si le CPU est Intel)

- **Dépôt** : [openvinotoolkit/openvino.genai](https://github.com/openvinotoolkit/openvino.genai)
  · Apache-2.0 · actif.
- **Pourquoi** : moteur d'Intel optimisé pour ses CPU, avec décodage spéculatif intégré.
  C'est le cadre de l'étude [Qwen3-8B + brouillon](https://huggingface.co/blog/intel-qwen3-agent).
- **Limites** : modèles à convertir au format OpenVINO (pas de GGUF), serveur différent
  (OpenVINO Model Server), écosystème Python lourd. Gain incertain sur un CPU AMD.
- **Avant tout** : vérifier le processeur du VPS (`lscpu | grep "Model name"`).

### mistral.rs (moteur natif Rust)

- **Dépôt** : [EricLBuehler/mistral.rs](https://github.com/EricLBuehler/mistral.rs) · MIT ·
  actif · 7,8 k étoiles. Déjà listé au README §7.
- **Pourquoi** : Rust, lit le GGUF, API compatible OpenAI, décodage spéculatif.
- **Limites** : aucune mesure CPU fiable trouvée face à llama.cpp ; à évaluer seulement si
  ik_llama.cpp déçoit. Pas un gain de vitesse garanti.

## 3. À surveiller (pas utilisable aujourd'hui sur ce VPS)

| Piste | Dépôt / source | Pourquoi pas maintenant |
|---|---|---|
| **MTP de Qwen 3.6** (le modèle prédit plusieurs tokens lui-même, ~×2 annoncé sur GPU) | llama.cpp `--spec-type draft-mtp` (fusionné en mai 2026) ; GGUF MTP chez ggml-org et Unsloth | Qwen 3.6 MTP n'existe qu'en 27B dense et 35B-A3B : trop gros pour 16 Go. À reprendre si un petit Qwen avec MTP sort |
| **Brouillon EAGLE-3 pour Qwen3-8B** (plus précis qu'un petit modèle) | llama.cpp `--spec-type draft-eagle3` ; brouillons `AngelSlim/Qwen3-8B_eagle3`, `Tengyunw/qwen3_8b_eagle3`, `RedHatAI/Qwen3-8B-speculator.eagle3` | Pas de conversion GGUF publiée trouvée ; conversion non vérifiée |
| **Modèles 1 bit** (2 à 6 fois plus rapides sur x86 selon l'article) | [microsoft/BitNet](https://github.com/microsoft/BitNet) · MIT · actif ([article](https://arxiv.org/pdf/2410.16144)) | Il faut des modèles entraînés en 1 bit (BitNet b1.58 2B) : trop faibles pour un assistant francophone |
| **Noyaux à tables (LUT) bas bits** | [microsoft/T-MAC](https://github.com/microsoft/T-MAC) · MIT | Dernier commit en juin 2025 (critère « < 6 mois » non rempli) ; intégré en partie à BitNet |

## 4. Écartés

| Dépôt | Raison |
|---|---|
| [mozilla-ai/llamafile](https://github.com/mozilla-ai/llamafile) | Ses noyaux CPU (tinyBLAS) ont été [intégrés à llama.cpp](https://github.com/Mozilla-Ocho/llamafile/releases/tag/0.7) : rien de plus à gagner |
| [intel/ipex-llm](https://github.com/intel/ipex-llm) | **Archivé** (janvier 2026) |
| [vllm-project/vllm](https://github.com/vllm-project/vllm), [LMCache/LMCache](https://github.com/LMCache/LMCache) | Faits pour servir beaucoup d'utilisateurs en parallèle, surtout sur GPU ; lourds, sans gain pour un seul utilisateur sur CPU |
| [kvcache-ai/ktransformers](https://github.com/kvcache-ai/ktransformers) | Hybride CPU + GPU : demande un GPU |
| [hao-ai-lab/LookaheadDecoding](https://github.com/hao-ai-lab/LookaheadDecoding) | Inactif depuis mars 2025, pensé pour GPU ; l'idée existe dans llama.cpp (`ngram-*`) |
| [SafeAILab/EAGLE](https://github.com/SafeAILab/EAGLE) | Sert à entraîner des brouillons EAGLE (GPU) ; l'utilisation passe par llama.cpp (§3) |
| [OpenNMT/CTranslate2](https://github.com/OpenNMT/CTranslate2) (pour le LLM) | Poids en int8 au mieux : deux fois plus de mémoire lue par token que Q4, donc plus lent sur ce CPU limité par la mémoire |
| [mostlygeek/llama-swap](https://github.com/mostlygeek/llama-swap) | Change de modèle à la demande : pratique, mais n'accélère rien |

## 5. Bonus : la voix (latence actuelle ~6 s pour 3 s de parole)

| Piste | Dépôt | Remarque |
|---|---|---|
| Fenêtre audio réduite | whisper.cpp `--audio-ctx` (déjà installé) | Whisper traite toujours 30 s d'audio ; réduire la fenêtre accélère l'encodeur, au risque de couper les phrases longues |
| faster-whisper | [SYSTRAN/faster-whisper](https://github.com/SYSTRAN/faster-whisper) · MIT · actif · 26 k étoiles | Whisper sur CTranslate2 en int8 ; serveur Python à ajouter. Comparer à whisper.cpp sur le VPS avant tout changement |

---

## Protocole de mesure (avant toute adoption)

1. Sur le VPS, arrêter le service `llm` (pour ne pas partager les vCPU), puis
   `docker run --rm -v /opt/aries/models:/models <image> llama-bench -m /models/Qwen3-8B-Q4_K_M.gguf -t 6 -p 512 -n 128`.
2. Répéter 3 fois (vCPU partagés : ±10 % de bruit) et garder la médiane.
3. Pour le décodage spéculatif : `llama-server` réel, même question longue, et comparer
   la ligne `eval time … tokens per second` des journaux avec et sans l'option.
4. Vérifier la RAM (`docker stats`) : `llm` doit rester sous `mem_limit: 8g`.
5. Tester la qualité sur une vraie conversation (mémoire, outils, français) : LFM2.5 était
   3 à 4 fois plus rapide mais a été rejeté sur ce point.
6. Consigner les résultats dans `docs/benchmarks.md` et l'adoption dans `docs/dependencies.md`.

Ordre conseillé : §1 cache de prompt → §1 KV q8_0 + flash attention → §1 n-gramme →
§2 ik_llama.cpp → §1 petit modèle brouillon.
