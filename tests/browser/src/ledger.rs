//! Real HTTP resends: one ledger-backed retry of a lost or late response.
use crate::checks::case;
use axum::{
    extract::{Path, Request, State},
    middleware::{self, Next},
    response::Response,
    routing::get,
    Json, Router,
};
use hemx::{
    advanced::{BuildFingerprint, Handle, Slot},
    Html,
};
use hemx_axum::{
    action, action_with_ledger, interactions, InteractionForm, LedgerLimits, MemoryLedger, Native,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Default)]
struct Progress {
    requests: usize,
    runs: usize,
    tokens: Vec<Option<String>>,
}
type StateMap = Arc<Mutex<BTreeMap<String, Progress>>>;

const SAVE: Handle<()> = Handle::new(7);

fn feedback(_: &InteractionForm, problem: &str) -> Html {
    Html::__rendered(format!("<p>{problem}</p>"), BuildFingerprint(0))
}

/// The handler counts its runs per case; the `running` case's real
/// submission takes 800 ms, longer than the 250 ms browser deadline.
fn inner(map: StateMap) -> Router {
    let registry = || {
        let map = Arc::clone(&map);
        interactions(BuildFingerprint(0)).on_async(SAVE, move |form| {
            let map = Arc::clone(&map);
            async move {
                let case = form.value("case").unwrap_or_default().to_owned();
                let runs = {
                    let mut map = map.lock().unwrap();
                    let progress = map.entry(case.clone()).or_default();
                    progress.runs += 1;
                    progress.runs
                };
                if case == "running" && runs == 2 {
                    tokio::time::sleep(Duration::from_millis(800)).await;
                }
                Slot::<String>::new(1).text(format!("saved {runs}"))
            }
        })
    };
    let limits = LedgerLimits {
        entries: 64,
        bytes: 1 << 20,
        age: Duration::from_secs(60),
    };
    Router::new()
        .route(
            "/ledger/kept/{id}",
            action_with_ledger(
                registry(),
                Native::new("/", feedback),
                MemoryLedger::new(limits),
            ),
        )
        .route(
            "/ledger/plain/{id}",
            action(registry(), Native::new("/", feedback)),
        )
}

pub fn routes() -> Router {
    let map = StateMap::default();
    let state_routes = Router::new()
        .route("/ledger/{mode}/{id}/state", get(state))
        .with_state(Arc::clone(&map));
    inner(Arc::clone(&map))
        .route_layer(middleware::from_fn_with_state(map, forward))
        .merge(state_routes)
}

async fn state(State(map): State<StateMap>, Path((_, id)): Path<(String, String)>) -> Json<Value> {
    let map = map.lock().unwrap();
    let empty = Progress::default();
    let progress = map.get(&id).unwrap_or(&empty);
    Json(json!({"requests": progress.requests, "runs": progress.runs, "tokens": progress.tokens}))
}

/// Count each request and its token, then let the real action answer. The
/// `lost` and `none` cases hold their real submission's completed response
/// past the browser deadline, as a slow network would.
async fn forward(
    State(map): State<StateMap>,
    Path(id): Path<String>,
    request: Request,
    next: Next,
) -> Response {
    let count = {
        let mut map = map.lock().unwrap();
        let progress = map.entry(id.clone()).or_default();
        progress.requests += 1;
        progress.tokens.push(
            request
                .headers()
                .get(hemx_axum::HEMX_TOKEN_HEADER)
                .and_then(|token| token.to_str().ok())
                .map(ToOwned::to_owned),
        );
        progress.requests
    };
    let (parts, body) = next.run(request).await.into_parts();
    let body = axum::body::to_bytes(body, 1 << 20).await.unwrap();
    if matches!(id.as_str(), "lost" | "none") && count == 2 {
        tokio::time::sleep(Duration::from_millis(800)).await;
    }
    Response::from_parts(parts, body.into())
}

pub fn assert_ledger(report: &Value, checked: &mut BTreeSet<String>) {
    let lost = case(report, checked, "ledger-lost");
    // The submission's first response was lost; its one resend got the recorded
    // result, so the handler ran once for it and the page shows that result.
    assert_eq!(lost["state"]["requests"], 3);
    assert_eq!(lost["state"]["runs"], 2);
    assert_eq!(lost["content"], "saved 2");
    let tokens = lost["state"]["tokens"].as_array().unwrap();
    assert!(tokens
        .iter()
        .all(|token| token.as_str().is_some_and(|token| token.len() == 32)));
    assert_ne!(tokens[0], tokens[1]);
    assert_eq!(tokens[1], tokens[2]);
    assert_eq!(lost["errors"].as_array().unwrap().len(), 1);
    assert_eq!(lost["errors"][0]["code"], "HEMX_REQUEST_FAILED");
    assert_eq!(lost["outlet"], ""); // the recovered result cleared the failure
    assert_eq!(lost["busy"], false);
    assert_eq!(lost["disabled"], false);
    assert_eq!(lost["value"], "unsaved");

    let running = case(report, checked, "ledger-running");
    // The resend arrived while the first request still ran: refused, not rerun.
    assert_eq!(running["state"]["requests"], 3);
    assert_eq!(running["state"]["runs"], 2);
    let codes: Vec<_> = running["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|error| error["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["HEMX_REQUEST_FAILED", "HEMX_SUBMISSION_PENDING"]);
    let tokens = running["state"]["tokens"].as_array().unwrap();
    assert_eq!(tokens[1], tokens[2]);
    assert_eq!(running["content"], "saved 1");
    assert_eq!(running["busy"], false);

    let none = case(report, checked, "ledger-none");
    // Without a ledger the lost response stays a reported failure: no resend.
    assert_eq!(none["state"]["requests"], 2);
    assert_eq!(none["state"]["runs"], 2);
    assert_eq!(none["errors"].as_array().unwrap().len(), 1);
    assert_eq!(none["errors"][0]["code"], "HEMX_REQUEST_FAILED");
    assert_eq!(none["content"], "saved 1");
    assert_eq!(none["busy"], false);
    assert_eq!(none["disabled"], false);
}
