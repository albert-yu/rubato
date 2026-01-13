# Rubato

Rubato is a modern, high-performance platform for classical music
recording management and sharing.
It's built with Rust (Axum) on the backend and uses server-side
rendered HTML (Askama) with HTMX for dynamic interactions, styled with Tailwind CSS.

## Project Structure

### Core Application
- **`src/`**: Contains the Rust source code for the backend application.
    - **`main.rs`**: Entry point of the application, sets up the Axum router, database connection, and middleware.
    - **`db.rs`**: Database models and schema definitions using SQLx.
    - **`handlers/`**: Request handlers for different parts of the application.
        - `public.rs`: Public-facing pages (home, profile, player).
        - `auth.rs`: Authentication logic (login, signup).
        - `admin.rs`: Administrative interface for managing data.
    - **`view.rs`**: Askama template structs for rendering HTML.
    - **`extractors.rs`**: Custom Axum extractors (e.g., for authentication).
    - **`middleware.rs`**: Custom middleware (e.g., for ETag handling).
    - **`storage.rs`**: Abstraction for file storage (S3 or local).

### Frontend & Templates
- **`templates/`**: HTML templates using Askama (Jinja-like syntax).
    - **`base.html`**: The main layout template, including the persistent music player and navigation.
    - **`index.html`**, **`profile.html`**, **`recording.html`**: Page templates.
    - **`*_content.html`**: HTMX partials for dynamic content loading.
    - **`admin/`**: Templates for the admin interface.
- **`assets/`**: Static assets like CSS and vendor scripts (HTMX).
- **`input.css`**: Tailwind CSS input file.
- **`tailwind.config.js`**: Tailwind CSS configuration.

### Database & Migrations
- **`migrations/`**: SQL migration files for setting up and evolving the PostgreSQL database schema.

### Scripts & Utilities
- **`scripts/`**: Utility scripts (e.g., for data import).
- **`update.sh`**: Script for updating the application (building CSS, running migrations).

## Technology Stack

- **Backend**: Rust, Axum, Tokio, SQLx
- **Database**: PostgreSQL
- **Frontend**: HTML (Askama), Tailwind CSS, HTMX, JavaScript (Vanilla)
- **Build Tools**: Cargo (Rust), Bun

## Development

1.  **Prerequisites**: Rust, PostgreSQL, Bun.
2.  **Setup**:
    - Configure `.env` (database URL, S3 settings, etc.).
    - Create migrations: Use `sqlx` CLI.
3.  **Run**: `cargo run`.
    - This will also run db migrations on startup
4.  **Update CSS**: `bun build:css`.
