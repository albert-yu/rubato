use argon2::{
    Argon2, PasswordHash,
    password_hash::{PasswordHasher, PasswordVerifier, SaltString},
};
use askama::Template;
use axum::{
    Form, Router,
    extract::{FromRequestParts, Path, State},
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Redirect, Response},
    routing::{delete, get, patch, post},
};
use axum_extra::extract::cookie::{Cookie, Key, SameSite, SignedCookieJar};
use rand::rngs::OsRng;
use serde::Deserialize;
use sqlx::{Pool, Postgres, migrate::MigrateDatabase, postgres::PgPoolOptions};
use std::io::Write;
use std::net::SocketAddr;
use tower_http::services::ServeDir;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

// --- Todo Structs ---
#[derive(sqlx::FromRow, serde::Serialize)]
struct Todo {
    id: i32,
    task: String,
    completed: bool,
}

#[derive(serde::Deserialize)]
struct CreateTodo {
    task: String,
}

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate;

#[derive(Template)]
#[template(path = "todo_item.html")]
struct TodoItemTemplate {
    todo: Todo,
}

// --- Music Structs ---

#[derive(sqlx::Type, serde::Serialize, Clone, Debug, PartialEq)]
#[sqlx(type_name = "user_role", rename_all = "lowercase")]
enum UserRole {
    Root,
    Admin,
    User,
}

#[derive(sqlx::FromRow, serde::Serialize, Clone, Debug)]
struct User {
    id: i32,
    email: String,
    password_hash: String,
    salt: String,
    musician_id: i32,
    role: UserRole,
}

struct AuthUser(User);

#[derive(Clone)]
struct AppState {
    pool: Pool<Postgres>,
    key: Key,
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

use axum::extract::FromRef;

impl<S> FromRequestParts<S> for AuthUser
where
    Pool<Postgres>: FromRef<S>,
    Key: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let pool = Pool::<Postgres>::from_ref(state);
        let jar: SignedCookieJar<Key> = SignedCookieJar::from_request_parts(parts, state)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Cookie error").into_response())?;

        if let Some(user_id) = jar.get("user_id") {
            if let Ok(id) = user_id.value().parse::<i32>() {
                if let Ok(user) = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
                    .bind(id)
                    .fetch_one(&pool)
                    .await
                {
                    return Ok(AuthUser(user));
                }
            }
        }

        Err((StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response())
    }
}

#[derive(sqlx::FromRow, serde::Serialize, Clone, Default)]
struct Musician {
    id: i32,
    handle: String,
    given_name: String,
    family_name: String,
}

#[derive(serde::Deserialize)]
struct CreateMusician {
    handle: String,
    given_name: String,
    family_name: String,
}

#[derive(sqlx::FromRow, serde::Serialize, Clone, Default)]
struct Composition {
    id: i32,
    slug: String,
    title: String,
    publish_date: Option<chrono::NaiveDate>,
    composer_id: i32,
}

#[derive(serde::Deserialize)]
struct CreateComposition {
    slug: String,
    title: String,
    #[serde(default)]
    #[serde(deserialize_with = "empty_string_as_none")]
    publish_date: Option<chrono::NaiveDate>,
    composer_id: i32,
}

#[derive(sqlx::FromRow, serde::Serialize, Clone, Default)]
struct Movement {
    id: i32,
    slug: String,
    title: String,
    index: i32,
    composition_id: i32,
}

#[derive(serde::Deserialize)]
struct CreateMovement {
    slug: String,
    title: String,
    index: i32,
}

#[derive(sqlx::FromRow, serde::Serialize, Clone, Default)]
struct Recording {
    id: i32,
    artist_id: i32,
    composition_id: i32,
    movement_id: Option<i32>,
    content_hash: String,
    file_key: String,
}

#[derive(serde::Deserialize)]
struct CreateRecording {
    artist_id: i32,
    composition_id: i32,
    #[serde(default)]
    #[serde(deserialize_with = "empty_string_as_none_i32")]
    movement_id: Option<i32>,
    content_hash: String,
    file_key: String,
}

// Helpers for deserialization
fn empty_string_as_none<'de, D, T>(de: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    let opt = Option::<String>::deserialize(de)?;
    match opt.as_deref() {
        None | Some("") => Ok(None),
        Some(s) => T::deserialize(serde::de::value::StrDeserializer::new(s)).map(Some),
    }
}

fn empty_string_as_none_i32<'de, D>(de: D) -> Result<Option<i32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt = Option::<String>::deserialize(de)?;
    match opt.as_deref() {
        None | Some("") => Ok(None),
        Some(s) => s.parse::<i32>().map(Some).map_err(serde::de::Error::custom),
    }
}

// --- Admin Templates ---

#[derive(Template)]
#[template(path = "admin/index.html")]
struct AdminIndexTemplate;

#[derive(Template)]
#[template(path = "admin/musicians.html")]
struct AdminMusiciansTemplate {
    musicians: Vec<Musician>,
}

#[derive(Template)]
#[template(path = "admin/musician_edit.html")]
struct AdminMusicianEditTemplate {
    musician: Musician,
}

#[derive(Template)]
#[template(path = "admin/compositions.html")]
struct AdminCompositionsTemplate {
    compositions: Vec<Composition>,
}

#[derive(Template)]
#[template(path = "admin/composition_edit.html")]
struct AdminCompositionEditTemplate {
    composition: Composition,
    movements: Vec<Movement>,
    musicians: Vec<Musician>,
}

#[derive(Template)]
#[template(path = "admin/recordings.html")]
struct AdminRecordingsTemplate {
    recordings: Vec<Recording>,
}

#[derive(Template)]
#[template(path = "admin/recording_edit.html")]
struct AdminRecordingEditTemplate {
    recording: Recording,
    musicians: Vec<Musician>,
    compositions: Vec<Composition>,
    movements: Vec<Movement>,
}

// --- Auth Templates ---
#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate {
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "404.html")]
struct NotFoundTemplate;

#[derive(Deserialize)]
struct LoginPayload {
    email: String,
    password: String,
}

// --- Common ---

struct HtmlTemplate<T>(T);

impl<T> IntoResponse for HtmlTemplate<T>
where
    T: Template,
{
    fn into_response(self) -> axum::response::Response {
        match self.0.render() {
            Ok(html) => axum::response::Html(html).into_response(),
            Err(err) => (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
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
    let app_state = AppState { pool, key };

    let app = Router::new()
        .route("/", get(index))
        .route("/login", get(login_form).post(login_post))
        .route("/logout", post(logout))
        .route("/todos", post(add_todo))
        .route("/todos/{id}", patch(toggle_todo).delete(delete_todo))
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
    jar: SignedCookieJar,
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
            let mut cookie = Cookie::new("user_id", user.id.to_string());
            cookie.set_http_only(true);
            cookie.set_same_site(SameSite::Lax);
            cookie.set_path("/");
            return (jar.add(cookie), Redirect::to("/admin")).into_response();
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

async fn logout(jar: SignedCookieJar) -> impl IntoResponse {
    (jar.remove(Cookie::from("user_id")), Redirect::to("/login"))
}

// --- Todo Handlers ---
async fn index() -> impl IntoResponse {
    HtmlTemplate(IndexTemplate)
}

async fn add_todo(
    State(pool): State<Pool<Postgres>>,
    Form(form): Form<CreateTodo>,
) -> impl IntoResponse {
    let todo = sqlx::query_as::<_, Todo>(
        "INSERT INTO todos (task) VALUES ($1) RETURNING id, task, completed",
    )
    .bind(form.task)
    .fetch_one(&pool)
    .await
    .unwrap();

    HtmlTemplate(TodoItemTemplate { todo })
}

async fn toggle_todo(State(pool): State<Pool<Postgres>>, Path(id): Path<i32>) -> impl IntoResponse {
    let todo = sqlx::query_as::<_, Todo>(
        "UPDATE todos SET completed = NOT completed WHERE id = $1 RETURNING id, task, completed",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();

    HtmlTemplate(TodoItemTemplate { todo })
}

async fn delete_todo(State(pool): State<Pool<Postgres>>, Path(id): Path<i32>) -> impl IntoResponse {
    sqlx::query("DELETE FROM todos WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    axum::http::StatusCode::OK
}

// --- Admin Handlers ---

async fn admin_index(auth: AuthUser) -> impl IntoResponse {
    let user = auth.0;
    if !matches!(user.role, UserRole::Root | UserRole::Admin) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }
    HtmlTemplate(AdminIndexTemplate).into_response()
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
    HtmlTemplate(AdminMusiciansTemplate { musicians }).into_response()
}

async fn admin_musician_new(auth: AuthUser) -> impl IntoResponse {
    if !matches!(auth.0.role, UserRole::Root | UserRole::Admin) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }
    HtmlTemplate(AdminMusicianEditTemplate {
        musician: Musician::default(),
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
    HtmlTemplate(AdminMusicianEditTemplate { musician }).into_response()
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
    HtmlTemplate(AdminCompositionsTemplate { compositions }).into_response()
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
    HtmlTemplate(AdminRecordingsTemplate { recordings }).into_response()
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
