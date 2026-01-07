-- Add migration script here
ALTER TABLE recordings ADD COLUMN mime_type TEXT NOT NULL DEFAULT 'application/octet-stream';