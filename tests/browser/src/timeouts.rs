//! Real HTTP deadlines: headers, response bodies, uploads and queued actions.
use crate::checks::case;
use axum::{
    body::Body,
    extract::{Path, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures_util::{stream, StreamExt};
use hemx::{
    advanced::{BuildFingerprint, Slot},
    IntoEffect,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    convert::Infallible,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Default)]
struct Progress {
    requests: usize,
    completed: usize,
    read_bytes: usize,
    body_complete: bool,
}
type StateMap = Arc<Mutex<BTreeMap<String, Progress>>>;

pub fn routes() -> Router {
    Router::new()
        .route("/timeouts/{id}/state", get(state))
        .route("/timeouts/{id}/{kind}", post(request))
        .with_state(StateMap::default())
}
async fn state(State(map): State<StateMap>, Path(id): Path<String>) -> Json<Value> {
    let map = map.lock().unwrap();
    let empty = Progress::default();
    let p = map.get(&id).unwrap_or(&empty);
    Json(
        json!({"requests":p.requests,"completed":p.completed,"readBytes":p.read_bytes,"bodyComplete":p.body_complete}),
    )
}
async fn request(
    State(map): State<StateMap>,
    Path((id, kind)): Path<(String, String)>,
    request: Request,
) -> Response {
    let count = {
        let mut map = map.lock().unwrap();
        let p = map.entry(id.clone()).or_default();
        p.requests += 1;
        p.requests
    };
    if kind == "upload" {
        let mut body = request.into_body().into_data_stream();
        if let Some(Ok(bytes)) = body.next().await {
            map.lock().unwrap().get_mut(&id).unwrap().read_bytes += bytes.len();
        }
        tokio::time::sleep(Duration::from_millis(800)).await;
        while let Some(chunk) = body.next().await {
            let Ok(bytes) = chunk else {
                return StatusCode::BAD_REQUEST.into_response();
            };
            map.lock().unwrap().get_mut(&id).unwrap().read_bytes += bytes.len();
        }
        map.lock().unwrap().get_mut(&id).unwrap().body_complete = true;
    }
    let delay = match kind.as_str() {
        "long" => 11_000,
        "slow" => 800,
        "queue" if count == 1 => 800,
        "queue" => 100,
        _ => 0,
    };
    tokio::time::sleep(Duration::from_millis(delay)).await;
    let bytes = Slot::<String>::new(1)
        .text(format!("complete {count}"))
        .into_batch(BuildFingerprint(0))
        .to_wire()
        .unwrap();
    if kind == "body" {
        let split = bytes.len() - 1;
        let first = bytes[..split].to_vec();
        let last = bytes[split..].to_vec();
        let stream =
            stream::once(async { Ok::<_, Infallible>(first) }).chain(stream::once(async move {
                tokio::time::sleep(Duration::from_millis(800)).await;
                map.lock().unwrap().get_mut(&id).unwrap().completed += 1;
                Ok::<_, Infallible>(last)
            }));
        return (
            [
                ("content-type", "application/hemx"),
                ("x-hemx-fingerprint", "0"),
            ],
            Body::from_stream(stream),
        )
            .into_response();
    }
    map.lock().unwrap().get_mut(&id).unwrap().completed += 1;
    (
        [
            ("content-type", "application/hemx"),
            ("x-hemx-fingerprint", "0"),
        ],
        bytes,
    )
        .into_response()
}

pub fn assert_timeouts(report: &Value, checked: &mut BTreeSet<String>) {
    for name in [
        "long",
        "local",
        "standalone",
        "queue",
        "bare",
        "ms",
        "seconds",
        "whitespace",
        "max",
        "max-seconds",
    ] {
        let observed = case(report, checked, &format!("timeouts-{name}"));
        assert_eq!(observed["errors"], json!([]), "{name}");
        let count = if name == "queue" { 2 } else { 1 };
        assert_eq!(observed["state"]["requests"], count, "{name}");
        assert_eq!(observed["state"]["completed"], count, "{name}");
        assert_eq!(observed["content"], format!("complete {count}"), "{name}");
        assert_eq!(observed["busy"], false, "{name}");
        assert_eq!(observed["disabled"], false, "{name}");
        assert_eq!(observed["value"], "unsaved", "{name}");
        if name == "long" {
            assert!(observed["elapsed"].as_f64().unwrap() >= 10_000.0);
        }
        if name == "queue" {
            assert!(observed["elapsed"].as_f64().unwrap() >= 800.0);
        }
    }
    for name in ["short", "body", "upload"] {
        let observed = case(report, checked, &format!("timeouts-{name}"));
        assert_eq!(observed["errors"].as_array().unwrap().len(), 1, "{name}");
        assert_eq!(
            observed["errors"][0]["code"], "HEMX_REQUEST_FAILED",
            "{name}"
        );
        assert!(
            observed["errors"][0]["message"]
                .as_str()
                .unwrap()
                .contains("250 ms"),
            "{name}"
        );
        assert_eq!(observed["state"]["requests"], 1, "{name}");
        assert_eq!(observed["content"], "initial", "{name}");
        assert_eq!(observed["busy"], false, "{name}");
        assert_eq!(observed["disabled"], false, "{name}");
        assert_eq!(observed["value"], "unsaved", "{name}");
        assert_eq!(
            observed["outlet"], "Zeit abgelaufen; Zustand prüfen",
            "{name}"
        );
        if name == "body" {
            assert_eq!(observed["headersSeen"], true);
        }
        if name == "upload" {
            assert!(observed["state"]["readBytes"].as_u64().unwrap() < 16 * 1024 * 1024);
            assert_eq!(observed["state"]["bodyComplete"], false);
            assert_eq!(observed["fileCount"], 1);
        }
    }
    let invalid = case(report, checked, "timeouts-invalid");
    for row in invalid.as_array().unwrap() {
        assert_eq!(row["state"]["requests"], 0, "{row}");
        assert_eq!(row["errors"].as_array().unwrap().len(), 1, "{row}");
        assert_eq!(row["errors"][0]["code"], "HEMX_REQUEST_FAILED", "{row}");
        assert_eq!(row["busy"], false);
        assert_eq!(row["disabled"], false);
    }
    let observed = case(report, checked, "timeouts-invalid-latest");
    assert_eq!(observed["state"]["requests"], 1);
    assert_eq!(observed["errors"].as_array().unwrap().len(), 1);
    assert_eq!(observed["content"], "complete 1");
    assert_eq!(observed["busy"], false);
}
