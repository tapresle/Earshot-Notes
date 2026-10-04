CREATE TABLE meetings (
    id          INTEGER PRIMARY KEY,
    title       TEXT    NOT NULL,
    created_at  INTEGER NOT NULL, -- Unix milliseconds
    started_at  INTEGER,          -- set when recording first starts
    ended_at    INTEGER           -- set when recording stops
);

CREATE INDEX meetings_created_at ON meetings (created_at DESC);

CREATE TABLE notes (
    meeting_id     INTEGER PRIMARY KEY REFERENCES meetings (id) ON DELETE CASCADE,
    body_markdown  TEXT    NOT NULL DEFAULT '',
    updated_at     INTEGER NOT NULL
);
