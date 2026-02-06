use serde::Deserialize;
use sqlx::{Pool, Postgres};

#[derive(sqlx::Type, serde::Serialize, Clone, Debug, PartialEq)]
#[sqlx(type_name = "user_role", rename_all = "lowercase")]
pub enum UserRole {
    Root,
    Admin,
    User,
}

#[derive(sqlx::Type, serde::Serialize, Clone, Debug, PartialEq)]
#[sqlx(type_name = "import_job_status", rename_all = "lowercase")]
pub enum ImportJobStatus {
    Pending,
    Processing,
    Completed,
    Failed,
    Cancelled,
}

impl std::fmt::Display for ImportJobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportJobStatus::Pending => write!(f, "Pending"),
            ImportJobStatus::Processing => write!(f, "Processing"),
            ImportJobStatus::Completed => write!(f, "Completed"),
            ImportJobStatus::Failed => write!(f, "Failed"),
            ImportJobStatus::Cancelled => write!(f, "Cancelled"),
        }
    }
}

#[derive(sqlx::Type, serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
#[sqlx(type_name = "visibility", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    Public,
    Unlisted,
    Private,
}

impl Default for Visibility {
    fn default() -> Self {
        Self::Public
    }
}

impl std::fmt::Display for Visibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Visibility::Public => write!(f, "Public"),
            Visibility::Unlisted => write!(f, "Unlisted"),
            Visibility::Private => write!(f, "Private"),
        }
    }
}

#[derive(sqlx::FromRow, serde::Serialize, Clone, Debug)]
pub struct ImportJob {
    pub id: i32,
    pub status: ImportJobStatus,
    pub success_count: i32,
    pub skip_count: i32,
    pub failure_count: i32,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

impl std::fmt::Display for UserRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UserRole::Root => write!(f, "Root"),
            UserRole::Admin => write!(f, "Admin"),
            UserRole::User => write!(f, "User"),
        }
    }
}

#[derive(sqlx::FromRow, serde::Serialize, Clone, Debug)]
pub struct User {
    pub id: i32,
    pub email: String,
    pub password_hash: String,
    pub salt: String,
    pub musician_id: i32,
    pub handle: String,
    pub role: UserRole,
}

impl User {
    pub fn is_privileged(&self) -> bool {
        matches!(self.role, UserRole::Root | UserRole::Admin)
    }
}

#[derive(sqlx::FromRow, serde::Serialize, Clone, Default)]
pub struct Musician {
    pub id: i32,
    pub handle: String,
    pub given_name: String,
    pub family_name: String,
    pub birth_date: Option<chrono::NaiveDate>,
    pub death_date: Option<chrono::NaiveDate>,
}

#[derive(serde::Deserialize)]
pub struct CreateMusician {
    pub handle: String,
    pub given_name: String,
    pub family_name: String,
    #[serde(default)]
    #[serde(deserialize_with = "empty_string_as_none")]
    pub birth_date: Option<chrono::NaiveDate>,
    #[serde(default)]
    #[serde(deserialize_with = "empty_string_as_none")]
    pub death_date: Option<chrono::NaiveDate>,
}

#[derive(sqlx::FromRow, serde::Serialize, Clone, Default)]
pub struct Composition {
    pub id: i32,
    pub slug: String,
    pub title: String,
    pub publish_date: Option<chrono::NaiveDate>,
    pub composer_id: i32,
}

#[derive(serde::Deserialize)]
pub struct CreateComposition {
    pub slug: String,
    pub title: String,
    #[serde(default)]
    #[serde(deserialize_with = "empty_string_as_none")]
    pub publish_date: Option<chrono::NaiveDate>,
    pub composer_id: i32,
}

#[derive(sqlx::FromRow, serde::Serialize, Clone, Default)]
pub struct Movement {
    pub id: i32,
    pub slug: String,
    pub title: String,
    pub index: i32,
    pub composition_id: i32,
}

#[derive(serde::Deserialize)]
pub struct CreateMovement {
    pub slug: String,
    pub title: String,
    pub index: i32,
}

#[derive(sqlx::FromRow, serde::Serialize, Clone, Default)]
pub struct Recording {
    pub id: i32,
    pub artist_id: i32,
    pub slug_id: i32,
    pub composition_id: i32,
    pub movement_id: Option<i32>,
    pub content_hash: String,
    pub file_key: String,
    pub mime_type: String,
    pub content_length: i64,
    pub notes: Option<String>,
    pub visibility: Visibility,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(serde::Deserialize)]
pub struct CreateRecording {
    pub artist_id: i32,
    pub composition_id: i32,
    #[serde(default)]
    #[serde(deserialize_with = "empty_string_as_none_i32")]
    pub movement_id: Option<i32>,
    pub content_hash: String,
    pub file_key: String,
    pub mime_type: String,
    pub content_length: i64,
    #[serde(default)]
    pub visibility: Visibility,
}

#[derive(sqlx::FromRow, serde::Serialize, Clone, Debug)]
pub struct RecordingFeedItem {
    pub id: i32,
    pub slug_id: i32,
    pub artist_handle: String,
    pub composer_family_name: String,
    pub composition_id: i32,
    pub composition_title: String,
    pub movement_id: Option<i32>,
    pub movement_index: Option<i32>,
    pub movement_title: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub file_key: String,
    pub mime_type: String,
    pub content_length: i64,
    pub notes: Option<String>,
    pub visibility: Visibility,
}

#[derive(sqlx::FromRow, serde::Serialize, Clone, Debug)]
pub struct SearchResult {
    pub composition_id: i32,
    pub composition_title: String,
    pub movement_id: Option<i32>,
    pub movement_index: Option<i32>,
    pub movement_title: Option<String>,
    pub composer_name: String,
}

pub fn prepare_search_terms(q: &str) -> Vec<String> {
    q.split_whitespace().map(|s| format!("%{}%", s)).collect()
}

pub async fn search_compositions_public(
    pool: &Pool<Postgres>,
    q: &str,
) -> Result<Vec<SearchResult>, sqlx::Error> {
    let search_words = prepare_search_terms(q);
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
        LEFT JOIN movements m ON c.id = m.composition_id
        WHERE 
          unaccent(concat_ws(' ', c.title, m.title, mus.handle, mus.given_name, mus.family_name)) 
          ILIKE ALL(SELECT unaccent(x) FROM unnest($1::text[]) x)
        ORDER BY c.title, m.index
        LIMIT 50
        "#,
    )
    .bind(search_words)
    .fetch_all(pool)
    .await
}

pub async fn search_compositions_admin(
    pool: &Pool<Postgres>,
    q: &str,
    limit: i64,
    offset: i64,
) -> Result<(i64, Vec<Composition>), sqlx::Error> {
    let search_words = prepare_search_terms(q);

    // Count
    let count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(DISTINCT c.id)
        FROM compositions c
        JOIN musicians mus ON c.composer_id = mus.id
        LEFT JOIN movements m ON c.id = m.composition_id
        WHERE 
          unaccent(concat_ws(' ', c.title, m.title, mus.handle, mus.given_name, mus.family_name)) 
          ILIKE ALL(SELECT unaccent(x) FROM unnest($1::text[]) x)
        "#,
    )
    .bind(&search_words)
    .fetch_one(pool)
    .await?;

    // Items
    let compositions = sqlx::query_as::<_, Composition>(
        r#"
        SELECT DISTINCT ON (c.id)
            c.id, 
            c.slug, 
            (mus.family_name || ': ' || c.title) as title, 
            c.publish_date, 
            c.composer_id
        FROM compositions c
        JOIN musicians mus ON c.composer_id = mus.id
        LEFT JOIN movements m ON c.id = m.composition_id
        WHERE 
          unaccent(concat_ws(' ', c.title, m.title, mus.handle, mus.given_name, mus.family_name)) 
          ILIKE ALL(SELECT unaccent(x) FROM unnest($1::text[]) x)
        ORDER BY c.id
        LIMIT $2 OFFSET $3
        "#,
    )
    .bind(&search_words)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok((count, compositions))
}

// Helpers for deserialization
pub fn empty_string_as_none<'de, D, T>(de: D) -> Result<Option<T>, D::Error>
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

pub fn empty_string_as_none_i32<'de, D>(de: D) -> Result<Option<i32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt = Option::<String>::deserialize(de)?;
    match opt.as_deref() {
        None | Some("") => Ok(None),
        Some(s) => s.parse::<i32>().map(Some).map_err(serde::de::Error::custom),
    }
}
