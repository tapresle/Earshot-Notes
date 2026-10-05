-- The sidebar pages through meetings newest first, with ties broken by id.
-- `meetings_created_at` orders ties by ascending rowid, so SQLite had to sort
-- each page; an index on both columns serves the seek and the order.
CREATE INDEX meetings_page ON meetings (created_at, id);

DROP INDEX meetings_created_at;
