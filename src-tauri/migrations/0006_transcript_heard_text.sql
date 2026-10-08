-- What Whistle heard, kept only for segments whose text the vocabulary
-- corrected. NULL means `text` is exactly what Whistle heard. Reapplying the
-- vocabulary starts from this, so removing a correction restores the original.
ALTER TABLE transcript_segments ADD COLUMN heard_text TEXT;
