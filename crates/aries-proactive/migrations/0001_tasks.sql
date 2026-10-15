-- Tâches planifiées. Horodatages en UTC, ISO 8601 (comparables comme chaînes).
CREATE TABLE tasks (
    id              INTEGER PRIMARY KEY,
    title           TEXT    NOT NULL,
    -- 'reminder' : envoyer `content` tel quel ; 'agent' : `content` est une consigne
    -- exécutée par l'agent, dont la réponse est envoyée (sauf « RIEN »).
    kind            TEXT    NOT NULL CHECK (kind IN ('reminder', 'agent')),
    content         TEXT    NOT NULL,
    -- Expression cron (heure locale) pour une tâche récurrente ; NULL = une seule fois.
    cron            TEXT,
    next_run_at     TEXT,
    last_run_at     TEXT,
    -- Conversation où l'agent travaille (créée à la première exécution).
    conversation_id INTEGER,
    active          INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX idx_tasks_due ON tasks (active, next_run_at);

CREATE TABLE task_runs (
    id          INTEGER PRIMARY KEY,
    task_id     INTEGER NOT NULL REFERENCES tasks (id) ON DELETE CASCADE,
    started_at  TEXT    NOT NULL,
    finished_at TEXT    NOT NULL,
    success     INTEGER NOT NULL,
    output      TEXT    NOT NULL
);
