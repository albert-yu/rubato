use crate::db::*;
use askama::Template;
use axum::http::StatusCode;
use axum::response::IntoResponse;

#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate {
    pub current_user: Option<User>,
    pub recordings: Vec<RecordingFeedItem>,
}

#[derive(Template)]
#[template(path = "index_content.html")]
pub struct IndexContentTemplate {
    pub current_user: Option<User>,
    pub recordings: Vec<RecordingFeedItem>,
}

#[derive(Template)]
#[template(path = "settings.html")]
pub struct SettingsTemplate {
    pub current_user: Option<User>,
    pub musician: Musician,
}

#[derive(Template)]
#[template(path = "settings_content.html")]
pub struct SettingsContentTemplate {
    pub current_user: Option<User>,
    pub musician: Musician,
}

#[derive(Template)]
#[template(path = "upload.html")]
pub struct UploadTemplate {
    pub current_user: Option<User>,
    pub error: Option<String>,
    pub compositions: Vec<Composition>,
}

#[derive(Template)]
#[template(path = "composition_picker.html")]
pub struct CompositionPickerTemplate;

#[derive(Template)]
#[template(path = "player.html")]
pub struct PlayerTemplate {
    pub recording: RecordingFeedItem,
}

#[derive(Template)]
#[template(path = "admin/index.html")]
pub struct AdminIndexTemplate {
    pub current_user: Option<User>,
}

#[derive(Template)]
#[template(path = "admin/musicians.html")]
pub struct AdminMusiciansTemplate {
    pub musicians: Vec<Musician>,
    pub current_user: Option<User>,
    pub page: i64,
    pub total_pages: i64,
}

#[derive(Template)]
#[template(path = "admin/musician_edit.html")]
pub struct AdminMusicianEditTemplate {
    pub musician: Musician,
    pub current_user: Option<User>,
}

#[derive(Template)]
#[template(path = "admin/compositions.html")]
pub struct AdminCompositionsTemplate {
    pub compositions: Vec<Composition>,
    pub current_user: Option<User>,
    pub page: i64,
    pub total_pages: i64,
}

#[derive(Template)]
#[template(path = "admin/composition_edit.html")]
pub struct AdminCompositionEditTemplate {
    pub composition: Composition,
    pub movements: Vec<Movement>,
    pub musicians: Vec<Musician>,
    pub current_user: Option<User>,
}

#[derive(Template)]
#[template(path = "admin/recordings.html")]
pub struct AdminRecordingsTemplate {
    pub recordings: Vec<Recording>,
    pub current_user: Option<User>,
    pub page: i64,
    pub total_pages: i64,
}

#[derive(Template)]
#[template(path = "admin/import.html")]
pub struct AdminImportTemplate {
    pub job: Option<ImportJob>,
    pub current_user: Option<User>,
}

#[derive(Template)]
#[template(path = "admin/recording_edit.html")]
pub struct AdminRecordingEditTemplate {
    pub recording: Recording,
    pub musicians: Vec<Musician>,
    pub compositions: Vec<Composition>,
    pub movements: Vec<Movement>,
    pub current_user: Option<User>,
}

#[derive(Template)]
#[template(path = "login.html")]
pub struct LoginTemplate {
    pub current_user: Option<User>,
    pub error: Option<String>,
}

#[derive(Template)]
#[template(path = "404.html")]
pub struct NotFoundTemplate;

pub struct HtmlTemplate<T>(pub T);

impl<T> IntoResponse for HtmlTemplate<T>
where
    T: Template,
{
    fn into_response(self) -> axum::response::Response {
        match self.0.render() {
            Ok(html) => axum::response::Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}
