-- Add migration script here
CREATE TABLE users (
    id SERIAL PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    musician_id INTEGER NOT NULL REFERENCES musicians(id) UNIQUE
);