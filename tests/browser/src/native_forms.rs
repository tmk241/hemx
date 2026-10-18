//! Application-owned loopback endpoints, deliberately not a generic Hemx API.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::{Path, RawQuery, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use hemx_axum::{EffectResponse, InteractionForm};
use hemx_browser_test::heml;
use serde_json::{json, Value};

#[derive(Default)]
struct Application {
    submissions: u32,
    mutations: u32,
    native: u32,
    enhanced: u32,
    fields: Vec<(String, String)>,
    method: String,
    message: String,
}

type Applications = Arc<Mutex<BTreeMap<String, Application>>>;

/// Each case POSTs reset before submitting. IDs isolate counters across cases;
/// the result and state routes are read-only and never count as submissions.
pub fn routes() -> Router {
    Router::new()
        .route("/native-forms/{id}/reset", post(reset))
        .route("/native-forms/{id}/state", get(state))
        .route("/native-forms/{id}/result", get(result))
        .route(
            "/native-forms/{id}/submit",
            get(submit_get).post(submit_post),
        )
        .route(
            "/native-forms/{id}/override",
            get(submit_get).post(submit_post),
        )
        .with_state(Applications::default())
}

async fn reset(State(apps): State<Applications>, Path(id): Path<String>) -> StatusCode {
    // Fixed, readable per-case identifiers; no cookies, database, or shared reset.
    if id.is_empty() || id.len() > 80 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return StatusCode::BAD_REQUEST;
    }
    apps.lock()
        .expect("native forms lock")
        .insert(id, Application::default());
    StatusCode::NO_CONTENT
}

async fn state(
    State(apps): State<Applications>,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let apps = apps.lock().expect("native forms lock");
    let app = apps.get(&id).ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(json!({
        "submissions": app.submissions, "mutations": app.mutations,
        "native": app.native, "enhanced": app.enhanced,
        "fields": app.fields, "method": app.method, "message": app.message
    })))
}

async fn result(State(apps): State<Applications>, Path(id): Path<String>) -> Response {
    let apps = apps.lock().expect("native forms lock");
    match apps.get(&id) {
        Some(app) => document(&app.message, false).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn submit_get(
    State(apps): State<Applications>,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    submit(
        &apps,
        &id,
        "GET",
        &headers,
        query.unwrap_or_default().as_bytes(),
    )
}

async fn submit_post(
    State(apps): State<Applications>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_none_or(|v| v.split(';').next() != Some("application/x-www-form-urlencoded"))
    {
        return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
    }
    submit(&apps, &id, "POST", &headers, &body)
}

/// The only application operation. Both transports validate here, before the
/// single mutation point. GET is a read-only query, never a mutation endpoint.
fn apply(
    app: &mut Application,
    method: &str,
    fields: Vec<(String, String)>,
    enhanced: bool,
) -> bool {
    app.submissions += 1;
    if enhanced {
        app.enhanced += 1;
    } else {
        app.native += 1;
    }
    app.method = method.to_owned();
    app.fields = fields;
    let values: Vec<_> = app
        .fields
        .iter()
        .filter(|(name, _)| name == "value")
        .collect();
    let valid = values.len() == 1 && !values[0].1.trim().is_empty();
    app.message = if valid {
        if method == "POST" {
            app.mutations += 1;
            "Saved"
        } else {
            "Found"
        }
    } else {
        "Value is required"
    }
    .to_owned();
    valid
}

fn submit(
    apps: &Applications,
    id: &str,
    method: &str,
    headers: &HeaderMap,
    body: &[u8],
) -> Response {
    let Some(fields) = decode(body) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let enhanced = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|part| part.trim() == "application/hemx"));
    let mut apps = apps.lock().expect("native forms lock");
    let Some(app) = apps.get_mut(id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // Both representations call the same validation and mutation exactly once.
    let valid = apply(app, method, fields, enhanced);
    if enhanced {
        return EffectResponse::new(
            heml::grandchild::nested_status.set(&app.message),
            heml::BUILD_FINGERPRINT,
        )
        .into_response();
    }
    if valid && method == "POST" {
        Redirect::to(&format!("/native-forms/{id}/result")).into_response()
    } else {
        (
            if valid {
                StatusCode::OK
            } else {
                StatusCode::UNPROCESSABLE_ENTITY
            },
            document(&app.message, !valid),
        )
            .into_response()
    }
}

fn document(message: &str, invalid: bool) -> Html<String> {
    // All messages are constants owned by apply, never unescaped user input.
    let error = if invalid {
        " aria-invalid=\"true\" aria-describedby=\"native-status\""
    } else {
        ""
    };
    Html(format!("<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>Native form result</title><body><main data-hemx-root data-hemx-build=\"{}\"><p id=\"native-status\">{message}</p><label>Value<input name=\"value\"{error}></label></main></body></html>", heml::BUILD_FINGERPRINT.0))
}

// Exercise the public decoder, not a second URL-encoding implementation.
fn decode(body: &[u8]) -> Option<Vec<(String, String)>> {
    InteractionForm::parse_urlencoded(body)
        .ok()
        .map(|form| form.fields().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hemx::advanced::EffectBatch;

    #[test]
    fn both_transports_share_validation_and_one_mutation() {
        for enhanced in [false, true] {
            let mut app = Application::default();
            assert!(!apply(
                &mut app,
                "POST",
                vec![("value".into(), " ".into())],
                enhanced
            ));
            assert_eq!(app.mutations, 0);
            assert!(apply(
                &mut app,
                "POST",
                vec![("value".into(), "ok".into())],
                enhanced
            ));
            assert_eq!(app.mutations, 1);
            assert!(apply(
                &mut app,
                "GET",
                vec![("value".into(), "ok".into())],
                enhanced
            ));
            assert_eq!(app.mutations, 1);
            assert_eq!(app.submissions, 3);
        }
    }

    // Exercise real TCP/Axum extraction and representations without a browser.
    // Browser navigation and runtime admission are asserted separately in JS.
    #[tokio::test]
    async fn loopback_native_and_enhanced_share_the_operation() {
        use std::io::{Read, Write};
        use std::net::TcpStream;
        use std::time::Duration;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, routes()).await.unwrap() });
        let request = |method: &str, path: &str, accept: &str, body: &str| {
            let request = format!("{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nAccept: {accept}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{body}", body.len());
            tokio::task::spawn_blocking(move || {
                let mut stream =
                    TcpStream::connect_timeout(&address, Duration::from_secs(2)).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                stream.write_all(request.as_bytes()).unwrap();
                let mut response = Vec::new();
                stream.read_to_end(&mut response).unwrap();
                let split = response.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
                (
                    String::from_utf8(response[..split].to_vec()).unwrap(),
                    response[split + 4..].to_vec(),
                )
            })
        };
        for (id, method, value, accept, status, mutations) in [
            ("native-post", "POST", "ok", "text/html", "303 See Other", 1),
            (
                "native-invalid",
                "POST",
                "",
                "text/html",
                "422 Unprocessable Entity",
                0,
            ),
            ("native-get", "GET", "ok", "text/html", "200 OK", 0),
            (
                "native-get-invalid",
                "GET",
                "",
                "text/html",
                "422 Unprocessable Entity",
                0,
            ),
            (
                "enhanced-post",
                "POST",
                "ok",
                "application/hemx",
                "200 OK",
                1,
            ),
            (
                "enhanced-invalid",
                "POST",
                "",
                "application/hemx",
                "200 OK",
                0,
            ),
        ] {
            let prefix = format!("/native-forms/{id}");
            let (headers, _) = request("POST", &format!("{prefix}/reset"), "text/html", "")
                .await
                .unwrap();
            assert!(headers.starts_with("HTTP/1.1 204"));
            let body = format!("__h=7&value={value}");
            let path = if method == "GET" {
                format!("{prefix}/submit?{body}")
            } else {
                format!("{prefix}/submit")
            };
            let (headers, bytes) = request(
                method,
                &path,
                accept,
                if method == "GET" { "" } else { &body },
            )
            .await
            .unwrap();
            assert!(
                headers.starts_with(&format!("HTTP/1.1 {status}")),
                "{headers}"
            );
            if accept == "application/hemx" {
                assert!(headers.contains("content-type: application/hemx"));
                let batch = EffectBatch::from_wire(&bytes).unwrap();
                assert_eq!(batch.ops.len(), 1);
            } else if status.starts_with("303") {
                assert!(headers.contains(&format!("location: {prefix}/result")));
                let (headers, bytes) = request("GET", &format!("{prefix}/result"), "text/html", "")
                    .await
                    .unwrap();
                assert!(headers.contains("content-type: text/html"));
                assert!(String::from_utf8(bytes).unwrap().contains(">Saved</p>"));
            } else {
                assert!(headers.contains("content-type: text/html"));
                assert!(String::from_utf8(bytes)
                    .unwrap()
                    .starts_with("<!doctype html>"));
            }
            let (_, bytes) = request("GET", &format!("{prefix}/state"), "application/json", "")
                .await
                .unwrap();
            let snapshot: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(snapshot["submissions"], 1);
            assert_eq!(snapshot["mutations"], mutations);
        }
        server.abort();
        let _ = server.await;
    }

    #[test]
    fn urlencoded_preserves_duplicates_and_rejects_bad_input() {
        assert_eq!(
            decode(b"__h=7&value=a+b&value=%C3%A9").unwrap(),
            vec![
                ("__h".into(), "7".into()),
                ("value".into(), "a b".into()),
                ("value".into(), "é".into())
            ]
        );
        for body in [
            b"__h=7&value=%".as_slice(),
            b"__h=7&value=%ZZ",
            b"__h=7&value=%FF",
        ] {
            assert!(decode(body).is_none());
        }
    }
}
