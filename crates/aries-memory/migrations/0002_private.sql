-- Données privées (ex. contenu d'un mail) : jamais envoyées à un LLM cloud.
-- Une conversation devient privée dès qu'un outil privé y a répondu ; ses souvenirs et
-- les faits qu'elle crée ou modifie le deviennent aussi.
ALTER TABLE conversations ADD COLUMN private INTEGER NOT NULL DEFAULT 0;
ALTER TABLE facts ADD COLUMN private INTEGER NOT NULL DEFAULT 0;
