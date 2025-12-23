-- Add migration script here
CREATE TYPE user_role AS ENUM ('root', 'admin', 'user');

CREATE TABLE users (
    id SERIAL PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    salt TEXT NOT NULL,
    musician_id INTEGER NOT NULL REFERENCES musicians(id) UNIQUE,
    role user_role NOT NULL DEFAULT 'user'
);