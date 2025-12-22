-- Add migration script here
CREATE TABLE musicians (
    id SERIAL PRIMARY KEY,
    handle TEXT NOT NULL UNIQUE,
    given_name TEXT NOT NULL,
    family_name TEXT NOT NULL
);

CREATE TABLE compositions (
    id SERIAL PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    publish_date DATE,
    composer_id INTEGER NOT NULL REFERENCES musicians(id)
);

CREATE TABLE movements (
    id SERIAL PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    index INTEGER NOT NULL,
    composition_id INTEGER NOT NULL REFERENCES compositions(id)
);

CREATE TABLE recordings (
    id SERIAL PRIMARY KEY,
    artist_id INTEGER NOT NULL REFERENCES musicians(id),
    composition_id INTEGER NOT NULL REFERENCES compositions(id),
    movement_id INTEGER REFERENCES movements(id),
    content_hash TEXT NOT NULL,
    file_key TEXT NOT NULL
);
