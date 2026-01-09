use axum::{
    Form,
    extract::{Multipart, Path, Query, Request, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use sqlx::{Pool, Postgres};
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tower::ServiceExt;
use tower_http::services::ServeFile;
use uuid::Uuid;

use crate::db::{CreateMusician, Musician, RecordingFeedItem};
use crate::extractors::{AuthUser, HtmxRequest, OptionalAuthUser};
use crate::storage::StorageService;
use crate::view::{
    CompositionPickerTemplate, HtmlTemplate, IndexContentTemplate, IndexTemplate, PlayerTemplate,
    SettingsContentTemplate, SettingsTemplate, UploadContentTemplate, UploadTemplate,
};

pub async fn index(
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
            r.file_key,
            r.mime_type,
            r.notes
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
        HtmlTemplate(IndexContentTemplate { recordings }).into_response()
    } else {
        HtmlTemplate(IndexTemplate {
            current_user: auth.0,
            recordings,
        })
        .into_response()
    }
}

pub async fn settings(
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

pub async fn settings_post(
    auth: AuthUser,
    State(pool): State<Pool<Postgres>>,
    Form(form): Form<CreateMusician>,
) -> impl IntoResponse {
    let _ = sqlx::query(
        "UPDATE musicians SET handle = $1, given_name = $2, family_name = $3, birth_date = $4, death_date = $5 WHERE id = $6",
    )
    .bind(form.handle)
    .bind(form.given_name)
    .bind(form.family_name)
    .bind(form.birth_date)
    .bind(form.death_date)
    .bind(auth.0.musician_id)
    .execute(&pool)
    .await
    .unwrap();
    Redirect::to("/settings")
}

pub async fn upload(auth: AuthUser, htmx: HtmxRequest) -> Response {
    if htmx.is_hx_boosted {
        HtmlTemplate(UploadContentTemplate { error: None }).into_response()
    } else {
        HtmlTemplate(UploadTemplate {
            current_user: Some(auth.0),
            error: None,
        })
        .into_response()
    }
}

pub async fn upload_post(
    auth: AuthUser,
    htmx: HtmxRequest,
    State(pool): State<Pool<Postgres>>,
    State(storage): State<Arc<dyn StorageService>>,
    mut multipart: Multipart,
) -> Response {
    use axum::body::Bytes;

    let render_error = |err: String| {
        if htmx.is_hx_boosted {
            HtmlTemplate(UploadContentTemplate { error: Some(err) }).into_response()
        } else {
            HtmlTemplate(UploadTemplate {
                current_user: Some(auth.0.clone()),
                error: Some(err),
            })
            .into_response()
        }
    };

    let mut file_data: Option<Bytes> = None;
    let mut composition_id = None;
    let mut movement_id = None;
    let mut notes = None;

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
        } else if name == "movement_id" {
            if let Ok(txt) = field.text().await {
                tracing::info!("Received movement_id: {}", txt);
                movement_id = txt.parse::<i32>().ok();
            }
        } else if name == "notes" {
            if let Ok(txt) = field.text().await {
                let txt = txt.trim();
                if !txt.is_empty() {
                    notes = Some(txt.to_string());
                }
            }
        }
    }

    let data = match file_data {
        Some(d) => d,
        None => {
            return render_error("No file uploaded.".to_string());
        }
    };

    let comp_id = match composition_id {
        Some(id) => id,
        None => {
            return render_error("No composition selected.".to_string());
        }
    };

    let mime_type = if let Some(kind) = infer::get(&data) {
        kind.mime_type().to_string()
    } else {
        "application/octet-stream".to_string()
    };

    if !mime_type.starts_with("audio/") {
        return render_error("Uploaded file is not a valid audio file.".to_string());
    }

    let file_key = Uuid::new_v4().to_string();
    if let Err(e) = storage.upload(&file_key, data.to_vec(), &mime_type).await {
        tracing::error!("Failed to upload file: {}", e);
        return render_error("Failed to save file.".to_string());
    }

    let _ = sqlx::query(
        "INSERT INTO recordings (artist_id, composition_id, movement_id, content_hash, file_key, mime_type, notes) VALUES ($1, $2, $3, $4, $5, $6, $7)"
    )
    .bind(auth.0.musician_id)
    .bind(comp_id)
    .bind(movement_id)
    .bind("hash_placeholder")
    .bind(file_key)
    .bind(mime_type)
    .bind(notes)
    .execute(&pool)
    .await
    .unwrap();

    Redirect::to("/").into_response()
}

pub async fn serve_audio(
    Path(key): Path<String>,
    State(storage): State<Arc<dyn StorageService>>,
    req: Request,
) -> Response {
    if std::env::var("APP_ENV").unwrap_or_default() == "production" {
        return Redirect::temporary(&storage.get_url(&key)).into_response();
    }

    let path = format!("uploads/recordings/{}", key);

    let mut file = match tokio::fs::File::open(&path).await {
        Ok(file) => file,
        Err(_) => return (StatusCode::NOT_FOUND, "File not found").into_response(),
    };

    let mut buffer = [0; 1024];
    let n = match file.read(&mut buffer).await {
        Ok(n) => n,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Error reading file").into_response(),
    };

    let mime_type = match infer::get(&buffer[..n]) {
        Some(kind) if kind.mime_type().starts_with("audio/") => kind.mime_type().to_string(),
        _ => return (StatusCode::BAD_REQUEST, "File is not an audio file").into_response(),
    };

    match ServeFile::new(&path).oneshot(req).await {
        Ok(response) => {
            let mut response = response.into_response();
            if let Ok(value) = axum::http::HeaderValue::from_str(&mime_type) {
                response
                    .headers_mut()
                    .insert(axum::http::header::CONTENT_TYPE, value);
            }
            response
        }
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Error serving file").into_response(),
    }
}

pub async fn get_player(
    Path(id): Path<i32>,
    State(pool): State<Pool<Postgres>>,
) -> impl IntoResponse {
    let recording = sqlx::query_as::<_, RecordingFeedItem>(
        r#"
        SELECT 
            r.id,
            m.handle as artist_handle,
            c.title as composition_title,
            mv.index as movement_index,
            mv.title as movement_title,
            r.created_at,
            r.file_key,
            r.mime_type,
            r.notes
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
pub struct SearchParams {
    q: String,
}

#[derive(sqlx::FromRow)]
pub struct SearchResult {
    composition_id: i32,
    composition_title: String,
    movement_id: Option<i32>,
    movement_index: Option<i32>,
    movement_title: Option<String>,
    composer_name: String,
}

pub async fn search_compositions(
    State(pool): State<Pool<Postgres>>,
    Query(params): Query<SearchParams>,
) -> impl IntoResponse {
    if params.q.trim().is_empty() {
        return axum::response::Html("".to_string()).into_response();
    }

    let search_words: Vec<String> = params
        .q
        .split_whitespace()
        .map(|s| format!("%{}%", s))
        .collect();

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
          unaccent(concat_ws(' ', c.title, m.title, mus.handle, mus.given_name, mus.family_name)) 
          ILIKE ALL(SELECT unaccent(x) FROM unnest($1::text[]) x)
        ORDER BY c.title, m.index
        LIMIT 50
        "#,
    )
    .bind(search_words)
    .fetch_all(&pool)
    .await
    .unwrap_or_default();

    if results.is_empty() {
        return axum::response::Html(
            r#"<div class="p-4 text-sm text-gray-500 text-center">No compositions found.</div>"#
                .to_string(),
        )
        .into_response();
    }

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

        let mut url = format!("/upload/select-composition/{}", res.composition_id);
        if let Some(m_id) = res.movement_id {
            url.push_str(&format!("?movement_id={}", m_id));
        }

        html.push_str(&format!(
            r##"<div class="cursor-pointer hover:bg-indigo-50 p-2 text-sm text-gray-700 border-b last:border-b-0" 
                    hx-get="{}"
                    hx-target="#composition-picker"
                    hx-swap="outerHTML">
                {}
            </div>"##,
            url, display_text
        ));
    }

    axum::response::Html(html).into_response()
}

#[derive(Deserialize)]
pub struct SelectCompositionParams {
    movement_id: Option<i32>,
}

pub async fn select_composition(
    Path(id): Path<i32>,
    Query(params): Query<SelectCompositionParams>,
    State(pool): State<Pool<Postgres>>,
) -> impl IntoResponse {
    let result = if let Some(m_id) = params.movement_id {
        sqlx::query_as::<_, SearchResult>(
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
            JOIN movements m ON c.id = m.composition_id
            WHERE c.id = $1 AND m.id = $2
            LIMIT 1
            "#,
        )
        .bind(id)
        .bind(m_id)
        .fetch_optional(&pool)
        .await
        .unwrap_or(None)
    } else {
        sqlx::query_as::<_, SearchResult>(
            r#"
            SELECT 
                c.id as composition_id,
                c.title as composition_title,
                NULL::integer as movement_id,
                NULL::integer as movement_index,
                NULL::text as movement_title,
                mus.given_name || ' ' || mus.family_name as composer_name
            FROM compositions c
            JOIN musicians mus ON c.composer_id = mus.id
            WHERE c.id = $1
            LIMIT 1
            "#,
        )
        .bind(id)
        .fetch_optional(&pool)
        .await
        .unwrap_or(None)
    };

    if let Some(res) = result {
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

        let movement_input = if let Some(m_id) = res.movement_id {
            format!(
                r#"<input type="hidden" name="movement_id" value="{}">"#,
                m_id
            )
        } else {
            "".to_string()
        };

        let html = format!(
            r##"<div id="composition-picker" class="relative">
                <label class="block text-sm font-medium text-gray-700">Composition <span class="text-red-500">*</span></label>
                <input type="hidden" name="composition_id" value="{}" required>
                {}
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
            res.composition_id, movement_input, display_text
        );
        axum::response::Html(html).into_response()
    } else {
        // Fallback if not found (shouldn't happen often)
        reset_composition().await.into_response()
    }
}

pub async fn reset_composition() -> impl IntoResponse {
    HtmlTemplate(CompositionPickerTemplate)
}
