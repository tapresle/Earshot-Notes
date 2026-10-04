CREATE TABLE summaries (
    id             INTEGER PRIMARY KEY,
    meeting_id     INTEGER NOT NULL REFERENCES meetings (id) ON DELETE CASCADE,
    upto_ms        INTEGER NOT NULL, -- transcript covered, as an offset from meetings.started_at
    is_final       INTEGER NOT NULL CHECK (is_final IN (0, 1)), -- 0 while the meeting was still recording
    body_markdown  TEXT    NOT NULL,
    model_id       TEXT    NOT NULL, -- kept even if the model is later removed
    created_at     INTEGER NOT NULL
);

CREATE INDEX summaries_meeting ON summaries (meeting_id, created_at DESC);

-- Local LLM files. Version, checksum, source, and minimum runtime are recorded
-- so models can be updated later.
CREATE TABLE models (
    id            TEXT    PRIMARY KEY,
    display_name  TEXT    NOT NULL,
    file_path     TEXT    NOT NULL,
    size_bytes    INTEGER NOT NULL,
    sha256        TEXT    NOT NULL UNIQUE,
    source        TEXT    NOT NULL CHECK (source IN ('import', 'download')),
    source_url    TEXT,
    version       TEXT,
    min_runtime   TEXT,     -- the oldest llama-cpp-2 version known to run it
    added_at      INTEGER NOT NULL
);

CREATE TABLE settings (
    key    TEXT PRIMARY KEY,
    value  TEXT NOT NULL
);
