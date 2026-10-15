# Activer le chat cloud (Gemini)

Par défaut, ArIès répond avec le modèle local Qwen3-8B (environ 5 tokens/s, soit 15 à
40 s par réponse). Avec une clé Gemini, le chat passe par le cloud : **premier mot en
moins d'une seconde**, environ 100 tokens/s.

Le modèle local reste actif :
- il **prend le relais automatiquement** si Gemini ne répond pas (panne, quota, réseau) ;
- il fait toujours la **mise à jour de la mémoire** en tâche de fond.

---

## 1. Créer la clé API

1. Ouvre https://aistudio.google.com/apikey et connecte-toi avec ton compte Google.
2. Clique sur **« Create API key »**. Choisis un projet existant ou laisse-le en créer un.
3. Copie la clé (elle commence par `AIza…`). **Ne la partage pas** : quiconque la possède
   peut utiliser ton quota.

> **Gratuit ou payant ?** L'offre gratuite suffit pour un usage personnel, mais Google
> peut utiliser les conversations pour améliorer ses modèles. Pour l'éviter, active la
> facturation sur le projet dans Google AI Studio : quelques centimes par jour pour un
> usage personnel.

---

## 2. Mettre la clé sur le serveur

Depuis PowerShell sur ton PC :

```powershell
ssh root@185.216.26.9
```

Puis sur le serveur :

```bash
cd /opt/aries
nano .env
```

Remplis la ligne `GEMINI_API_KEY`, sans espace ni guillemets :

```
GEMINI_API_KEY=AIzaSy...ta-clé...
```

Enregistre avec **Ctrl+O** puis **Entrée**, et quitte avec **Ctrl+X**.

Redémarre ArIès pour qu'il lise la clé (seul le service `aries` redémarre, en quelques
secondes) :

```bash
docker compose up -d --force-recreate aries
```

---

## 3. Vérifier que le cloud est actif

```bash
docker compose logs aries | grep "chat :"
```

| Message affiché | Signification |
|---|---|
| `chat : cloud, avec bascule sur le modèle local` | ✅ Le cloud est actif |
| `chat : clé cloud absente, modèle local uniquement` | La clé est vide ou mal écrite dans `.env` |

Ensuite, pose une question sur https://aries.antoine-vivian.fr : la réponse doit
commencer presque instantanément.

Si Gemini refuse une requête (clé invalide, quota dépassé…), ArIès répond quand même
avec le modèle local, plus lentement. Pour voir pourquoi :

```bash
docker compose logs aries | grep "bascule"
```

| Erreur dans le log | Cause probable |
|---|---|
| `400` / `API key not valid` | Clé mal copiée |
| `403` | API non activée sur le projet, ou clé restreinte |
| `404` / `model not found` | Nom de modèle inconnu (voir étape 4) |
| `429` | Quota gratuit dépassé : attendre, ou activer la facturation |

---

## 4. (Optionnel) Changer de modèle Gemini

Le modèle est défini dans `config/aries.toml` :

```toml
[llm.cloud]
model = "gemini-3.5-flash"
```

Pour voir les modèles disponibles avec ta clé :

```bash
KEY=$(grep ^GEMINI_API_KEY= /opt/aries/.env | cut -d= -f2)
curl -s https://generativelanguage.googleapis.com/v1beta/openai/models \
  -H "Authorization: Bearer $KEY" | grep -o '"id": *"[^"]*flash[^"]*"'
```

Après avoir modifié `config/aries.toml`, il faut reconstruire l'image, car la
configuration y est copiée :

```bash
docker compose up -d --build aries
```

---

## Désactiver le cloud

Vide la clé dans `.env` (`GEMINI_API_KEY=`), puis :

```bash
docker compose up -d --force-recreate aries
```

ArIès repasse en 100 % local.

---

## Ce qui part chez Google

Quand le cloud est actif, chaque message envoyé au chat inclut :
- la personnalité d'ArIès ;
- l'historique récent de la conversation ;
- les faits et souvenirs que la mémoire a jugés utiles pour ce message.

La base de la mémoire (`data/aries.db`, Qdrant) et sa mise à jour restent **sur le
serveur**.
