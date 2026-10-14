# Benchmarks des modèles locaux

VPS PulseHeberg Economy « Silver G1 » : 8 vCPU partagés, AVX2 sans AVX-512, 16 Go, sans GPU.
Mesures `llama-bench` (llama.cpp, `--cpus 6`) du 2026-10-14. `pp512` = lecture du prompt,
`tg` = génération, en tokens/s. Les vCPU étant partagés, compter ±10 % de bruit.

| Modèle | Quantif. | Taille | Threads | pp512 | tg |
|---|---|---|---|---|---|
| **Qwen3-8B** (retenu) | Q4_K_M | 4,7 Go | 6 | 21,1 | 5,0 |
| Qwen3-8B | Q4_K_M | 4,7 Go | 5 | 16,6 | 3,9 |
| Qwen3-8B | Q4_K_M | 4,7 Go | 4 | 14,8 | 3,9 |
| Qwen3.5-9B | Q4_K_M | 5,3 Go | 6 | 18,9 | 4,0 |
| Qwen3.5-4B | Q4_K_M | 2,5 Go | 6 | 35,3 | 6,4 |
| Qwen3.5-4B | Q4_0 | 2,4 Go | 6 | 28,7 | 5,6 |
| LFM2.5-8B-A1B (MoE) | Q4_K_M | 4,8 Go | 6 | 63,6 | 13,3 |
| LFM2.5-8B-A1B (MoE) | Q4_0 | 4,5 Go | 6 | 64,5 | 16,3 |
| LFM2.5-8B-A1B (MoE) | Q4_0 | 4,5 Go | 4 | 49,3 | 19,7 |
| LFM2.5-8B-A1B (MoE) | Q4_0 | 4,5 Go | 8 | 57,4 | 5,9 |

## Conclusions

- **8 threads est à proscrire** : les vCPU partagés s'effondrent.
- **LFM2.5-8B-A1B** est 3 à 4 fois plus rapide, mais **rejeté au test réel** : il recopie le
  bloc de contexte mémoire dans ses réponses, répète les réponses précédentes et se
  trompe sur un calcul de créneaux simple.
- **Qwen3.5-4B** n'apporte que ~×1,3 en génération pour une qualité moindre : pas rentable.
- **Qwen3.5-9B** est plus lent que Qwen3-8B sur ce CPU.
- Pour la vitesse du chat, la solution est le **cloud** (Gemini Flash), avec Qwen3-8B en
  secours et pour la consolidation de la mémoire.
