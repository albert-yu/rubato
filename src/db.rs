use serde::Deserialize;

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
}

#[derive(serde::Deserialize)]
pub struct CreateMusician {
    pub handle: String,
    pub given_name: String,
    pub family_name: String,
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
    pub composition_id: i32,
    pub movement_id: Option<i32>,
    pub content_hash: String,
    pub file_key: String,
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
}

#[derive(sqlx::FromRow, serde::Serialize, Clone, Debug)]
pub struct RecordingFeedItem {
    pub id: i32,
    pub artist_handle: String,
    pub composition_title: String,
    pub movement_index: Option<i32>,
    pub movement_title: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub file_key: String,
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
