use argon2::password_hash::{PasswordHasher, SaltString, rand_core::OsRng};
use argon2::{Argon2, PasswordHash, PasswordVerifier};
use axum::{
    extract::{Form, State},
    response::{IntoResponse, Redirect},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use jsonwebtoken::{EncodingKey, Header, encode};
use serde::Deserialize;
use sqlx::{Pool, Postgres};

use crate::db::{User, UserRole};
use crate::extractors::{Claims, OptionalAuthUser};
use crate::view::{HtmlTemplate, LoginTemplate, SignupTemplate};

#[derive(Deserialize)]
pub struct LoginPayload {
    identity: String,
    password: String,
}

#[derive(Deserialize)]
pub struct SignupPayload {
    email: String,
    handle: String,
    password: String,
}

pub async fn login_form(auth: OptionalAuthUser) -> impl IntoResponse {
    HtmlTemplate(LoginTemplate {
        current_user: auth.0,
        error: None,
    })
}

pub async fn signup_form(auth: OptionalAuthUser) -> impl IntoResponse {
    HtmlTemplate(SignupTemplate {
        current_user: auth.0,
        error: None,
    })
}

pub async fn signup_post(
    auth: OptionalAuthUser,
    State(pool): State<Pool<Postgres>>,
    State(encoding_key): State<EncodingKey>,
    jar: CookieJar,
    Form(payload): Form<SignupPayload>,
) -> impl IntoResponse {
    // Validate handle format
    let handle = &payload.handle;
    let is_valid_format = handle.chars().all(|c| c.is_alphanumeric() || c == '-')
        && !handle.starts_with('-')
        && !handle.ends_with('-')
        && !handle.is_empty();

    if !is_valid_format {
        return (
            jar,
            HtmlTemplate(SignupTemplate {
                current_user: auth.0,
                error: Some("Username must contain only alphanumeric characters or hyphens, and cannot start or end with a hyphen.".to_string()),
            }),
        )
            .into_response();
    }

    // Check if email already exists
    let email_exists = sqlx::query!("SELECT id FROM users WHERE email = $1", payload.email)
        .fetch_optional(&pool)
        .await
        .unwrap_or(None)
        .is_some();

    if email_exists {
        return (
            jar,
            HtmlTemplate(SignupTemplate {
                current_user: auth.0,
                error: Some("Email already taken".to_string()),
            }),
        )
            .into_response();
    }

    // Check if handle already exists
    let handle_exists = sqlx::query!("SELECT id FROM musicians WHERE handle = $1", payload.handle)
        .fetch_optional(&pool)
        .await
        .unwrap_or(None)
        .is_some();

    if handle_exists {
        return (
            jar,
            HtmlTemplate(SignupTemplate {
                current_user: auth.0,
                error: Some("Handle already taken".to_string()),
            }),
        )
            .into_response();
    }

    // Create Musician
    let musician_id = sqlx::query!(
        "INSERT INTO musicians (handle, given_name, family_name) VALUES ($1, '', '') RETURNING id",
        payload.handle
    )
    .fetch_one(&pool)
    .await;

    let musician_id = match musician_id {
        Ok(record) => record.id,
        Err(_) => {
            return (
                jar,
                HtmlTemplate(SignupTemplate {
                    current_user: auth.0,
                    error: Some("Failed to create user (musician)".to_string()),
                }),
            )
                .into_response();
        }
    };

    // Hash password
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let password_hash = match argon2.hash_password(payload.password.as_bytes(), &salt) {
        Ok(hash) => hash.to_string(),
        Err(_) => {
            return (
                jar,
                HtmlTemplate(SignupTemplate {
                    current_user: auth.0,
                    error: Some("Failed to hash password".to_string()),
                }),
            )
                .into_response();
        }
    };

    // Create User
    let user_id = sqlx::query!(
        "INSERT INTO users (email, password_hash, salt, musician_id, role) VALUES ($1, $2, $3, $4, $5) RETURNING id",
        payload.email,
        password_hash,
        salt.as_str(),
        musician_id,
        UserRole::User as UserRole
    )
    .fetch_one(&pool)
    .await;

    let user_id = match user_id {
        Ok(record) => record.id,
        Err(_) => {
            return (
                jar,
                HtmlTemplate(SignupTemplate {
                    current_user: auth.0,
                    error: Some("Failed to create user".to_string()),
                }),
            )
                .into_response();
        }
    };

    // Login (create token)
    let claims = Claims {
        sub: user_id.to_string(),
        exp: (chrono::Utc::now() + chrono::Duration::hours(24)).timestamp() as usize,
    };

    if let Ok(token) = encode(&Header::default(), &claims, &encoding_key) {
        let mut cookie = Cookie::new("auth_token", token);
        cookie.set_http_only(true);
        cookie.set_same_site(SameSite::Lax);
        cookie.set_path("/");
        return (jar.add(cookie), Redirect::to("/")).into_response();
    }

    (
        jar,
        HtmlTemplate(SignupTemplate {
            current_user: auth.0,
            error: Some("Failed to login after signup".to_string()),
        }),
    )
        .into_response()
}

pub async fn login_post(
    auth: OptionalAuthUser,
    State(pool): State<Pool<Postgres>>,
    State(encoding_key): State<EncodingKey>,
    jar: CookieJar,
    Form(payload): Form<LoginPayload>,
) -> impl IntoResponse {
    let user = sqlx::query_as::<_, User>(
        "SELECT u.* FROM users u JOIN musicians m ON u.musician_id = m.id WHERE u.email = $1 OR m.handle = $1"
    )
        .bind(&payload.identity)
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
                return (jar.add(cookie), Redirect::to("/")).into_response();
            }
        }
    }

    (
        jar,
        HtmlTemplate(LoginTemplate {
            current_user: auth.0,
            error: Some("Invalid identity or password".to_string()),
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
