use axum::{
    Form,
    extract::{Multipart, Path, Query, Request, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use sqlx::{Pool, Postgres};
use std::sync::Arc;
use uuid::Uuid;

use crate::db::{CreateMusician, Musician, RecordingFeedItem};
use crate::extractors::{AuthUser, HtmxRequest, OptionalAuthUser};
use crate::storage::StorageService;
use crate::view::{
    CompositionPickerTemplate, HtmlTemplate, IndexContentTemplate, IndexTemplate,
    ProfileContentTemplate, ProfileTemplate, RecordingContentTemplate,
    RecordingEditContentTemplate, RecordingEditTemplate, RecordingTemplate,
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
            r.slug_id,
            m.handle as artist_handle,
            (c_mus.family_name || ': ' || c.title) as composition_title,
            mv.index as movement_index,
            mv.title as movement_title,
            r.created_at,
            r.file_key,
            r.mime_type,
            r.notes
        FROM recordings r
        JOIN musicians m ON r.artist_id = m.id
        JOIN compositions c ON r.composition_id = c.id
        JOIN musicians c_mus ON c.composer_id = c_mus.id
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
    let mut field_mime = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or_default().to_string();
        tracing::info!("Received field: {}", name);
        if name == "file" {
            field_mime = field.content_type().map(|s| s.to_string());
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
        field_mime.unwrap_or_else(|| "application/octet-stream".to_string())
    };

    if !mime_type.starts_with("audio/") {
        return render_error("Uploaded file is not a valid audio file.".to_string());
    }

    let mut hasher = Sha256::new();
    hasher.update(&data);
    let content_hash = hex::encode(hasher.finalize());

    let file_key = Uuid::new_v4().to_string();
    if let Err(e) = storage
        .upload("recordings", &file_key, data.to_vec(), &mime_type)
        .await
    {
        tracing::error!("Failed to upload file: {}", e);
        return render_error("Failed to save file.".to_string());
    }

    let _ = sqlx::query(
        "INSERT INTO recordings (artist_id, composition_id, movement_id, content_hash, file_key, mime_type, notes) VALUES ($1, $2, $3, $4, $5, $6, $7)"
    )
    .bind(auth.0.musician_id)
    .bind(comp_id)
    .bind(movement_id)
    .bind(content_hash)
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
    State(pool): State<Pool<Postgres>>,
    State(storage): State<Arc<dyn StorageService>>,
    req: Request,
) -> Response {
    let recording = match sqlx::query!(
        "SELECT content_hash, mime_type FROM recordings WHERE file_key = $1",
        key
    )
    .fetch_optional(&pool)
    .await
    {
        Ok(Some(r)) => r,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };

    let range = req
        .headers()
        .get(axum::http::header::RANGE)
        .and_then(|v| v.to_str().ok());

    match storage.get_content("recordings", &key, range).await {
        Ok(file) => {
            let status = if file.content_range.is_some() {
                StatusCode::PARTIAL_CONTENT
            } else {
                StatusCode::OK
            };

            let mut response = (status, file.body).into_response();
            let headers = response.headers_mut();

            // Use stored MIME type if available, otherwise fallback to inferred
            if let Ok(content_type) = recording.mime_type.parse() {
                headers.insert(axum::http::header::CONTENT_TYPE, content_type);
            } else if let Ok(content_type) = file.content_type.parse() {
                headers.insert(axum::http::header::CONTENT_TYPE, content_type);
            }

            if let Ok(len) = axum::http::HeaderValue::from_str(&file.content_length.to_string()) {
                headers.insert(axum::http::header::CONTENT_LENGTH, len);
            }
            if let Ok(accept_ranges) = file.accept_ranges.parse() {
                headers.insert(axum::http::header::ACCEPT_RANGES, accept_ranges);
            }
            if let Some(content_range) = file.content_range {
                if let Ok(content_range) = content_range.parse() {
                    headers.insert(axum::http::header::CONTENT_RANGE, content_range);
                }
            }

            // Safari requires an ETag (or Last-Modified) for Range requests to work reliably
            let etag = format!("\"{}\"", recording.content_hash);
            if let Ok(hv) = etag.parse() {
                headers.insert(axum::http::header::ETAG, hv);
            }

            headers.insert(
                axum::http::header::CACHE_CONTROL,
                // 1 month
                axum::http::HeaderValue::from_static("s-maxage=2592000"),
            );

            response
        }
        Err(e) => {
            tracing::error!("Error serving audio for key {}: {}", key, e);
            // If it was a range error from S3, it might manifest as a 416, but we'll return 404 for simplicity unless we want more complex mapping
            StatusCode::NOT_FOUND.into_response()
        }
    }
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
            r#"<div class="p-6 text-[10px] uppercase tracking-widest text-sonata-slate text-center font-light">No repertoire found.</div>"#
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
            r##"<div class="cursor-pointer hover:bg-sonata-black p-4 text-xs font-light text-sonata-pearl border-b border-sonata-border/30 last:border-b-0 transition-colors" 
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
                <label class="block text-[10px] uppercase tracking-widest font-light text-sonata-slate mb-2">Selected Repertoire</label>
                <input type="hidden" name="composition_id" value="{}" required>
                {}
                <div class="flex items-center justify-between p-4 border border-sonata-charcoal rounded-sm bg-sonata-black">
                    <span class="text-xs font-light text-sonata-pearl">{}</span>
                    <button type="button" 
                            hx-get="/upload/reset-composition" 
                            hx-target="#composition-picker" 
                            hx-swap="outerHTML" 
                            class="text-sonata-slate hover:text-sonata-pearl transition-colors">
                        <svg class="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1" d="M6 18L18 6M6 6l12 12" />
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

pub async fn profile(
    Path(handle): Path<String>,
    auth: OptionalAuthUser,
    htmx: HtmxRequest,
    State(pool): State<Pool<Postgres>>,
) -> Response {
    let profile_user =
        match sqlx::query_as::<_, Musician>("SELECT * FROM musicians WHERE handle = $1")
            .bind(&handle)
            .fetch_optional(&pool)
            .await
        {
            Ok(Some(m)) => m,
            Ok(None) => return StatusCode::NOT_FOUND.into_response(),
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };

    let recordings = sqlx::query_as::<_, RecordingFeedItem>(
        r#"
        SELECT 
            r.id,
            r.slug_id,
            m.handle as artist_handle,
            (c_mus.family_name || ': ' || c.title) as composition_title,
            mv.index as movement_index,
            mv.title as movement_title,
            r.created_at,
            r.file_key,
            r.mime_type,
            r.notes
        FROM recordings r
        JOIN musicians m ON r.artist_id = m.id
        JOIN compositions c ON r.composition_id = c.id
        JOIN musicians c_mus ON c.composer_id = c_mus.id
        LEFT JOIN movements mv ON r.movement_id = mv.id
        WHERE m.id = $1
        ORDER BY r.created_at DESC
        "#,
    )
    .bind(profile_user.id)
    .fetch_all(&pool)
    .await
    .unwrap_or_default();

    if htmx.is_hx_boosted {
        HtmlTemplate(ProfileContentTemplate {
            current_user: auth.0,
            profile_user,
            recordings,
        })
        .into_response()
    } else {
        HtmlTemplate(ProfileTemplate {
            current_user: auth.0,
            profile_user,
            recordings,
        })
        .into_response()
    }
}

pub async fn recording_detail(
    Path((handle, slug_id)): Path<(String, i32)>,
    auth: OptionalAuthUser,
    htmx: HtmxRequest,
    State(pool): State<Pool<Postgres>>,
) -> Response {
    let recording = match sqlx::query_as::<_, RecordingFeedItem>(
        r#"
        SELECT 
            r.id,
            r.slug_id,
            m.handle as artist_handle,
            (c_mus.family_name || ': ' || c.title) as composition_title,
            mv.index as movement_index,
            mv.title as movement_title,
            r.created_at,
            r.file_key,
            r.mime_type,
            r.notes
        FROM recordings r
        JOIN musicians m ON r.artist_id = m.id
        JOIN compositions c ON r.composition_id = c.id
        JOIN musicians c_mus ON c.composer_id = c_mus.id
        LEFT JOIN movements mv ON r.movement_id = mv.id
        WHERE m.handle = $1 AND r.slug_id = $2
        "#,
    )
    .bind(&handle)
    .bind(slug_id)
    .fetch_optional(&pool)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    if htmx.is_hx_boosted {
        HtmlTemplate(RecordingContentTemplate {
            current_user: auth.0,
            recording,
        })
        .into_response()
    } else {
        HtmlTemplate(RecordingTemplate {
            current_user: auth.0,
            recording,
        })
        .into_response()
    }
}

pub async fn recording_edit(
    Path((handle, slug_id)): Path<(String, i32)>,
    auth: AuthUser,
    htmx: HtmxRequest,
    State(pool): State<Pool<Postgres>>,
) -> Response {
    let recording = match sqlx::query_as::<_, RecordingFeedItem>(
        r#"
        SELECT 
            r.id,
            r.slug_id,
            m.handle as artist_handle,
            (c_mus.family_name || ': ' || c.title) as composition_title,
            mv.index as movement_index,
            mv.title as movement_title,
            r.created_at,
            r.file_key,
            r.mime_type,
            r.notes
        FROM recordings r
        JOIN musicians m ON r.artist_id = m.id
        JOIN compositions c ON r.composition_id = c.id
        JOIN musicians c_mus ON c.composer_id = c_mus.id
        LEFT JOIN movements mv ON r.movement_id = mv.id
        WHERE m.handle = $1 AND r.slug_id = $2
        "#,
    )
    .bind(&handle)
    .bind(slug_id)
    .fetch_optional(&pool)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    if auth.0.handle != recording.artist_handle {
        return StatusCode::FORBIDDEN.into_response();
    }

    if htmx.is_hx_boosted {
        HtmlTemplate(RecordingEditContentTemplate { recording }).into_response()
    } else {
        HtmlTemplate(RecordingEditTemplate {
            current_user: Some(auth.0),
            recording,
        })
        .into_response()
    }
}

#[derive(Deserialize)]
pub struct UpdateRecordingNotes {
    pub notes: String,
}

pub async fn recording_update(
    Path((handle, slug_id)): Path<(String, i32)>,
    auth: AuthUser,
    State(pool): State<Pool<Postgres>>,
    Form(form): Form<UpdateRecordingNotes>,
) -> impl IntoResponse {
    let recording = match sqlx::query_as::<_, RecordingFeedItem>(
        r#"
        SELECT 
            r.id,
            r.slug_id,
            m.handle as artist_handle,
            (c_mus.family_name || ': ' || c.title) as composition_title,
            mv.index as movement_index,
            mv.title as movement_title,
            r.created_at,
            r.file_key,
            r.mime_type,
            r.notes
        FROM recordings r
        JOIN musicians m ON r.artist_id = m.id
        JOIN compositions c ON r.composition_id = c.id
        JOIN musicians c_mus ON c.composer_id = c_mus.id
        LEFT JOIN movements mv ON r.movement_id = mv.id
        WHERE m.handle = $1 AND r.slug_id = $2
        "#,
    )
    .bind(&handle)
    .bind(slug_id)
    .fetch_optional(&pool)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    if auth.0.handle != recording.artist_handle {
        return StatusCode::FORBIDDEN.into_response();
    }

    // 2. Update notes
    let _ = sqlx::query("UPDATE recordings SET notes = $1 WHERE id = $2")
        .bind(if form.notes.trim().is_empty() {
            None
        } else {
            Some(form.notes.trim())
        })
        .bind(recording.id)
        .execute(&pool)
        .await
        .unwrap();

    Redirect::to(&format!("/{}/recordings/{}", handle, slug_id)).into_response()
}

pub async fn recording_delete(
    Path((handle, slug_id)): Path<(String, i32)>,
    auth: AuthUser,
    State(pool): State<Pool<Postgres>>,
    State(storage): State<Arc<dyn StorageService>>,
) -> Response {
    // 1. Verify existence and ownership
    let recording = match sqlx::query!(
        r#"
        SELECT
            r.id,
            m.handle as artist_handle,
            r.file_key
        FROM recordings r
        JOIN musicians m ON r.artist_id = m.id
        WHERE m.handle = $1 AND r.slug_id = $2
        "#,
        handle,
        slug_id
    )
    .fetch_optional(&pool)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    if auth.0.handle != recording.artist_handle {
        return StatusCode::FORBIDDEN.into_response();
    }

    // 2. Delete from storage
    if let Err(e) = storage.delete("recordings", &recording.file_key).await {
        tracing::error!("Failed to delete file from storage: {}", e);
        // We might want to continue anyway to clean up DB, or return error.
        // Let's continue so the DB record doesn't become an orphan without a file.
    }

    // 3. Delete from database
    let _ = sqlx::query("DELETE FROM recordings WHERE id = $1")
        .bind(recording.id)
        .execute(&pool)
        .await
        .unwrap();

    Redirect::to(&format!("/{}", handle)).into_response()
}
