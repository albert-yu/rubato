use axum::{
    body::Body,
    extract::Request,
    http::{StatusCode, header},
    middleware::Next,
    response::Response,
};

fn compute_fnv1a(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in text.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub async fn etag_middleware(req: Request, next: Next) -> Response {
    let if_none_match = req.headers().get(header::IF_NONE_MATCH).cloned();

    let mut response = next.run(req).await;

    if !response.status().is_success() {
        return response;
    }

    // Always add Cache-Control: no-cache as requested
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-cache"),
    );

    if response.headers().contains_key(header::ETAG) {
        return response;
    }

    let last_modified = response.headers().get(header::LAST_MODIFIED).cloned();
    let content_length = response.headers().get(header::CONTENT_LENGTH).cloned();

    if let (Some(last_modified), Some(content_length)) = (last_modified, content_length) {
        let last_modified_str = last_modified.to_str().unwrap_or_default();
        let content_length_str = content_length.to_str().unwrap_or_default();

        let hash = compute_fnv1a(last_modified_str);
        let etag = format!(
            "\"{:x}-{:x}\"",
            content_length_str.parse::<u64>().unwrap_or(0),
            hash
        );

        if let Some(if_none_match) = if_none_match {
            if let Ok(if_none_match_str) = if_none_match.to_str() {
                if if_none_match_str == "*" || if_none_match_str.contains(&etag) {
                    let (mut parts, _) = response.into_parts();
                    parts.status = StatusCode::NOT_MODIFIED;
                    parts.headers.insert(header::ETAG, etag.parse().unwrap());
                    return Response::from_parts(parts, Body::empty());
                }
            }
        }

        response
            .headers_mut()
            .insert(header::ETAG, etag.parse().unwrap());
    }

    response
}
