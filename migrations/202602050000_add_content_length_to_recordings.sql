-- Add migration script here
ALTER TABLE recordings ADD COLUMN content_length BIGINT NOT NULL DEFAULT 0;
