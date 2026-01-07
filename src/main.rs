use argon2::{
    Argon2, PasswordHash,
    password_hash::{PasswordHasher, PasswordVerifier, SaltString},
};
use axum::{
    Form, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, FromRequestParts, Multipart, Path, State},
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
use uuid::Uuid;

mod db;
mod storage;
mod view;
use db::*;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
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

struct HtmxRequest {
    is_hx_boosted: bool,
}

impl<S> FromRequestParts<S> for HtmxRequest
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let is_hx_boosted = parts.headers.get("hx-boosted").is_some_and(|v| v == "true");
        Ok(HtmxRequest { is_hx_boosted })
    }
}

// --- Auth Templates ---

use axum::extract::Query;

#[derive(Deserialize)]
struct LoginPayload {
    email: String,
    password: String,
}

#[derive(Deserialize)]
struct PaginationParams {
    page: Option<i64>,
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
        import_cancel_token: Arc::new(Mutex::new(None)),
    };

    let app = Router::new()
        .route("/", get(index))
        .route("/settings", get(settings).post(settings_post))
        .route(
            "/upload",
            get(upload)
                .post(upload_post)
                .layer(DefaultBodyLimit::max(1024 * 1024 * 50)),
        )
        .route("/upload/select-composition/{id}", get(select_composition))
        .route("/upload/reset-composition", get(reset_composition))
        .route("/audio/{key}", get(serve_audio))
        .route("/player/{id}", get(get_player))
        .route("/login", get(login_form).post(login_post))
        .route("/logout", post(logout))
        .route("/search/compositions", get(search_compositions))
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
        .route("/admin/import", get(admin_import).post(admin_import_start))
        .route("/admin/import/cancel", post(admin_import_cancel))
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
async fn login_form(auth: OptionalAuthUser) -> impl IntoResponse {
    HtmlTemplate(LoginTemplate {
        current_user: auth.0,
        error: None,
    })
}

async fn login_post(
    auth: OptionalAuthUser,
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
            current_user: auth.0,
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

async fn index(
    auth: OptionalAuthUser,
    htmx: HtmxRequest,
    State(pool): State<Pool<Postgres>>,
) -> Response {
    let recordings = sqlx::query_as::<_, RecordingFeedItem>(
        r#"
        SELECT 
            r.id,
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
        "#,
    )
    .fetch_all(&pool)
    .await
    .unwrap_or_default();

    if htmx.is_hx_boosted {
        HtmlTemplate(IndexContentTemplate {
            current_user: auth.0,
            recordings,
        })
        .into_response()
    } else {
        HtmlTemplate(IndexTemplate {
            current_user: auth.0,
            recordings,
        })
        .into_response()
    }
}

async fn settings(
    auth: AuthUser,
    htmx: HtmxRequest,
    State(pool): State<Pool<Postgres>>,
) -> Response {
    let current_user = auth.0;
    let musician = sqlx::query_as::<_, Musician>("SELECT * FROM musicians WHERE id = $1")
        .bind(current_user.musician_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    if htmx.is_hx_boosted {
        HtmlTemplate(SettingsContentTemplate {
            current_user: Some(current_user),
            musician,
        })
        .into_response()
    } else {
        HtmlTemplate(SettingsTemplate {
            current_user: Some(current_user),
            musician,
        })
        .into_response()
    }
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

async fn upload(
    auth: AuthUser,
    _htmx: HtmxRequest,
    State(pool): State<Pool<Postgres>>,
) -> impl IntoResponse {
    let compositions =
        sqlx::query_as::<_, Composition>("SELECT * FROM compositions ORDER BY title")
            .fetch_all(&pool)
            .await
            .unwrap_or_default();

    HtmlTemplate(UploadTemplate {
        current_user: Some(auth.0),
        error: None,
        compositions,
    })
}

async fn upload_post(
    auth: AuthUser,
    _htmx: HtmxRequest,
    State(pool): State<Pool<Postgres>>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let mut file_data: Option<Bytes> = None;
    let mut composition_id = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or_default().to_string();
        tracing::info!("Received field: {}", name);
        if name == "file" {
            match field.bytes().await {
                Ok(bytes) => {
                    tracing::info!("Received file bytes: {} bytes", bytes.len());
                    file_data = Some(bytes);
                }
                Err(e) => {
                    tracing::error!("Failed to read file bytes: {}", e);
                }
            }
        } else if name == "composition_id" {
            if let Ok(txt) = field.text().await {
                tracing::info!("Received composition_id: {}", txt);
                composition_id = txt.parse::<i32>().ok();
            }
        }
    }

    let compositions =
        sqlx::query_as::<_, Composition>("SELECT * FROM compositions ORDER BY title")
            .fetch_all(&pool)
            .await
            .unwrap_or_default();

    let data = match file_data {
        Some(d) => d,
        None => {
            return HtmlTemplate(UploadTemplate {
                current_user: Some(auth.0.clone()),
                error: Some("No file uploaded.".to_string()),
                compositions,
            })
            .into_response();
        }
    };

    let comp_id = match composition_id {
        Some(id) => id,
        None => {
            return HtmlTemplate(UploadTemplate {
                current_user: Some(auth.0.clone()),
                error: Some("No composition selected.".to_string()),
                compositions,
            })
            .into_response();
        }
    };

    let is_audio = if let Some(kind) = infer::get(&data) {
        kind.mime_type().starts_with("audio/")
    } else {
        false
    };

    if !is_audio {
        return HtmlTemplate(UploadTemplate {
            current_user: Some(auth.0),
            error: Some("Uploaded file is not a valid audio file.".to_string()),
            compositions,
        })
        .into_response();
    }

    let file_key = Uuid::new_v4().to_string();
    let path = format!("uploads/recordings/{}", file_key);
    if let Some(parent) = std::path::Path::new(&path).parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    if tokio::fs::write(&path, &data).await.is_err() {
        return HtmlTemplate(UploadTemplate {
            current_user: Some(auth.0),
            error: Some("Failed to save file.".to_string()),
            compositions,
        })
        .into_response();
    }

    let _ = sqlx::query(
        "INSERT INTO recordings (artist_id, composition_id, content_hash, file_key) VALUES ($1, $2, $3, $4)"
    )
    .bind(auth.0.musician_id)
    .bind(comp_id)
    .bind("hash_placeholder")
    .bind(file_key)
    .execute(&pool)
    .await
    .unwrap();

    Redirect::to("/").into_response()
}

async fn serve_audio(Path(key): Path<String>) -> impl IntoResponse {
    let path = format!("uploads/recordings/{}", key);
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            if let Some(kind) = infer::get(&bytes) {
                if kind.mime_type().starts_with("audio/") {
                    return (
                        [(axum::http::header::CONTENT_TYPE, kind.mime_type())],
                        bytes,
                    )
                        .into_response();
                }
            }
            (StatusCode::BAD_REQUEST, "File is not an audio file").into_response()
        }
        Err(_) => (StatusCode::NOT_FOUND, "File not found").into_response(),
    }
}

async fn get_player(Path(id): Path<i32>, State(pool): State<Pool<Postgres>>) -> impl IntoResponse {
    let recording = sqlx::query_as::<_, RecordingFeedItem>(
        r#"
        SELECT 
            r.id,
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
        WHERE r.id = $1
        "#,
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();

    HtmlTemplate(PlayerTemplate { recording })
}

#[derive(Deserialize)]

struct SearchParams {
    q: String,
}

#[derive(sqlx::FromRow)]

struct SearchResult {
    composition_id: i32,
    composition_title: String,
    movement_id: Option<i32>,
    movement_index: Option<i32>,
    movement_title: Option<String>,
    composer_name: String,
}

async fn search_compositions(
    State(pool): State<Pool<Postgres>>,

    Query(params): Query<SearchParams>,
) -> impl IntoResponse {
    if params.q.trim().is_empty() {
        return axum::response::Html("".to_string()).into_response();
    }

    let search_pattern = format!("%{}%", params.q);

    let results = sqlx::query_as::<_, SearchResult>(
        r#"
        SELECT 
            c.id as composition_id,
            c.title as composition_title,
            m.id as movement_id,
            m.index as movement_index,
            m.title as movement_title,
            mus.given_name || ' ' || mus.family_name as composer_name
        FROM compositions c
        JOIN musicians mus ON c.composer_id = mus.id
        LEFT JOIN movements m ON c.id = m.composition_id
        WHERE 
          c.title ILIKE $1 
         OR
          m.title ILIKE $1
         OR
          mus.handle ILIKE  $1
         OR
          mus.given_name ILIKE  $1
         OR
          mus.family_name ILIKE $1
        ORDER BY c.title, m.index
        LIMIT 50
        "#,
    )
    .bind(search_pattern)
    .fetch_all(&pool)
    .await
    .unwrap_or_default();

    let mut html = String::new();

    for res in results {
        let display_text = if let Some(mov) = res.movement_title
            && let Some(mov_i) = res.movement_index
        {
            format!(
                "{}: {} {}. {}",
                res.composer_name,
                res.composition_title,
                mov_i + 1,
                mov
            )
        } else {
            format!("{}: {}", res.composer_name, res.composition_title)
        };

        // let safe_title = display_text.replace("\"", "&quot;");

        html.push_str(&format!(
            r##"<div class="cursor-pointer hover:bg-indigo-50 p-2 text-sm text-gray-700 border-b last:border-b-0" 
                    hx-get="/upload/select-composition/{}"
                    hx-target="#composition-picker"
                    hx-swap="outerHTML">
                {}
            </div>"##,
            res.composition_id, display_text
        ));
    }

    axum::response::Html(html).into_response()
}

async fn select_composition(
    Path(id): Path<i32>,
    State(pool): State<Pool<Postgres>>,
) -> impl IntoResponse {
    let result = sqlx::query_as::<_, SearchResult>(
        r#"
        SELECT 
            c.id as composition_id,
            c.title as composition_title,
            m.id as movement_id,
            m.index as movement_index,
            m.title as movement_title,
            mus.given_name || ' ' || mus.family_name as composer_name
        FROM compositions c
        JOIN musicians mus ON c.composer_id = mus.id
        LEFT JOIN movements m ON c.id = m.composition_id
        WHERE c.id = $1
        LIMIT 1
        "#,
    )
    .bind(id)
    .fetch_optional(&pool)
    .await
    .unwrap_or(None);

    if let Some(res) = result {
        let display_text = format!("{}: {}", res.composer_name, res.composition_title);

        let html = format!(
            r##"<div id="composition-picker" class="relative">
                <label class="block text-sm font-medium text-gray-700">Composition</label>
                <input type="hidden" name="composition_id" value="{}" required>
                <div class="mt-1 flex items-center justify-between p-2 border border-gray-300 rounded-md bg-gray-50">
                    <span class="text-sm text-gray-900 font-medium">{}</span>
                    <button type="button" 
                            hx-get="/upload/reset-composition" 
                            hx-target="#composition-picker" 
                            hx-swap="outerHTML" 
                            class="text-gray-400 hover:text-gray-500">
                        <svg class="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
                        </svg>
                    </button>
                </div>
            </div>"##,
            res.composition_id, display_text
        );
        axum::response::Html(html).into_response()
    } else {
        // Fallback if not found (shouldn't happen often)
        reset_composition().await.into_response()
    }
}

async fn reset_composition() -> impl IntoResponse {
    HtmlTemplate(CompositionPickerTemplate)
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
async fn admin_musicians(
    auth: AuthUser,
    State(pool): State<Pool<Postgres>>,
    Query(params): Query<PaginationParams>,
) -> impl IntoResponse {
    if !matches!(auth.0.role, UserRole::Root | UserRole::Admin) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }

    let page = params.page.unwrap_or(1).max(1);
    let limit = 25;
    let offset = (page - 1) * limit;

    let total_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM musicians")
        .fetch_one(&pool)
        .await
        .unwrap_or(0);

    let total_pages = (total_count as f64 / limit as f64).ceil() as i64;

    let musicians =
        sqlx::query_as::<_, Musician>("SELECT * FROM musicians ORDER BY id LIMIT $1 OFFSET $2")
            .bind(limit)
            .bind(offset)
            .fetch_all(&pool)
            .await
            .unwrap_or_default();

    HtmlTemplate(AdminMusiciansTemplate {
        musicians,
        current_user: Some(auth.0),
        page,
        total_pages,
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
    Query(params): Query<PaginationParams>,
) -> impl IntoResponse {
    if !matches!(auth.0.role, UserRole::Root | UserRole::Admin) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }

    let page = params.page.unwrap_or(1).max(1);
    let limit = 25;
    let offset = (page - 1) * limit;

    let total_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM compositions")
        .fetch_one(&pool)
        .await
        .unwrap_or(0);

    let total_pages = (total_count as f64 / limit as f64).ceil() as i64;

    let compositions = sqlx::query_as::<_, Composition>(
        "SELECT * FROM compositions ORDER BY id LIMIT $1 OFFSET $2",
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(&pool)
    .await
    .unwrap_or_default();

    HtmlTemplate(AdminCompositionsTemplate {
        compositions,
        current_user: Some(auth.0),
        page,
        total_pages,
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
async fn admin_recordings(
    auth: AuthUser,
    State(pool): State<Pool<Postgres>>,
    Query(params): Query<PaginationParams>,
) -> impl IntoResponse {
    if !matches!(auth.0.role, UserRole::Root | UserRole::Admin) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }

    let page = params.page.unwrap_or(1).max(1);
    let limit = 25;
    let offset = (page - 1) * limit;

    let total_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM recordings")
        .fetch_one(&pool)
        .await
        .unwrap_or(0);

    let total_pages = (total_count as f64 / limit as f64).ceil() as i64;

    let recordings =
        sqlx::query_as::<_, Recording>("SELECT * FROM recordings ORDER BY id LIMIT $1 OFFSET $2")
            .bind(limit)
            .bind(offset)
            .fetch_all(&pool)
            .await
            .unwrap_or_default();

    HtmlTemplate(AdminRecordingsTemplate {
        recordings,
        current_user: Some(auth.0),
        page,
        total_pages,
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

// --- Import Logic ---

#[derive(Deserialize)]
struct ImslpPeopleResponse {
    #[serde(flatten)]
    items: std::collections::HashMap<String, ImslpPerson>,
    // metadata: serde_json::Value,
}

#[derive(Deserialize)]
struct ImslpPerson {
    id: String, // "Bach, Johann Sebastian"
                // type: String,
                // intvals: serde_json::Value,
}

#[derive(Deserialize)]
struct ImslpWorksResponse {
    #[serde(flatten)]
    items: std::collections::HashMap<String, ImslpWork>,
}

#[derive(Deserialize)]
struct ImslpWork {
    // id: String,
    intvals: ImslpWorkIntvals,
}

#[derive(Deserialize)]
struct ImslpWorkIntvals {
    composer: String,
    worktitle: String,
}

fn slugify(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn format_name(name: &str) -> (String, String) {
    let name = name.strip_prefix("Category:").unwrap_or(name);
    let parts: Vec<&str> = name.split(',').map(|s| s.trim()).collect();
    if parts.len() >= 2 {
        // "Bach, Johann Sebastian" -> given: "Johann Sebastian", family: "Bach"
        (parts[1..].join(" "), parts[0].to_string())
    } else {
        ("".to_string(), name.to_string())
    }
}

fn name_to_full_handle(name: &str) -> String {
    let (given, family) = format_name(name);
    if given.is_empty() {
        slugify(&family)
    } else {
        slugify(&format!("{} {}", given, family))
    }
}

async fn admin_import(auth: AuthUser, State(pool): State<Pool<Postgres>>) -> impl IntoResponse {
    if !auth.0.is_privileged() {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }

    let job = sqlx::query_as::<_, ImportJob>(
        "SELECT * FROM import_jobs ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_optional(&pool)
    .await
    .unwrap_or(None);

    HtmlTemplate(AdminImportTemplate {
        job,
        current_user: Some(auth.0),
    })
    .into_response()
}

#[derive(Deserialize)]
struct StartImportParams {
    #[serde(default)]
    replace_existing: bool,
}

async fn admin_import_start(
    auth: AuthUser,
    State(pool): State<Pool<Postgres>>,
    State(cancel_token_mutex): State<Arc<Mutex<Option<CancellationToken>>>>,
    Form(params): Form<StartImportParams>,
) -> impl IntoResponse {
    if !auth.0.is_privileged() {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }

    let mut token_lock = cancel_token_mutex.lock().await;
    if token_lock.is_some() {
        // Job already running
        return Redirect::to("/admin/import").into_response();
    }

    let job = sqlx::query_as::<_, ImportJob>(
        "INSERT INTO import_jobs (status) VALUES ('processing') RETURNING *",
    )
    .bind(ImportJobStatus::Processing)
    .fetch_one(&pool)
    .await
    .unwrap();

    let cancel_token = CancellationToken::new();
    *token_lock = Some(cancel_token.clone());

    let pool_clone = pool.clone();
    let job_id = job.id;
    let cancel_token_mutex_clone = cancel_token_mutex.clone();
    let replace_existing = params.replace_existing;

    tokio::spawn(async move {
        let result = run_import(pool_clone, cancel_token, job_id, replace_existing).await;

        let mut lock = cancel_token_mutex_clone.lock().await;
        *lock = None;

        if let Err(e) = result {
            tracing::error!("Import job {} failed: {:?}", job_id, e);
        }
    });

    Redirect::to("/admin/import").into_response()
}

async fn admin_import_cancel(
    auth: AuthUser,
    State(cancel_token_mutex): State<Arc<Mutex<Option<CancellationToken>>>>,
) -> impl IntoResponse {
    if !auth.0.is_privileged() {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }

    let mut token_lock = cancel_token_mutex.lock().await;
    if let Some(token) = token_lock.take() {
        token.cancel();
    }

    Redirect::to("/admin/import").into_response()
}

async fn run_import(
    pool: Pool<Postgres>,
    cancel_token: CancellationToken,
    job_id: i32,
    replace_existing: bool,
) -> anyhow::Result<()> {
    let mut success_count = 0;
    let mut skip_count = 0;
    let mut failure_count = 0;

    macro_rules! update_job {
        ($status:expr) => {
            let _ = sqlx::query("UPDATE import_jobs SET status = $1, success_count = $2, skip_count = $3, failure_count = $4, updated_at = NOW() WHERE id = $5")
                .bind($status)
                .bind(success_count)
                .bind(skip_count)
                .bind(failure_count)
                .bind(job_id)
                .execute(&pool)
                .await;
        };
    }

    // 1. Process People
    let people_dir = "imslp/people";
    if let Ok(mut entries) = tokio::fs::read_dir(people_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            if cancel_token.is_cancelled() {
                update_job!(ImportJobStatus::Cancelled);
                return Ok(());
            }

            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                let content = tokio::fs::read_to_string(&path).await?;
                let data: std::collections::HashMap<String, serde_json::Value> =
                    serde_json::from_str(&content)?;

                for (key, value) in data {
                    if key == "metadata" || !value.is_object() {
                        continue;
                    }
                    if cancel_token.is_cancelled() {
                        update_job!(ImportJobStatus::Cancelled);
                        return Ok(());
                    }

                    if let Some(name) = value.get("id").and_then(|v| v.as_str()) {
                        let handle = name_to_full_handle(name);
                        let (given_name, family_name) = format_name(name);

                        let exists = sqlx::query_scalar::<_, bool>(
                            "SELECT EXISTS(SELECT 1 FROM musicians WHERE handle = $1)",
                        )
                        .bind(&handle)
                        .fetch_one(&pool)
                        .await?;

                        if exists && !replace_existing {
                            skip_count += 1;
                        } else {
                            // If exists && replace_existing, we update (UPSERT or separate UPDATE)
                            // Or just DELETE and INSERT? ID preservation might be important for foreign keys.
                            // Better to use ON CONFLICT DO UPDATE.
                            let res = sqlx::query(
                                r#"
                                INSERT INTO musicians (handle, given_name, family_name) 
                                VALUES ($1, $2, $3)
                                ON CONFLICT (handle) DO UPDATE SET
                                    given_name = EXCLUDED.given_name,
                                    family_name = EXCLUDED.family_name
                                "#,
                            )
                            .bind(handle)
                            .bind(given_name)
                            .bind(family_name)
                            .execute(&pool)
                            .await;

                            match res {
                                Ok(_) => success_count += 1,
                                Err(_) => failure_count += 1,
                            }
                        }
                    }
                    // Update every 10 items to show progress
                    if (success_count + skip_count + failure_count) % 10 == 0 {
                        update_job!(ImportJobStatus::Processing);
                    }
                }
            }
        }
    }

    // 2. Process Works
    let works_dir = "imslp/works";
    if let Ok(mut entries) = tokio::fs::read_dir(works_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            if cancel_token.is_cancelled() {
                update_job!(ImportJobStatus::Cancelled);
                return Ok(());
            }

            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                let content = tokio::fs::read_to_string(&path).await?;
                let data: std::collections::HashMap<String, serde_json::Value> =
                    serde_json::from_str(&content)?;

                for (key, value) in data {
                    if key == "metadata" || !value.is_object() {
                        continue;
                    }
                    if cancel_token.is_cancelled() {
                        update_job!(ImportJobStatus::Cancelled);
                        return Ok(());
                    }

                    let intvals = value.get("intvals");
                    let composer_name = intvals
                        .and_then(|i| i.get("composer"))
                        .and_then(|v| v.as_str());
                    let work_title = intvals
                        .and_then(|i| i.get("worktitle"))
                        .and_then(|v| v.as_str());

                    if let (Some(c_name), Some(w_title)) = (composer_name, work_title) {
                        let composer_handle = name_to_full_handle(c_name);
                        let (given_c, family_c) = format_name(c_name);
                        let full_title_for_slug = format!("{} {} {}", given_c, family_c, w_title);
                        let slug = slugify(&full_title_for_slug);

                        let exists = sqlx::query_scalar::<_, bool>(
                            "SELECT EXISTS(SELECT 1 FROM compositions WHERE slug = $1)",
                        )
                        .bind(&slug)
                        .fetch_one(&pool)
                        .await?;

                        if exists && !replace_existing {
                            skip_count += 1;
                        } else {
                            // Find or create musician (might not have been in people list but is in works list)
                            let composer_id = sqlx::query_scalar::<_, i32>(
                                "SELECT id FROM musicians WHERE handle = $1",
                            )
                            .bind(&composer_handle)
                            .fetch_optional(&pool)
                            .await?;

                            let composer_id = match composer_id {
                                Some(id) => id,
                                None => {
                                    sqlx::query_scalar::<_, i32>("INSERT INTO musicians (handle, given_name, family_name) VALUES ($1, $2, $3) RETURNING id")
                                        .bind(composer_handle)
                                        .bind(given_c)
                                        .bind(family_c)
                                        .fetch_one(&pool)
                                        .await?
                                }
                            };

                            let res = sqlx::query(
                                r#"
                                INSERT INTO compositions (slug, title, composer_id) 
                                VALUES ($1, $2, $3)
                                ON CONFLICT (slug) DO UPDATE SET
                                    title = EXCLUDED.title,
                                    composer_id = EXCLUDED.composer_id
                                "#,
                            )
                            .bind(slug)
                            .bind(w_title)
                            .bind(composer_id)
                            .execute(&pool)
                            .await;

                            match res {
                                Ok(_) => success_count += 1,
                                Err(_) => failure_count += 1,
                            }
                        }
                    }

                    if (success_count + skip_count + failure_count) % 10 == 0 {
                        update_job!(ImportJobStatus::Processing);
                    }
                }
            }
        }
    }

    update_job!(ImportJobStatus::Completed);
    Ok(())
}
