use axum::{
    Form,
    extract::{Multipart, Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect},
};
use serde::Deserialize;
use sqlx::{Pool, Postgres};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::db::*;
use crate::extractors::AuthUser;
use crate::view::*;

#[derive(Deserialize)]
pub struct PaginationParams {
    pub page: Option<i64>,
    pub q: Option<String>,
}

pub async fn admin_index(auth: AuthUser) -> impl IntoResponse {
    let user = auth.0;
    if !matches!(user.role, UserRole::Root | UserRole::Admin) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }
    HtmlTemplate(AdminIndexTemplate {
        current_user: Some(user),
        active_nav: "dashboard",
    })
    .into_response()
}

// Musicians
pub async fn admin_musicians(
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
    let q = params.q.filter(|s| !s.trim().is_empty());

    let (total_count, musicians) = if let Some(ref query) = q {
        let search_pattern = format!("%{}%", query);
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM musicians WHERE given_name ILIKE $1 OR family_name ILIKE $1",
        )
        .bind(&search_pattern)
        .fetch_one(&pool)
        .await
        .unwrap_or(0);

        let mus = sqlx::query_as::<_, Musician>(
            "SELECT * FROM musicians WHERE given_name ILIKE $1 OR family_name ILIKE $1 ORDER BY id LIMIT $2 OFFSET $3",
        )
        .bind(&search_pattern)
        .bind(limit)
        .bind(offset)
        .fetch_all(&pool)
        .await
        .unwrap_or_default();
        (count, mus)
    } else {
        let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM musicians")
            .fetch_one(&pool)
            .await
            .unwrap_or(0);

        let mus =
            sqlx::query_as::<_, Musician>("SELECT * FROM musicians ORDER BY id LIMIT $1 OFFSET $2")
                .bind(limit)
                .bind(offset)
                .fetch_all(&pool)
                .await
                .unwrap_or_default();
        (count, mus)
    };

    let total_pages = (total_count as f64 / limit as f64).ceil() as i64;

    HtmlTemplate(AdminMusiciansTemplate {
        musicians,
        current_user: Some(auth.0),
        page,
        total_pages,
        q,
        active_nav: "musicians",
    })
    .into_response()
}

pub async fn admin_musician_new(auth: AuthUser) -> impl IntoResponse {
    if !matches!(auth.0.role, UserRole::Root | UserRole::Admin) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }
    HtmlTemplate(AdminMusicianEditTemplate {
        musician: Musician::default(),
        current_user: Some(auth.0),
        active_nav: "musicians",
    })
    .into_response()
}

pub async fn admin_musician_create(
    auth: AuthUser,
    State(pool): State<Pool<Postgres>>,
    Form(form): Form<CreateMusician>,
) -> impl IntoResponse {
    if !matches!(auth.0.role, UserRole::Root | UserRole::Admin) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }
    let _ =
        sqlx::query("INSERT INTO musicians (handle, given_name, family_name, birth_date, death_date) VALUES ($1, $2, $3, $4, $5)")
            .bind(form.handle)
            .bind(form.given_name)
            .bind(form.family_name)
            .bind(form.birth_date)
            .bind(form.death_date)
            .execute(&pool)
            .await
            .unwrap();
    Redirect::to("/admin/musicians").into_response()
}

pub async fn admin_musician_edit(
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
        active_nav: "musicians",
    })
    .into_response()
}

pub async fn admin_musician_update(
    auth: AuthUser,
    State(pool): State<Pool<Postgres>>,
    Path(id): Path<i32>,
    Form(form): Form<CreateMusician>,
) -> impl IntoResponse {
    if !matches!(auth.0.role, UserRole::Root | UserRole::Admin) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }
    let _ = sqlx::query(
        "UPDATE musicians SET handle = $1, given_name = $2, family_name = $3, birth_date = $4, death_date = $5 WHERE id = $6",
    )
    .bind(form.handle)
    .bind(form.given_name)
    .bind(form.family_name)
    .bind(form.birth_date)
    .bind(form.death_date)
    .bind(id)
    .execute(&pool)
    .await
    .unwrap();
    Redirect::to("/admin/musicians").into_response()
}

pub async fn admin_musician_delete(
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
pub async fn admin_compositions(
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
    let q = params.q.filter(|s| !s.trim().is_empty());

    let (total_count, compositions) = if let Some(ref query) = q {
        search_compositions_admin(&pool, query, limit, offset)
            .await
            .unwrap_or_default()
    } else {
        let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM compositions")
            .fetch_one(&pool)
            .await
            .unwrap_or(0);

        let comps = sqlx::query_as::<_, Composition>(
            "SELECT c.id, c.slug, (m.family_name || ': ' || c.title) as title, c.publish_date, c.composer_id FROM compositions c JOIN musicians m ON c.composer_id = m.id ORDER BY c.id LIMIT $1 OFFSET $2",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&pool)
        .await
        .unwrap_or_default();
        (count, comps)
    };

    let total_pages = (total_count as f64 / limit as f64).ceil() as i64;

    HtmlTemplate(AdminCompositionsTemplate {
        compositions,
        current_user: Some(auth.0),
        page,
        total_pages,
        q,
        active_nav: "compositions",
    })
    .into_response()
}

pub async fn admin_composition_new(auth: AuthUser) -> impl IntoResponse {
    if !matches!(auth.0.role, UserRole::Root | UserRole::Admin) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }
    HtmlTemplate(AdminCompositionEditTemplate {
        composition: Composition::default(),
        movements: vec![],
        current_user: Some(auth.0),
        active_nav: "compositions",
    })
    .into_response()
}

pub async fn admin_composition_create(
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

pub async fn admin_composition_edit(
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
    HtmlTemplate(AdminCompositionEditTemplate {
        composition,
        movements,
        current_user: Some(auth.0),
        active_nav: "compositions",
    })
    .into_response()
}

pub async fn admin_composition_update(
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

pub async fn admin_composition_delete(
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
pub async fn admin_movement_create(
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

pub async fn admin_movement_delete(
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
pub async fn admin_recordings(
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
        q: params.q,
        active_nav: "recordings",
    })
    .into_response()
}

pub async fn admin_recording_new(
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
    HtmlTemplate(AdminRecordingEditTemplate {
        recording: Recording::default(),
        musicians,
        current_user: Some(auth.0),
        active_nav: "recordings",
        composition_display: None,
    })
    .into_response()
}

pub async fn admin_recording_create(
    auth: AuthUser,
    State(pool): State<Pool<Postgres>>,
    Form(form): Form<CreateRecording>,
) -> impl IntoResponse {
    if !matches!(auth.0.role, UserRole::Root) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }
    let _ = sqlx::query("INSERT INTO recordings (artist_id, composition_id, movement_id, content_hash, file_key, mime_type, visibility) VALUES ($1, $2, $3, $4, $5, $6, $7)")
        .bind(form.artist_id)
        .bind(form.composition_id)
        .bind(form.movement_id)
        .bind(form.content_hash)
        .bind(form.file_key)
        .bind(form.mime_type)
        .bind(form.visibility)
        .execute(&pool)
        .await
        .unwrap();
    Redirect::to("/admin/recordings").into_response()
}

pub async fn admin_recording_edit(
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

    let composition_display = if recording.composition_id != 0 {
        if let Some(movement_id) = recording.movement_id {
            let row = sqlx::query(
                r#"
                SELECT 
                    c.title,
                    m.title,
                    m.index,
                    mus.given_name || ' ' || mus.family_name
                FROM compositions c
                JOIN musicians mus ON c.composer_id = mus.id
                JOIN movements m ON m.id = $2
                WHERE c.id = $1
                "#,
            )
            .bind(recording.composition_id)
            .bind(movement_id)
            .fetch_optional(&pool)
            .await
            .unwrap_or(None);

            if let Some(row) = row {
                use sqlx::Row;
                let c_title: String = row.get(0);
                let m_title: String = row.get(1);
                let m_index: i32 = row.get(2);
                let composer: String = row.get(3);
                Some(format!(
                    "{}: {} {}. {}",
                    composer,
                    c_title,
                    m_index + 1,
                    m_title
                ))
            } else {
                None
            }
        } else {
            let row = sqlx::query(
                r#"
                SELECT 
                    c.title,
                    mus.given_name || ' ' || mus.family_name
                FROM compositions c
                JOIN musicians mus ON c.composer_id = mus.id
                WHERE c.id = $1
                "#,
            )
            .bind(recording.composition_id)
            .fetch_optional(&pool)
            .await
            .unwrap_or(None);

            if let Some(row) = row {
                use sqlx::Row;
                let c_title: String = row.get(0);
                let composer: String = row.get(1);
                Some(format!("{}: {}", composer, c_title))
            } else {
                None
            }
        }
    } else {
        None
    };

    HtmlTemplate(AdminRecordingEditTemplate {
        recording,
        musicians,
        current_user: Some(auth.0),
        active_nav: "recordings",
        composition_display,
    })
    .into_response()
}

pub async fn admin_recording_update(
    auth: AuthUser,
    State(pool): State<Pool<Postgres>>,
    Path(id): Path<i32>,
    Form(form): Form<CreateRecording>,
) -> impl IntoResponse {
    if !matches!(auth.0.role, UserRole::Root | UserRole::Admin) {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }
    let _ = sqlx::query(
        "UPDATE recordings SET artist_id = $1, composition_id = $2, movement_id = $3, content_hash = $4, file_key = $5, mime_type = $6, visibility = $7 WHERE id = $8",
    )
    .bind(form.artist_id)
    .bind(form.composition_id)
    .bind(form.movement_id)
    .bind(form.content_hash)
    .bind(form.file_key)
    .bind(form.mime_type)
    .bind(form.visibility)
    .bind(id)
    .execute(&pool)
    .await
    .unwrap();
    Redirect::to("/admin/recordings").into_response()
}

pub async fn admin_recording_delete(
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
pub struct OpenOpusDump {
    pub composers: Vec<OpenOpusComposer>,
}

#[derive(Deserialize)]
pub struct OpenOpusComposer {
    pub complete_name: String, // "John Adams"
    pub name: String,          // "Adams"
    pub birth: Option<String>, // "1947-01-01"
    pub death: Option<String>, // null or "YYYY-MM-DD"
    pub works: Vec<OpenOpusWork>,
}

#[derive(Deserialize)]
pub struct OpenOpusWork {
    pub title: String,
    pub movements: Option<Vec<String>>,
}

fn slugify(s: &str) -> String {
    deunicode::deunicode(s)
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn parse_date(s: Option<&String>) -> Option<chrono::NaiveDate> {
    s.and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
}

pub async fn admin_import(auth: AuthUser, State(pool): State<Pool<Postgres>>) -> impl IntoResponse {
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
        active_nav: "import",
    })
    .into_response()
}

pub async fn admin_import_start(
    auth: AuthUser,
    State(pool): State<Pool<Postgres>>,
    State(cancel_token_mutex): State<Arc<Mutex<Option<CancellationToken>>>>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    if !auth.0.is_privileged() {
        return (StatusCode::NOT_FOUND, HtmlTemplate(NotFoundTemplate)).into_response();
    }

    let mut token_lock = cancel_token_mutex.lock().await;
    if token_lock.is_some() {
        // Job already running
        return Redirect::to("/admin/import").into_response();
    }

    let mut file_content = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or_default().to_string();
        tracing::info!("Import: Received field: {}", name);
        if name == "file" {
            match field.bytes().await {
                Ok(bytes) => {
                    tracing::info!("Import: Received file bytes: {} bytes", bytes.len());
                    match String::from_utf8(bytes.to_vec()) {
                        Ok(text) => file_content = Some(text),
                        Err(e) => {
                            tracing::error!("Import: Failed to convert file bytes to string: {}", e)
                        }
                    }
                }
                Err(e) => tracing::error!("Import: Failed to read file bytes: {}", e),
            }
        }
    }

    let content = match file_content {
        Some(c) => c,
        None => {
            tracing::error!("Import: No valid file content found.");
            return Redirect::to("/admin/import").into_response();
        }
    };

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

    tokio::spawn(async move {
        let result = run_import(pool_clone, cancel_token, job_id, content).await;

        let mut lock = cancel_token_mutex_clone.lock().await;
        *lock = None;

        if let Err(e) = result {
            tracing::error!("Import job {} failed: {:?}", job_id, e);
        }
    });

    Redirect::to("/admin/import").into_response()
}

pub async fn admin_import_cancel(
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
    content: String,
) -> anyhow::Result<()> {
    let dump: OpenOpusDump = serde_json::from_str(&content)?;

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

    for composer in dump.composers {
        if cancel_token.is_cancelled() {
            update_job!(ImportJobStatus::Cancelled);
            return Ok(());
        }

        // Process Musician
        let handle = slugify(&composer.complete_name);

        // Split complete_name into given and family
        // Heuristic: Last word is family name, rest is given name.
        // Or better: Use `name` field from JSON as family name (it seems to be just the surname usually).
        // Then remove `name` from `complete_name` to get `given_name`.

        let family_name = composer.name.trim().to_string();
        let given_name = if let Some(stripped) = composer.complete_name.strip_suffix(&family_name) {
            stripped.trim().to_string()
        } else {
            // Fallback if complete_name doesn't end with name (e.g. name="Bach", complete="Johann Sebastian Bach" -> "Johann Sebastian")
            // But what if name="Bach" and complete="Bach, Johann Sebastian"?
            // OpenOpus complete_name seems to be "Given Family".
            let parts: Vec<&str> = composer.complete_name.split_whitespace().collect();
            if parts.len() > 1 {
                parts[..parts.len() - 1].join(" ")
            } else {
                "".to_string()
            }
        };

        // If given_name ended up empty but complete_name wasn't same as family_name, try to fix.
        // Actually, let's just use the logic: given = complete replace family with "" if matches.
        let given_name = if given_name.is_empty() && composer.complete_name != family_name {
            composer
                .complete_name
                .replace(&family_name, "")
                .trim()
                .to_string()
        } else {
            given_name
        };

        let birth_date = parse_date(composer.birth.as_ref());
        let death_date = parse_date(composer.death.as_ref());

        let musician_id = {
            let existing =
                sqlx::query_as::<_, Musician>("SELECT * FROM musicians WHERE handle = $1")
                    .bind(&handle)
                    .fetch_optional(&pool)
                    .await?;

            if let Some(m) = existing {
                let needs_update = m.given_name != given_name
                    || m.family_name != family_name
                    || m.birth_date != birth_date
                    || m.death_date != death_date;

                if !needs_update {
                    success_count += 1;
                    m.id
                } else {
                    let _ = sqlx::query(
                        "UPDATE musicians SET given_name = $1, family_name = $2, birth_date = $3, death_date = $4 WHERE id = $5",
                    )
                    .bind(&given_name)
                    .bind(&family_name)
                    .bind(birth_date)
                    .bind(death_date)
                    .bind(m.id)
                    .execute(&pool)
                    .await?;
                    success_count += 1;
                    m.id
                }
            } else {
                let res = sqlx::query_scalar::<_, i32>(
                    "INSERT INTO musicians (handle, given_name, family_name, birth_date, death_date) VALUES ($1, $2, $3, $4, $5) RETURNING id",
                )
                .bind(&handle)
                .bind(&given_name)
                .bind(&family_name)
                .bind(birth_date)
                .bind(death_date)
                .fetch_one(&pool)
                .await;

                match res {
                    Ok(id) => {
                        success_count += 1;
                        id
                    }
                    Err(_) => {
                        failure_count += 1;
                        continue;
                    }
                }
            }
        };

        // Process Works
        for work in composer.works {
            if cancel_token.is_cancelled() {
                update_job!(ImportJobStatus::Cancelled);
                return Ok(());
            }

            let title = work.title;
            // let subtitle = work.subtitle; // Unused for now
            let slug_base = format!("{} {}", composer.complete_name, title);
            let slug = slugify(&slug_base);

            let existing_comp =
                sqlx::query_as::<_, Composition>("SELECT * FROM compositions WHERE slug = $1")
                    .bind(&slug)
                    .fetch_optional(&pool)
                    .await?;

            if let Some(comp) = existing_comp {
                let mut content_changed = comp.title != title || comp.composer_id != musician_id;

                if !content_changed {
                    let existing_movements = sqlx::query_as::<_, Movement>(
                        "SELECT * FROM movements WHERE composition_id = $1 ORDER BY index",
                    )
                    .bind(comp.id)
                    .fetch_all(&pool)
                    .await?;

                    let new_movements =
                        work.movements.as_ref().map(|v| v.as_slice()).unwrap_or(&[]);

                    if existing_movements.len() != new_movements.len() {
                        content_changed = true;
                    } else {
                        for (i, mv_title) in new_movements.iter().enumerate() {
                            if existing_movements[i].title != *mv_title
                                || existing_movements[i].index != i as i32
                            {
                                content_changed = true;
                                break;
                            }
                        }
                    }
                }

                if !content_changed {
                    skip_count += 1;
                } else {
                    let _ = sqlx::query(
                        "UPDATE compositions SET title = $1, composer_id = $2 WHERE id = $3",
                    )
                    .bind(&title)
                    .bind(musician_id)
                    .bind(comp.id)
                    .execute(&pool)
                    .await?;

                    let existing_movements = sqlx::query_as::<_, Movement>(
                        "SELECT * FROM movements WHERE composition_id = $1 ORDER BY index",
                    )
                    .bind(comp.id)
                    .fetch_all(&pool)
                    .await?;

                    let new_movements_data =
                        work.movements.as_ref().map(|v| v.as_slice()).unwrap_or(&[]);

                    // Update or Insert
                    for (i, mv_title) in new_movements_data.iter().enumerate() {
                        let mv_slug = slugify(&format!(
                            "{} {} {} {}",
                            composer.complete_name, title, i, mv_title
                        ));

                        if i < existing_movements.len() {
                            let existing = &existing_movements[i];
                            if existing.title != *mv_title || existing.slug != mv_slug {
                                let _ = sqlx::query(
                                    "UPDATE movements SET title = $1, slug = $2 WHERE id = $3",
                                )
                                .bind(mv_title)
                                .bind(mv_slug)
                                .bind(existing.id)
                                .execute(&pool)
                                .await;
                            }
                        } else {
                            let _ = sqlx::query(
                                "INSERT INTO movements (slug, title, index, composition_id) VALUES ($1, $2, $3, $4)",
                            )
                            .bind(mv_slug)
                            .bind(mv_title)
                            .bind(i as i32)
                            .bind(comp.id)
                            .execute(&pool)
                            .await;
                        }
                    }

                    // Delete extras
                    if existing_movements.len() > new_movements_data.len() {
                        tracing::info!(
                            "Import: Deleting {} extra movement(s) for composition {}.",
                            existing_movements.len() - new_movements_data.len(),
                            comp.slug
                        );
                        for movement in existing_movements.iter().skip(new_movements_data.len()) {
                            let _ = sqlx::query(
                                "UPDATE recordings SET movement_id = NULL WHERE movement_id = $1",
                            )
                            .bind(movement.id)
                            .execute(&pool)
                            .await;
                            let _ = sqlx::query("DELETE FROM movements WHERE id = $1")
                                .bind(movement.id)
                                .execute(&pool)
                                .await;
                        }
                    }
                    success_count += 1;
                }
            } else {
                let res = sqlx::query_scalar::<_, i32>(
                    "INSERT INTO compositions (slug, title, composer_id) VALUES ($1, $2, $3) RETURNING id",
                )
                .bind(&slug)
                .bind(&title)
                .bind(musician_id)
                .fetch_one(&pool)
                .await;

                match res {
                    Ok(composition_id) => {
                        success_count += 1;
                        if let Some(movements) = work.movements {
                            for (i, mv_title) in movements.iter().enumerate() {
                                let mv_slug = slugify(&format!(
                                    "{} {} {} {}",
                                    composer.complete_name, title, i, mv_title
                                ));
                                let _ = sqlx::query(
                                    "INSERT INTO movements (slug, title, index, composition_id) VALUES ($1, $2, $3, $4)",
                                )
                                .bind(mv_slug)
                                .bind(mv_title)
                                .bind(i as i32)
                                .bind(composition_id)
                                .execute(&pool)
                                .await;
                            }
                        }
                    }
                    Err(_) => failure_count += 1,
                }
            }

            if (success_count + skip_count + failure_count) % 50 == 0 {
                update_job!(ImportJobStatus::Processing);
            }
        }
    }

    update_job!(ImportJobStatus::Completed);
    Ok(())
}
