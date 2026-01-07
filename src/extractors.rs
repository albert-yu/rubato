use axum::{
    extract::{FromRef, FromRequestParts},
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use axum_extra::extract::cookie::CookieJar;
use jsonwebtoken::{DecodingKey, Validation, decode};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Postgres};

use crate::db::User;
use crate::view::{HtmlTemplate, NotFoundTemplate};

pub struct AuthUser(pub User);

pub struct OptionalAuthUser(pub Option<User>);

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
}

pub struct HtmxRequest {
    pub is_hx_boosted: bool,
}

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
