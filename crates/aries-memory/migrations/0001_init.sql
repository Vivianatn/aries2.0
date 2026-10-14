-- Horodatages en UTC, format ISO 8601 (comparables comme chaînes).

CREATE TABLE conversations (
    id         INTEGER PRIMARY KEY,
    title      TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE messages (
    id              INTEGER PRIMARY KEY,
    conversation_id INTEGER NOT NULL REFERENCES conversations (id) ON DELETE CASCADE,
    role            TEXT    NOT NULL CHECK (role IN ('user', 'assistant')),
    content         TEXT    NOT NULL,
    -- 1 quand la consolidation (faits + souvenir) a traité le message.
    consolidated    INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX idx_messages_conversation ON messages (conversation_id, id);
CREATE INDEX idx_messages_pending ON messages (consolidated, conversation_id);

-- Mémoire sémantique : faits durables sur l'utilisateur.
CREATE TABLE facts (
    id                     INTEGER PRIMARY KEY,
    content                TEXT NOT NULL,
    source_conversation_id INTEGER REFERENCES conversations (id) ON DELETE SET NULL,
    created_at             TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at             TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- Mémoire épisodique : résumés horodatés de morceaux de conversation.
CREATE TABLE episodes (
    id              INTEGER PRIMARY KEY,
    conversation_id INTEGER NOT NULL REFERENCES conversations (id) ON DELETE CASCADE,
    summary         TEXT NOT NULL,
    started_at      TEXT NOT NULL,
    ended_at        TEXT NOT NULL
);
CREATE INDEX idx_episodes_conversation ON episodes (conversation_id, id);
