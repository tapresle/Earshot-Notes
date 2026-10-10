-- Questions asked about a meeting and the local model's answers (#24). A
-- question and its answer are saved together once the answer is complete.
CREATE TABLE chat_messages (
    id             INTEGER PRIMARY KEY,
    meeting_id     INTEGER NOT NULL REFERENCES meetings (id) ON DELETE CASCADE,
    role           TEXT    NOT NULL CHECK (role IN ('user', 'assistant')),
    body_markdown  TEXT    NOT NULL,
    -- The sources an answer cites, as a JSON array (`store::Citation`); '[]' for questions.
    sources_json   TEXT    NOT NULL DEFAULT '[]',
    model_id       TEXT, -- the model that answered; NULL for questions
    created_at     INTEGER NOT NULL
);

CREATE INDEX chat_messages_meeting ON chat_messages (meeting_id, id);
