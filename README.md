# rubato

Upload your recordings of classical pieces!

## Requirements

* [`cargo`](https://doc.rust-lang.org/cargo/getting-started/installation.html)
* [`bun`](https://bun.sh/) to manage Node packages
* [`docker`](https://www.docker.com/products/docker-desktop/) to run a
local PostgreSQL database
* a local `.env` file (see `.env.example`)

## Local dev commands

CSS doesn't look right? Be sure to run:

```sh
bun build:css
```

This will update the compiled Tailwind classes used.

During normal development, you can just run

```sh
cargo run
```

The app will be running at `http://localhost:3000`.

## Importing composition metadata

Download dump.json from
[albert-yu/openopus](https://github.com/albert-yu/openopus).

Then, go to /admin/import to import the data via the UI.
