CREATE TABLE transcript_segments (
    id          INTEGER PRIMARY KEY,
    meeting_id  INTEGER NOT NULL REFERENCES meetings (id) ON DELETE CASCADE,
    source      TEXT    NOT NULL CHECK (source IN ('me', 'them')),
    start_ms    INTEGER NOT NULL, -- offset from meetings.started_at
    end_ms      INTEGER NOT NULL,
    text        TEXT    NOT NULL,
    language    TEXT
);

CREATE INDEX transcript_segments_meeting ON transcript_segments (meeting_id, start_ms);
