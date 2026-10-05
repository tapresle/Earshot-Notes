-- Full-text indexes for sidebar search over titles, notes, and transcripts.
-- Each index is an external-content FTS5 table: it reads text from the table
-- it covers instead of storing a second copy, and the triggers below keep it
-- in step. Deleting from an external-content index needs the old values,
-- which the triggers pass through.

CREATE VIRTUAL TABLE meetings_fts USING fts5 (
    title,
    content = 'meetings',
    content_rowid = 'id',
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE VIRTUAL TABLE notes_fts USING fts5 (
    body_markdown,
    content = 'notes',
    content_rowid = 'meeting_id',
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE VIRTUAL TABLE transcript_fts USING fts5 (
    text,
    content = 'transcript_segments',
    content_rowid = 'id',
    tokenize = 'unicode61 remove_diacritics 2'
);

-- Index everything saved before this migration.
INSERT INTO meetings_fts (meetings_fts) VALUES ('rebuild');
INSERT INTO notes_fts (notes_fts) VALUES ('rebuild');
INSERT INTO transcript_fts (transcript_fts) VALUES ('rebuild');

CREATE TRIGGER meetings_fts_insert AFTER INSERT ON meetings BEGIN
    INSERT INTO meetings_fts (rowid, title) VALUES (new.id, new.title);
END;

CREATE TRIGGER meetings_fts_delete AFTER DELETE ON meetings BEGIN
    INSERT INTO meetings_fts (meetings_fts, rowid, title) VALUES ('delete', old.id, old.title);
END;

CREATE TRIGGER meetings_fts_update AFTER UPDATE OF title ON meetings BEGIN
    INSERT INTO meetings_fts (meetings_fts, rowid, title) VALUES ('delete', old.id, old.title);
    INSERT INTO meetings_fts (rowid, title) VALUES (new.id, new.title);
END;

CREATE TRIGGER notes_fts_insert AFTER INSERT ON notes BEGIN
    INSERT INTO notes_fts (rowid, body_markdown) VALUES (new.meeting_id, new.body_markdown);
END;

CREATE TRIGGER notes_fts_delete AFTER DELETE ON notes BEGIN
    INSERT INTO notes_fts (notes_fts, rowid, body_markdown)
    VALUES ('delete', old.meeting_id, old.body_markdown);
END;

CREATE TRIGGER notes_fts_update AFTER UPDATE OF body_markdown ON notes BEGIN
    INSERT INTO notes_fts (notes_fts, rowid, body_markdown)
    VALUES ('delete', old.meeting_id, old.body_markdown);
    INSERT INTO notes_fts (rowid, body_markdown) VALUES (new.meeting_id, new.body_markdown);
END;

CREATE TRIGGER transcript_fts_insert AFTER INSERT ON transcript_segments BEGIN
    INSERT INTO transcript_fts (rowid, text) VALUES (new.id, new.text);
END;

CREATE TRIGGER transcript_fts_delete AFTER DELETE ON transcript_segments BEGIN
    INSERT INTO transcript_fts (transcript_fts, rowid, text) VALUES ('delete', old.id, old.text);
END;

CREATE TRIGGER transcript_fts_update AFTER UPDATE OF text ON transcript_segments BEGIN
    INSERT INTO transcript_fts (transcript_fts, rowid, text) VALUES ('delete', old.id, old.text);
    INSERT INTO transcript_fts (rowid, text) VALUES (new.id, new.text);
END;
