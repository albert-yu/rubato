CREATE TABLE IF NOT EXISTS todos (
    id SERIAL PRIMARY KEY,
    task TEXT NOT NULL,
    completed BOOLEAN NOT NULL DEFAULT FALSE
);

INSERT INTO todos (task) VALUES ('Learn Rust'), ('Build a Web App');
