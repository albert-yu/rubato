use argon2::{
    Argon2,
    password_hash::{PasswordHasher, SaltString},
};
use axum::{
    Router,
    extract::{DefaultBodyLimit, FromRef},
    routing::{delete, get, post},
};
use axum_extra::extract::cookie::Key;
use jsonwebtoken::{DecodingKey, EncodingKey};
use rand::rngs::OsRng;
use sqlx::{Pool, Postgres, migrate::MigrateDatabase, postgres::PgPoolOptions};
use std::io::Write;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tower::ServiceBuilder;
use tower_http::services::ServeDir;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod db;
mod extractors;
mod handlers;
mod middleware;
mod storage;
mod view;

use db::*;
use handlers::{admin, auth, public};

#[derive(Clone)]
struct AppState {
    pool: Pool<Postgres>,
    key: Key,
    jwt_encoding_key: EncodingKey,
    jwt_decoding_key: DecodingKey,
    storage: Arc<dyn storage::StorageService>,
    import_cancel_token: Arc<Mutex<Option<CancellationToken>>>,
}

impl FromRef<AppState> for Pool<Postgres> {
    fn from_ref(state: &AppState) -> Self {
        state.pool.clone()
    }
}

impl FromRef<AppState> for Key {
    fn from_ref(state: &AppState) -> Self {
        state.key.clone()
    }
}

impl FromRef<AppState> for Arc<dyn storage::StorageService> {
    fn from_ref(state: &AppState) -> Self {
        state.storage.clone()
    }
}

impl FromRef<AppState> for EncodingKey {
    fn from_ref(state: &AppState) -> Self {
        state.jwt_encoding_key.clone()
    }
}

impl FromRef<AppState> for DecodingKey {
    fn from_ref(state: &AppState) -> Self {
        state.jwt_decoding_key.clone()
    }
}

impl FromRef<AppState> for Arc<Mutex<Option<CancellationToken>>> {
    fn from_ref(state: &AppState) -> Self {
        state.import_cancel_token.clone()
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "rubato=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");

    if !Postgres::database_exists(&db_url).await.unwrap_or(false) {
        tracing::info!("Creating database {}", db_url);
        Postgres::create_database(&db_url).await?;
    } else {
        tracing::info!("Database already exists");
    }

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await?;

    // Run migrations
    tracing::info!("Running migrations...");
    sqlx::migrate!("./migrations").run(&pool).await?;
    tracing::info!("Migrations executed successfully.");

    // Check for root user
    let root_user = sqlx::query_as::<_, User>(
        "SELECT u.*, m.handle FROM users u JOIN musicians m ON u.musician_id = m.id WHERE u.role = 'root'"
    )
    .fetch_optional(&pool)
    .await?;

    if root_user.is_none() {
        println!("Root user not found. Please create one.");

        print!("Email: ");
        std::io::stdout().flush()?;
        let mut email = String::new();
        std::io::stdin().read_line(&mut email)?;
        let email = email.trim();

        print!("Password: ");
        std::io::stdout().flush()?;
        let password = rpassword::read_password()?;

        // Create Musician for root
        let musician = sqlx::query_as::<_, Musician>(
            "INSERT INTO musicians (handle, given_name, family_name) VALUES ($1, $2, $3) RETURNING *"
        )
        .bind("root")
        .bind("Root")
        .bind("User")
        .fetch_one(&pool)
        .await?;

        // Hash password
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let password_hash = argon2
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| anyhow::anyhow!("Password hashing failed: {}", e))?
            .to_string();

        // Create User
        sqlx::query(
            "INSERT INTO users (email, password_hash, salt, musician_id, role) VALUES ($1, $2, $3, $4, 'root')"
        )
        .bind(email)
        .bind(password_hash)
        .bind(salt.as_str())
        .bind(musician.id)
        .execute(&pool)
        .await?;

        println!("Root user created successfully.");
    } else {
        tracing::info!("Root user already exists.");
    }

    let key = Key::generate();

    let jwt_secret = std::env::var("JWT_SECRET").expect("JWT_SECRET must be set");

    let jwt_encoding_key = EncodingKey::from_secret(jwt_secret.as_bytes());

    let jwt_decoding_key = DecodingKey::from_secret(jwt_secret.as_bytes());

    let storage: Arc<dyn storage::StorageService> =
        if std::env::var("APP_ENV").unwrap_or_default() == "production" {
            tracing::info!("Initializing S3 Storage");

            let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;

            let client = aws_sdk_s3::Client::new(&config);
            let bucket =
                std::env::var("S3_BUCKET_NAME").expect("S3_BUCKET_NAME must be set in production");
            let public_url = std::env::var("S3_PUBLIC_URL")
                .unwrap_or_else(|_| format!("https://{}.s3.amazonaws.com", bucket));
            Arc::new(storage::S3Storage::new(client, bucket, public_url))
        } else {
            tracing::info!("Initializing Local Storage");
            Arc::new(storage::LocalStorage::new("uploads", "/uploads"))
        };

    let app_state = AppState {
        pool,
        key,
        jwt_encoding_key,
        jwt_decoding_key,
        storage,
        import_cancel_token: Arc::new(Mutex::new(None)),
    };

    let app = Router::new()
        .route("/", get(public::index))
        .route(
            "/settings",
            get(public::settings).post(public::settings_post),
        )
        .route(
            "/upload",
            get(public::upload)
                .post(public::upload_post)
                .layer(DefaultBodyLimit::max(1024 * 1024 * 50)),
        )
        .route(
            "/upload/select-composition/{id}",
            get(public::select_composition),
        )
        .route("/upload/reset-composition", get(public::reset_composition))
        .route("/audio/{key}", get(public::serve_audio))
        .route("/login", get(auth::login_form).post(auth::login_post))
        .route("/signup", get(auth::signup_form).post(auth::signup_post))
        .route("/logout", post(auth::logout))
        .route("/search/compositions", get(public::search_compositions))
        // Admin
        .route("/admin", get(admin::admin_index))
        .route(
            "/admin/musicians",
            get(admin::admin_musicians).post(admin::admin_musician_create),
        )
        .route("/admin/musician/new", get(admin::admin_musician_new))
        .route(
            "/admin/musician/{id}",
            get(admin::admin_musician_edit)
                .post(admin::admin_musician_update)
                .delete(admin::admin_musician_delete),
        )
        .route(
            "/admin/compositions",
            get(admin::admin_compositions).post(admin::admin_composition_create),
        )
        .route("/admin/composition/new", get(admin::admin_composition_new))
        .route(
            "/admin/composition/{id}",
            get(admin::admin_composition_edit)
                .post(admin::admin_composition_update)
                .delete(admin::admin_composition_delete),
        )
        .route(
            "/admin/composition/{id}/movements",
            post(admin::admin_movement_create),
        )
        .route(
            "/admin/movements/{id}",
            delete(admin::admin_movement_delete),
        )
        .route(
            "/admin/recordings",
            get(admin::admin_recordings).post(admin::admin_recording_create),
        )
        .route("/admin/recording/new", get(admin::admin_recording_new))
        .route(
            "/admin/recording/{id}",
            get(admin::admin_recording_edit)
                .post(admin::admin_recording_update)
                .delete(admin::admin_recording_delete),
        )
        .route(
            "/admin/import",
            get(admin::admin_import)
                .post(admin::admin_import_start)
                .layer(DefaultBodyLimit::max(1024 * 1024 * 50)),
        )
        .route("/admin/import/cancel", post(admin::admin_import_cancel))
        .route("/{handle}", get(public::profile))
        .route(
            "/{handle}/recordings/{slug_id}",
            get(public::recording_detail),
        )
        .route(
            "/{handle}/recordings/{slug_id}/edit",
            get(public::recording_edit)
                .post(public::recording_update)
                .delete(public::recording_delete),
        )
        .nest_service(
            "/assets",
            ServiceBuilder::new()
                .layer(axum::middleware::from_fn(middleware::etag_middleware))
                .service(ServeDir::new("assets")),
        )
        .nest_service("/uploads", ServeDir::new("uploads"))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(app_state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::info!("listening on http://{}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
