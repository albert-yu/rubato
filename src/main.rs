use argon2::{
    Argon2, PasswordHash,
    password_hash::{PasswordHasher, PasswordVerifier, SaltString},
};
use axum::{
    Form, Router,
    extract::{FromRequestParts, Path, State},
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Redirect, Response},
    routing::{delete, get, post},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, Key, SameSite};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Postgres, migrate::MigrateDatabase, postgres::PgPoolOptions};
use std::io::Write;
use std::net::SocketAddr;
use tower_http::services::ServeDir;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod db;
mod storage;
mod view;
use db::*;
use std::sync::Arc;
use view::*;

struct AuthUser(User);

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: String,
    exp: usize,
}

#[derive(Clone)]
struct AppState {
    pool: Pool<Postgres>,
    key: Key,
    jwt_encoding_key: EncodingKey,
    jwt_decoding_key: DecodingKey,
    storage: Arc<dyn storage::StorageService>,
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

use axum::extract::FromRef;

impl<S> FromRequestParts<S> for AuthUser
where
    Pool<Postgres>: FromRef<S>,
    DecodingKey: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let pool = Pool::<Postgres>::from_ref(state);
        let decoding_key = DecodingKey::from_ref(state);
        let jar = CookieJar::from_request_parts(parts, state)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Cookie error").into_response())?;

        if let Some(cookie) = jar.get("auth_token") {
            let token = cookie.value();
            let validation = Validation::default();
            if let Ok(token_data) = decode::<Claims>(token, &decoding_key, &validation) {
                if let Ok(id) = token_data.claims.sub.parse::<i32>() {
                    if let Ok(user) = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
                        .bind(id)
                        .fetch_one(&pool)
                        .await
                    {
                        return Ok(AuthUser(user));
                    }
                }
            }
        }

        tracing::debug!("Auth failed: No valid session found.");
        Err((StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response())
    }
}

struct OptionalAuthUser(Option<User>);

impl<S> FromRequestParts<S> for OptionalAuthUser
where
    Pool<Postgres>: FromRef<S>,
    DecodingKey: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let pool = Pool::<Postgres>::from_ref(state);
        let decoding_key = DecodingKey::from_ref(state);
        let jar = CookieJar::from_request_parts(parts, state)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Cookie error").into_response())?;

        if let Some(cookie) = jar.get("auth_token") {
            let token = cookie.value();
            let validation = Validation::default();
            if let Ok(token_data) = decode::<Claims>(token, &decoding_key, &validation) {
                if let Ok(id) = token_data.claims.sub.parse::<i32>() {
                    if let Ok(user) = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
                        .bind(id)
                        .fetch_one(&pool)
                        .await
                    {
                        return Ok(OptionalAuthUser(Some(user)));
                    }
                }
            }
        }

        Ok(OptionalAuthUser(None))
    }
}

// --- Auth Templates ---

#[derive(Deserialize)]
struct LoginPayload {
    email: String,
    password: String,
}

// --- Common ---

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
    let root_user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE role = 'root'")
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
    };

    let app = Router::new()
        .route("/", get(index))
        .route("/settings", get(settings).post(settings_post))
        .route("/upload", get(upload))
        .route("/login", get(login_form).post(login_post))
        .route("/logout", post(logout))
        // Admin
        .route("/admin", get(admin_index))
        .route(
            "/admin/musicians",
            get(admin_musicians).post(admin_musician_create),
        )
        .route("/admin/musician/new", get(admin_musician_new))
        .route(
            "/admin/musician/{id}",
            get(admin_musician_edit)
                .post(admin_musician_update)
                .delete(admin_musician_delete),
        )
        .route(
            "/admin/compositions",
            get(admin_compositions).post(admin_composition_create),
        )
        .route("/admin/composition/new", get(admin_composition_new))
        .route(
            "/admin/composition/{id}",
            get(admin_composition_edit)
                .post(admin_composition_update)
                .delete(admin_composition_delete),
        )
        .route(
            "/admin/composition/{id}/movements",
            post(admin_movement_create),
        )
        .route("/admin/movements/{id}", delete(admin_movement_delete))
        .route(
            "/admin/recordings",
            get(admin_recordings).post(admin_recording_create),
        )
        .route("/admin/recording/new", get(admin_recording_new))
        .route(
            "/admin/recording/{id}",
            get(admin_recording_edit)
                .post(admin_recording_update)
                .delete(admin_recording_delete),
        )
        .nest_service("/assets", ServeDir::new("assets"))
        .nest_service("/uploads", ServeDir::new("uploads"))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(app_state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::info!("listening on http://{}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

// --- Auth Handlers ---
async fn login_form() -> impl IntoResponse {
    HtmlTemplate(LoginTemplate { error: None })
}

async fn login_post(
    State(pool): State<Pool<Postgres>>,
    State(encoding_key): State<EncodingKey>,
    jar: CookieJar,
    Form(payload): Form<LoginPayload>,
) -> impl IntoResponse {
    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE email = $1")
        .bind(&payload.email)
        .fetch_optional(&pool)
        .await
        .unwrap_or(None);

    if let Some(user) = user {
        let parsed_hash = PasswordHash::new(&user.password_hash).unwrap();
        if Argon2::default()
            .verify_password(payload.password.as_bytes(), &parsed_hash)
            .is_ok()
        {
            let claims = Claims {
                sub: user.id.to_string(),
                exp: (chrono::Utc::now() + chrono::Duration::hours(24)).timestamp() as usize,
            };

            if let Ok(token) = encode(&Header::default(), &claims, &encoding_key) {
                let mut cookie = Cookie::new("auth_token", token);
                cookie.set_http_only(true);
                cookie.set_same_site(SameSite::Lax);
                cookie.set_path("/");
                return (jar.add(cookie), Redirect::to("/admin")).into_response();
            }
        }
    }

    (
        jar,
        HtmlTemplate(LoginTemplate {
            error: Some("Invalid email or password".to_string()),
        }),
    )
        .into_response()
}

async fn logout(jar: CookieJar) -> impl IntoResponse {
    (
        jar.remove(Cookie::from("auth_token")),
        Redirect::to("/login"),
    )
}

// --- Todo Handlers ---
async fn index(auth: OptionalAuthUser, State(pool): State<Pool<Postgres>>) -> impl IntoResponse {
    let recordings = sqlx::query_as::<_, RecordingFeedItem>(
        r#"
        SELECT 
            m.handle as artist_handle,
            c.title as composition_title,
            mv.index as movement_index,
            mv.title as movement_title,
            r.created_at,
            r.file_key
        FROM recordings r
        JOIN musicians m ON r.artist_id = m.id
        JOIN compositions c ON r.composition_id = c.id
        LEFT JOIN movements mv ON r.movement_id = mv.id
        ORDER BY r.created_at DESC
        "#
    )
    .fetch_all(&pool)
    .await
    .unwrap_or_default();

    HtmlTemplate(IndexTemplate { 
        current_user: auth.0,
        recordings,
    })
}

async fn settings(auth: AuthUser, State(pool): State<Pool<Postgres>>) -> impl IntoResponse {
    let musician = sqlx::query_as::<_, Musician>("SELECT * FROM musicians WHERE id = $1")
        .bind(auth.0.musician_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    HtmlTemplate(SettingsTemplate {
        current_user: Some(auth.0),
        musician,
    })
}

async fn settings_post(
    auth: AuthUser,
    State(pool): State<Pool<Postgres>>,
    Form(form): Form<CreateMusician>,
) -> impl IntoResponse {
    let _ = sqlx::query(
        "UPDATE musicians SET handle = $1, given_name = $2, family_name = $3 WHERE id = $4",
    )
    .bind(form.handle)
    .bind(form.given_name)
    .bind(form.family_name)
    .bind(auth.0.musician_id)
    .execute(&pool)
    .await
    .unwrap();
    Redirect::to("/settings")
}

async fn upload(auth: AuthUser) -> impl IntoResponse {
    HtmlTemplate(UploadTemplate { current_user: Some(auth.0) })
}

// --- Auth Handlers ---

async fn admin_index(auth: AuthUser) -> impl IntoResponse {
    let user = auth.0;
    if !matches!(user.role, UserRole::Root | UserRole::Admin) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }
    HtmlTemplate(AdminIndexTemplate {
        current_user: Some(user),
    })
    .into_response()
}

// Musicians
async fn admin_musicians(auth: AuthUser, State(pool): State<Pool<Postgres>>) -> impl IntoResponse {
    if !matches!(auth.0.role, UserRole::Root | UserRole::Admin) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }
    let musicians = sqlx::query_as::<_, Musician>("SELECT * FROM musicians ORDER BY id")
        .fetch_all(&pool)
        .await
        .unwrap_or_default();
    HtmlTemplate(AdminMusiciansTemplate {
        musicians,
        current_user: Some(auth.0),
    })
    .into_response()
}

async fn admin_musician_new(auth: AuthUser) -> impl IntoResponse {
    if !matches!(auth.0.role, UserRole::Root | UserRole::Admin) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }
    HtmlTemplate(AdminMusicianEditTemplate {
        musician: Musician::default(),
        current_user: Some(auth.0),
    })
    .into_response()
}

async fn admin_musician_create(
    auth: AuthUser,
    State(pool): State<Pool<Postgres>>,
    Form(form): Form<CreateMusician>,
) -> impl IntoResponse {
    if !matches!(auth.0.role, UserRole::Root | UserRole::Admin) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }
    let _ =
        sqlx::query("INSERT INTO musicians (handle, given_name, family_name) VALUES ($1, $2, $3)")
            .bind(form.handle)
            .bind(form.given_name)
            .bind(form.family_name)
            .execute(&pool)
            .await
            .unwrap();
    Redirect::to("/admin/musicians").into_response()
}

async fn admin_musician_edit(
    auth: AuthUser,
    State(pool): State<Pool<Postgres>>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    if !matches!(auth.0.role, UserRole::Root | UserRole::Admin) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }
    let musician = sqlx::query_as::<_, Musician>("SELECT * FROM musicians WHERE id = $1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    HtmlTemplate(AdminMusicianEditTemplate {
        musician,
        current_user: Some(auth.0),
    })
    .into_response()
}

async fn admin_musician_update(
    auth: AuthUser,
    State(pool): State<Pool<Postgres>>,
    Path(id): Path<i32>,
    Form(form): Form<CreateMusician>,
) -> impl IntoResponse {
    if !matches!(auth.0.role, UserRole::Root | UserRole::Admin) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }
    let _ = sqlx::query(
        "UPDATE musicians SET handle = $1, given_name = $2, family_name = $3 WHERE id = $4",
    )
    .bind(form.handle)
    .bind(form.given_name)
    .bind(form.family_name)
    .bind(id)
    .execute(&pool)
    .await
    .unwrap();
    Redirect::to("/admin/musicians").into_response()
}

async fn admin_musician_delete(
    auth: AuthUser,
    State(pool): State<Pool<Postgres>>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    if !matches!(auth.0.role, UserRole::Root | UserRole::Admin) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }
    sqlx::query("DELETE FROM musicians WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    Redirect::to("/admin/musicians").into_response()
}

// Compositions
async fn admin_compositions(
    auth: AuthUser,
    State(pool): State<Pool<Postgres>>,
) -> impl IntoResponse {
    if !matches!(auth.0.role, UserRole::Root | UserRole::Admin) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }
    let compositions = sqlx::query_as::<_, Composition>("SELECT * FROM compositions ORDER BY id")
        .fetch_all(&pool)
        .await
        .unwrap_or_default();
    HtmlTemplate(AdminCompositionsTemplate {
        compositions,
        current_user: Some(auth.0),
    })
    .into_response()
}

async fn admin_composition_new(
    auth: AuthUser,
    State(pool): State<Pool<Postgres>>,
) -> impl IntoResponse {
    if !matches!(auth.0.role, UserRole::Root | UserRole::Admin) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }
    let musicians = sqlx::query_as::<_, Musician>("SELECT * FROM musicians ORDER BY family_name")
        .fetch_all(&pool)
        .await
        .unwrap_or_default();
    HtmlTemplate(AdminCompositionEditTemplate {
        composition: Composition::default(),
        movements: vec![],
        musicians,
        current_user: Some(auth.0),
    })
    .into_response()
}

async fn admin_composition_create(
    auth: AuthUser,
    State(pool): State<Pool<Postgres>>,
    Form(form): Form<CreateComposition>,
) -> impl IntoResponse {
    if !matches!(auth.0.role, UserRole::Root | UserRole::Admin) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }
    let _ = sqlx::query(
        "INSERT INTO compositions (slug, title, publish_date, composer_id) VALUES ($1, $2, $3, $4)",
    )
    .bind(form.slug)
    .bind(form.title)
    .bind(form.publish_date)
    .bind(form.composer_id)
    .execute(&pool)
    .await
    .unwrap();
    Redirect::to("/admin/compositions").into_response()
}

async fn admin_composition_edit(
    auth: AuthUser,
    State(pool): State<Pool<Postgres>>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    if !matches!(auth.0.role, UserRole::Root | UserRole::Admin) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }
    let composition = sqlx::query_as::<_, Composition>("SELECT * FROM compositions WHERE id = $1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let movements = sqlx::query_as::<_, Movement>(
        "SELECT * FROM movements WHERE composition_id = $1 ORDER BY index",
    )
    .bind(id)
    .fetch_all(&pool)
    .await
    .unwrap_or_default();
    let musicians = sqlx::query_as::<_, Musician>("SELECT * FROM musicians ORDER BY family_name")
        .fetch_all(&pool)
        .await
        .unwrap_or_default();
    HtmlTemplate(AdminCompositionEditTemplate {
        composition,
        movements,
        musicians,
        current_user: Some(auth.0),
    })
    .into_response()
}

async fn admin_composition_update(
    auth: AuthUser,
    State(pool): State<Pool<Postgres>>,
    Path(id): Path<i32>,
    Form(form): Form<CreateComposition>,
) -> impl IntoResponse {
    if !matches!(auth.0.role, UserRole::Root | UserRole::Admin) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }
    let _ = sqlx::query("UPDATE compositions SET slug = $1, title = $2, publish_date = $3, composer_id = $4 WHERE id = $5")
        .bind(form.slug)
        .bind(form.title)
        .bind(form.publish_date)
        .bind(form.composer_id)
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    Redirect::to(&format!("/admin/composition/{}", id)).into_response()
}

async fn admin_composition_delete(
    auth: AuthUser,
    State(pool): State<Pool<Postgres>>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    if !matches!(auth.0.role, UserRole::Root | UserRole::Admin) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }
    sqlx::query("DELETE FROM movements WHERE composition_id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM compositions WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    Redirect::to("/admin/compositions").into_response()
}

// Movements
async fn admin_movement_create(
    auth: AuthUser,
    State(pool): State<Pool<Postgres>>,
    Path(composition_id): Path<i32>,
    Form(form): Form<CreateMovement>,
) -> impl IntoResponse {
    if !matches!(auth.0.role, UserRole::Root | UserRole::Admin) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }
    let _ = sqlx::query(
        "INSERT INTO movements (slug, title, index, composition_id) VALUES ($1, $2, $3, $4)",
    )
    .bind(form.slug.clone())
    .bind(form.title.clone())
    .bind(form.index)
    .bind(composition_id)
    .execute(&pool)
    .await
    .unwrap();

    // Return updated list row - but simplest to just return an empty string and let client reload or return the new row.
    // The template expects `hx-target="#movements-list" hx-swap="beforeend"`.
    // So we should render a table row.
    // I need a template fragment for just the row.
    // But since I didn't create a separate fragment template, I'll cheat and return an HTML string or just redirect to reload the page (but HTMX expects partial).
    // Actually, to keep it simple, I'll redirect which HTMX will follow and swap the whole body if I used `hx-target="body"`, but I used `#movements-list`.
    // If I return the whole page, it might break.
    // I'll define a simple struct `MovementRowTemplate` inline or generic.
    // Or I can just format! a string.
    let movement = sqlx::query_as::<_, Movement>(
        "SELECT * FROM movements WHERE composition_id = $1 AND slug = $2",
    )
    .bind(composition_id)
    .bind(form.slug)
    .fetch_one(&pool)
    .await
    .unwrap();

    let html = format!(
        r#"
    <tr>
        <td class="px-4 py-2 whitespace-nowrap text-sm text-gray-500">{}</td>
        <td class="px-4 py-2 whitespace-nowrap text-sm text-gray-900">{}</td>
        <td class="px-4 py-2 whitespace-nowrap text-sm text-gray-500">{}</td>
        <td class="px-4 py-2 whitespace-nowrap text-right text-sm font-medium">
            <button hx-delete="/admin/movements/{}" hx-confirm="Delete movement?" hx-target="closest tr" class="text-red-600 hover:text-red-900">Delete</button>
        </td>
    </tr>
    "#,
        movement.index, movement.title, movement.slug, movement.id
    );

    axum::response::Html(html).into_response()
}

async fn admin_movement_delete(
    auth: AuthUser,
    State(pool): State<Pool<Postgres>>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    if !matches!(auth.0.role, UserRole::Root | UserRole::Admin) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }
    sqlx::query("DELETE FROM movements WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    axum::http::StatusCode::OK.into_response()
}

// Recordings
async fn admin_recordings(auth: AuthUser, State(pool): State<Pool<Postgres>>) -> impl IntoResponse {
    if !matches!(auth.0.role, UserRole::Root | UserRole::Admin) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }
    let recordings = sqlx::query_as::<_, Recording>("SELECT * FROM recordings ORDER BY id")
        .fetch_all(&pool)
        .await
        .unwrap_or_default();
    HtmlTemplate(AdminRecordingsTemplate {
        recordings,
        current_user: Some(auth.0),
    })
    .into_response()
}

async fn admin_recording_new(
    auth: AuthUser,
    State(pool): State<Pool<Postgres>>,
) -> impl IntoResponse {
    if !matches!(auth.0.role, UserRole::Root | UserRole::Admin) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }
    let musicians = sqlx::query_as::<_, Musician>("SELECT * FROM musicians ORDER BY family_name")
        .fetch_all(&pool)
        .await
        .unwrap_or_default();
    let compositions =
        sqlx::query_as::<_, Composition>("SELECT * FROM compositions ORDER BY title")
            .fetch_all(&pool)
            .await
            .unwrap_or_default();
    let movements =
        sqlx::query_as::<_, Movement>("SELECT * FROM movements ORDER BY composition_id, index")
            .fetch_all(&pool)
            .await
            .unwrap_or_default();
    HtmlTemplate(AdminRecordingEditTemplate {
        recording: Recording::default(),
        musicians,
        compositions,
        movements,
        current_user: Some(auth.0),
    })
    .into_response()
}

async fn admin_recording_create(
    auth: AuthUser,
    State(pool): State<Pool<Postgres>>,
    Form(form): Form<CreateRecording>,
) -> impl IntoResponse {
    if !matches!(auth.0.role, UserRole::Root) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }
    let _ = sqlx::query("INSERT INTO recordings (artist_id, composition_id, movement_id, content_hash, file_key) VALUES ($1, $2, $3, $4, $5)")
        .bind(form.artist_id)
        .bind(form.composition_id)
        .bind(form.movement_id)
        .bind(form.content_hash)
        .bind(form.file_key)
        .execute(&pool)
        .await
        .unwrap();
    Redirect::to("/admin/recordings").into_response()
}

async fn admin_recording_edit(
    auth: AuthUser,
    State(pool): State<Pool<Postgres>>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    if !matches!(auth.0.role, UserRole::Root | UserRole::Admin) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }
    let recording = sqlx::query_as::<_, Recording>("SELECT * FROM recordings WHERE id = $1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let musicians = sqlx::query_as::<_, Musician>("SELECT * FROM musicians ORDER BY family_name")
        .fetch_all(&pool)
        .await
        .unwrap_or_default();
    let compositions =
        sqlx::query_as::<_, Composition>("SELECT * FROM compositions ORDER BY title")
            .fetch_all(&pool)
            .await
            .unwrap_or_default();
    let movements =
        sqlx::query_as::<_, Movement>("SELECT * FROM movements ORDER BY composition_id, index")
            .fetch_all(&pool)
            .await
            .unwrap_or_default();
    HtmlTemplate(AdminRecordingEditTemplate {
        recording,
        musicians,
        compositions,
        movements,
        current_user: Some(auth.0),
    })
    .into_response()
}

async fn admin_recording_update(
    auth: AuthUser,
    State(pool): State<Pool<Postgres>>,
    Path(id): Path<i32>,
    Form(form): Form<CreateRecording>,
) -> impl IntoResponse {
    if !matches!(auth.0.role, UserRole::Root) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }
    let _ = sqlx::query("UPDATE recordings SET artist_id = $1, composition_id = $2, movement_id = $3, content_hash = $4, file_key = $5 WHERE id = $6")
        .bind(form.artist_id)
        .bind(form.composition_id)
        .bind(form.movement_id)
        .bind(form.content_hash)
        .bind(form.file_key)
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    Redirect::to("/admin/recordings").into_response()
}

async fn admin_recording_delete(
    auth: AuthUser,
    State(pool): State<Pool<Postgres>>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    if !matches!(auth.0.role, UserRole::Root) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }
    sqlx::query("DELETE FROM recordings WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    Redirect::to("/admin/recordings").into_response()
}
