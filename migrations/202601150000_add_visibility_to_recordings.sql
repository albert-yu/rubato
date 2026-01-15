CREATE TYPE visibility AS ENUM ('public', 'unlisted', 'private');

ALTER TABLE recordings
ADD COLUMN visibility visibility NOT NULL DEFAULT 'public';
