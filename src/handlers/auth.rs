use argon2::{Argon2, PasswordHash, PasswordVerifier};
use axum::{
    extract::{Form, State},
    response::{IntoResponse, Redirect},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use jsonwebtoken::{EncodingKey, Header, encode};
use serde::Deserialize;
use sqlx::{Pool, Postgres};

use crate::db::User;
use crate::extractors::{Claims, OptionalAuthUser};
use crate::view::{HtmlTemplate, LoginTemplate};

#[derive(Deserialize)]
pub struct LoginPayload {
    email: String,
    password: String,
}

pub async fn login_form(auth: OptionalAuthUser) -> impl IntoResponse {
    HtmlTemplate(LoginTemplate {
        current_user: auth.0,
        error: None,
    })
}

pub async fn login_post(
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

pub async fn logout(jar: CookieJar) -> impl IntoResponse {
    (
        jar.remove(Cookie::from("auth_token")),
        Redirect::to("/login"),
    )
}
