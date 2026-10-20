//! Real loopback failures; counters expose accidental HTTP replay.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use hemx_browser_test::error_cases;
use serde_json::{json, Value};

use crate::checks::case;

type Counts = Arc<Mutex<BTreeMap<String, u32>>>;

pub fn routes() -> Router {
    Router::new()
        .route("/errors/{id}/state", get(state))
        .route("/errors/{id}/{kind}", get(failure).post(failure))
        .with_state(Counts::default())
}

async fn state(State(counts): State<Counts>, Path(id): Path<String>) -> Json<u32> {
    Json(*counts.lock().unwrap().get(&id).unwrap_or(&0))
}

async fn failure(
    State(counts): State<Counts>,
    Path((id, kind)): Path<(String, String)>,
) -> Response {
    let count = {
        let mut counts = counts.lock().unwrap();
        let count = counts.entry(id).or_default();
        *count += 1;
        *count
    };
    if kind == "timeout" {
        // Longer than the runtime's real ten-second request deadline.
        tokio::time::sleep(std::time::Duration::from_secs(11)).await;
        return StatusCode::NO_CONTENT.into_response();
    }
    if kind == "sse" {
        if count > 1 {
            return StatusCode::NO_CONTENT.into_response();
        }
        let mut body = String::new();
        for batch in [error_cases::failed_batch(), error_cases::recovery_batch()] {
            body.push_str(&format!(
                "event: hemx\ndata: {}\n\n",
                hemx_axum::encode_sse_batch(&batch).unwrap()
            ));
        }
        return ([("content-type", "text/event-stream")], body).into_response();
    }
    let (status, fingerprint, body) = match kind.as_str() {
        "batch" => (
            StatusCode::OK,
            "0",
            error_cases::failed_batch().to_wire().unwrap(),
        ),
        "decode" => (StatusCode::OK, "0", vec![0]),
        "http" => (StatusCode::SERVICE_UNAVAILABLE, "0", vec![]),
        "stale" => (StatusCode::CONFLICT, "1", vec![]),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    (
        status,
        [
            ("content-type", "application/hemx"),
            ("x-hemx-fingerprint", fingerprint),
        ],
        body,
    )
        .into_response()
}

pub fn assert_errors(report: &Value, checked: &mut BTreeSet<String>) {
    for (transport, kind, code, status, default) in [
        (
            "post",
            "batch",
            "HEMX_BATCH_FAILED",
            Value::Null,
            "Request failed",
        ),
        (
            "post",
            "decode",
            "HEMX_BATCH_FAILED",
            Value::Null,
            "Request failed",
        ),
        (
            "post",
            "http",
            "HEMX_REQUEST_FAILED",
            json!(503),
            "Request failed (503)",
        ),
        (
            "post",
            "timeout",
            "HEMX_REQUEST_FAILED",
            Value::Null,
            "Request failed",
        ),
        (
            "post",
            "stale",
            "HEMX_STALE_BUILD",
            json!(409),
            "This page is out of date. Reload to continue.",
        ),
        (
            "navigate",
            "batch",
            "HEMX_BATCH_FAILED",
            Value::Null,
            "Request failed",
        ),
        (
            "public",
            "batch",
            "HEMX_BATCH_FAILED",
            Value::Null,
            "Request failed",
        ),
        (
            "public",
            "decode",
            "HEMX_BATCH_FAILED",
            Value::Null,
            "Request failed",
        ),
        (
            "public",
            "abi",
            "HEMX_UNSUPPORTED_ABI",
            Value::Null,
            "Request failed",
        ),
        (
            "public",
            "stale",
            "HEMX_STALE_BUILD",
            Value::Null,
            "This page is out of date. Reload to continue.",
        ),
        (
            "sse",
            "batch",
            "HEMX_BATCH_FAILED",
            Value::Null,
            "Request failed",
        ),
    ] {
        let name = format!("errors-{transport}-{kind}");
        let observed = case(report, checked, &name);
        assert_eq!(observed["events"].as_array().unwrap().len(), 1, "{name}");
        let event = &observed["events"][0];
        assert_eq!(event["owner"], "owner", "{name}");
        assert_eq!(event["code"], code, "{name}");
        assert_eq!(event["status"], status, "{name}");
        assert_eq!(event["defaultText"], default, "{name}");
        assert_eq!(event["hidden"], false, "{name}");
        assert!(!event["message"].as_str().unwrap().is_empty(), "{name}");
        if kind == "timeout" {
            assert!(
                event["message"].as_str().unwrap().contains("TimeoutError"),
                "{name}"
            );
            assert!(
                event["message"].as_str().unwrap().contains("10000 ms"),
                "{name}"
            );
        }
        assert_eq!(
            observed["text"], "Fehler <b>nicht wiederholen</b>",
            "{name}"
        );
        assert_eq!(
            observed["markupChildren"], 0,
            "{name}: translation stays text"
        );
        assert_eq!(
            observed["foreign"],
            json!(["nested", "island", "sibling"]),
            "{name}"
        );
        assert_eq!(observed["foreignEvents"], 0, "{name}");
        assert_eq!(
            observed["content"],
            if kind == "batch" {
                "before failure"
            } else {
                "initial"
            },
            "{name}"
        );
        if transport == "public" {
            assert_eq!(observed["accepted"], false, "{name}");
            assert_eq!(observed["nextAccepted"], true, "{name}");
        } else {
            assert_eq!(observed["requests"], 1, "{name}: never replay");
            assert_eq!(observed["sameDocument"], true, "{name}");
            assert_eq!(observed["historyDelta"], 0, "{name}");
        }
        if transport == "sse" || transport == "public" {
            assert_eq!(
                observed["completed"], true,
                "{name}: next independent batch survives"
            );
        }
    }
}
