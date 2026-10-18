//! Bounded application-owned native stream endpoints, not a production adapter.
use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{stream, StreamExt};
use hemx::advanced::{
    BuildFingerprint, Effect, EffectBatch, IntoEffect, PatchMode, ResourceId, ResourceKind,
    ResourceRef, SafeHtml, EFFECT_BATCH_ABI_VERSION,
};
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};

const MAX_OPENS: usize = 8;
const MAX_PUSHES: usize = 16;
const CONNECTION_LIMIT: Duration = Duration::from_secs(45);

#[derive(Default)]
struct Case {
    opened: usize,
    closed: usize,
    pushed: usize,
    next: usize,
    live: BTreeMap<usize, mpsc::Sender<Output>>,
}

#[derive(Clone)]
pub struct Streams {
    cases: Arc<Mutex<BTreeMap<String, Case>>>,
    stop: watch::Sender<bool>,
}

impl Streams {
    pub fn new() -> Self {
        Self {
            cases: Arc::default(),
            stop: watch::channel(false).0,
        }
    }

    pub fn routes(&self) -> Router {
        Router::new()
            .route("/streams/{id}/sse", get(sse))
            .route("/streams/burst", get(burst))
            .route("/streams/delta-burst", get(delta_burst))
            .route("/streams/patch-fold", get(patch_fold))
            .route("/streams/{id}/state", get(state))
            .route("/streams/{id}/push/{kind}", post(push))
            .with_state(self.clone())
    }

    /// Wake every live body/socket before the main server's graceful shutdown.
    pub fn shutdown(&self) {
        self.stop.send_replace(true);
    }

    fn connect(&self, id: &str) -> Result<(Lease, mpsc::Receiver<Output>), StatusCode> {
        if !valid_id(id) || *self.stop.borrow() {
            return Err(StatusCode::BAD_REQUEST);
        }
        let mut cases = self.cases.lock().expect("streams lock");
        let case = cases.entry(id.to_owned()).or_default();
        // A source change may open its replacement while the old native close
        // is still in flight. Two leases bound that brief overlap.
        if case.opened >= MAX_OPENS || case.live.len() >= 2 {
            return Err(StatusCode::TOO_MANY_REQUESTS);
        }
        let (sender, receiver) = mpsc::channel(4);
        let serial = case.next;
        case.next += 1;
        case.opened += 1;
        case.live.insert(serial, sender);
        Ok((
            Lease {
                streams: self.clone(),
                id: id.to_owned(),
                serial,
            },
            receiver,
        ))
    }
}

// A fixed inventory bounds both memory and accidental cross-case traffic.
fn valid_id(id: &str) -> bool {
    ["sse"].iter().any(|transport| {
        [
            "delivery",
            "nested",
            "lifecycle",
            "malformed",
            "fingerprint",
            "missing-api",
            "origin",
            "protocol",
            "credentials",
        ]
        .iter()
        .any(|case| id == format!("{transport}-{case}"))
    })
}

struct Lease {
    streams: Streams,
    id: String,
    serial: usize,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut cases = self.streams.cases.lock().expect("streams lock");
        let case = cases.get_mut(&self.id).expect("connected stream case");
        case.live.remove(&self.serial);
        case.closed += 1;
    }
}

#[derive(Clone, Copy)]
enum Output {
    Valid,
    Default,
    Recovery,
    Malformed,
    Fingerprint,
    Text,
}

pub(crate) fn batch(text: &str, fingerprint: u64) -> EffectBatch {
    EffectBatch {
        abi_version: EFFECT_BATCH_ABI_VERSION,
        fingerprint: BuildFingerprint(fingerprint),
        ops: vec![
            Effect::Patch {
                target: ResourceRef::unscoped(ResourceId {
                    kind: ResourceKind::Slot,
                    id: 1,
                }),
                mode: PatchMode::Morph,
                html: SafeHtml::trusted(text),
            },
            Effect::Dispatch {
                event: "stream-proof".into(),
                payload: vec![0, 128, 255, 195, 40],
            },
        ],
    }
}

impl Output {
    fn batch(self) -> EffectBatch {
        batch(
            match self {
                Self::Default => "default",
                Self::Recovery => "recovered",
                _ => "delivered",
            },
            if matches!(self, Self::Fingerprint) {
                9
            } else {
                0
            },
        )
    }

    fn sse(self) -> Event {
        match self {
            Self::Malformed | Self::Text => Event::default().event("hemx").data("AA"),
            Self::Default => Event::default()
                .data(hemx_axum::encode_sse_batch(&self.batch()).expect("canonical SSE")),
            _ => hemx_axum::sse_event(self.batch()).expect("canonical SSE event"),
        }
    }
}

async fn state(
    State(streams): State<Streams>,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    if !valid_id(&id) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let cases = streams.cases.lock().expect("streams lock");
    let empty = Case::default();
    let case = cases.get(&id).unwrap_or(&empty);
    Ok(Json(
        json!({"opened": case.opened, "closed": case.closed, "active": case.live.len(), "pushed": case.pushed}),
    ))
}

async fn push(
    State(streams): State<Streams>,
    Path((id, kind)): Path<(String, String)>,
) -> StatusCode {
    let output = match kind.as_str() {
        "valid" => Output::Valid,
        "default" => Output::Default,
        "recovery" => Output::Recovery,
        "malformed" => Output::Malformed,
        "fingerprint" => Output::Fingerprint,
        "text" => Output::Text,
        _ => return StatusCode::BAD_REQUEST,
    };
    let mut cases = streams.cases.lock().expect("streams lock");
    let Some(case) = cases.get_mut(&id) else {
        return StatusCode::NOT_FOUND;
    };
    if case.pushed >= MAX_PUSHES || case.live.len() != 1 {
        return StatusCode::CONFLICT;
    }
    if case
        .live
        .values()
        .next()
        .expect("one connection")
        .try_send(output)
        .is_err()
    {
        return StatusCode::CONFLICT;
    }
    case.pushed += 1;
    StatusCode::NO_CONTENT
}

/// 200 canonical batches written at once and deliberately not folded on the
/// server (no `hemx_axum::sse`), so the browser must fold them per frame.
/// The connection then stays open: a closed SSE stream would be replayed by
/// `EventSource` reconnection.
async fn burst() -> impl IntoResponse {
    let events: Vec<Result<Event, Infallible>> = (1..=200)
        .map(|n| {
            // A pure declaration: a Dispatch would be a fold barrier.
            let update = EffectBatch {
                abi_version: EFFECT_BATCH_ABI_VERSION,
                fingerprint: BuildFingerprint(0),
                ops: vec![Effect::Patch {
                    target: ResourceRef::unscoped(ResourceId {
                        kind: ResourceKind::Slot,
                        id: 1,
                    }),
                    mode: PatchMode::Morph,
                    html: SafeHtml::trusted(n.to_string()),
                }],
            };
            Ok(hemx_axum::sse_event(update).expect("canonical SSE event"))
        })
        .collect();
    Sse::new(stream::iter(events).chain(stream::pending()))
}

/// 151 unfolded row deltas: append k1..k100, remove the 50 odd keys, then move
/// k100 to the front. The runtime folds what arrives within one frame into one
/// chained delta () and must still reach the exact final order.
/// A replacement then a morph of the same region, sent unfolded. Applied one
/// after the other they end in the replacement element holding "done"; folding
/// them into the morph alone would leave the original element ().
async fn patch_fold() -> impl IntoResponse {
    let slot = ResourceRef::unscoped(ResourceId {
        kind: ResourceKind::Slot,
        id: 1,
    });
    let events = [
        (
            PatchMode::Replace,
            "<section data-hemx-resource=\"1\">first</section>",
        ),
        (PatchMode::Morph, "done"),
    ]
    .map(|(mode, html)| {
        let batch = EffectBatch {
            abi_version: EFFECT_BATCH_ABI_VERSION,
            fingerprint: BuildFingerprint(0),
            ops: vec![Effect::Patch {
                target: slot.clone(),
                mode,
                html: SafeHtml::trusted(html),
            }],
        };
        Ok::<_, Infallible>(hemx_axum::sse_event(batch).expect("canonical SSE event"))
    });
    Sse::new(stream::iter(events).chain(stream::pending()))
}

async fn delta_burst() -> impl IntoResponse {
    // Every state the collection passes through, starting empty.
    let mut states: Vec<Vec<String>> = vec![Vec::new()];
    for n in 1..=100 {
        let mut next = states.last().unwrap().clone();
        next.push(format!("k{n}"));
        states.push(next);
    }
    for n in (1..=99).step_by(2) {
        let gone = format!("k{n}");
        let next = states
            .last()
            .unwrap()
            .iter()
            .filter(|key| **key != gone)
            .cloned()
            .collect();
        states.push(next);
    }
    let mut last: Vec<String> = states.last().unwrap().clone();
    last.retain(|key| key != "k100");
    last.insert(0, "k100".into());
    states.push(last);
    let collection = hemx::advanced::Slot::<()>::new(1);
    let events: Vec<Result<Event, Infallible>> = states
        .windows(2)
        .map(|pair| {
            let added = pair[1].iter().filter(|key| !pair[0].contains(key));
            let rows = added.map(|key| {
                (
                    key.clone(),
                    SafeHtml::trusted(format!("<li data-hemx-key=\"{key}\">{key}</li>")),
                )
            });
            let delta = collection.rows_delta_html(&pair[0], &pair[1], rows);
            Ok(hemx_axum::sse_event(delta.into_batch(BuildFingerprint(0)))
                .expect("canonical SSE event"))
        })
        .collect();
    Sse::new(stream::iter(events).chain(stream::pending()))
}

async fn sse(
    State(streams): State<Streams>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    let (lease, receiver) = streams.connect(&id)?;
    let stop = streams.stop.subscribe();
    let deadline = tokio::time::Instant::now() + CONNECTION_LIMIT;
    let events = stream::unfold(
        (lease, receiver, stop),
        move |(lease, mut receiver, mut stop)| async move {
            if *stop.borrow() {
                return None;
            }
            let output = tokio::select! {
                _ = stop.changed() => None,
                _ = tokio::time::sleep_until(deadline) => None,
                output = receiver.recv() => output,
            };
            output.map(|output| (Ok::<_, Infallible>(output.sse()), (lease, receiver, stop)))
        },
    );
    // Flush an initial protocol comment so native EventSource can announce an
    // idle connection before the test sends its first application event.
    let connected =
        stream::once(async { Ok::<_, Infallible>(Event::default().comment("connected")) });
    Ok(Sse::new(connected.chain(events)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_stream_producers_preserve_canonical_opaque_payload() {
        for output in [
            Output::Valid,
            Output::Default,
            Output::Recovery,
            Output::Fingerprint,
        ] {
            let bytes = output.batch().to_wire().unwrap();
            assert_eq!(EffectBatch::from_wire(&bytes).unwrap(), output.batch());
            assert!(!hemx_axum::encode_sse_batch(&output.batch())
                .unwrap()
                .contains('='));
            assert!(
                matches!(&output.batch().ops[1], Effect::Dispatch { payload, .. } if payload == &[0, 128, 255, 195, 40])
            );
        }
    }

    #[tokio::test]
    async fn push_routes_reject_unknown_cases_kinds_full_queues_and_budgets() {
        let streams = Streams::new();
        assert_eq!(
            state(State(streams.clone()), Path("unknown".into()))
                .await
                .unwrap_err(),
            StatusCode::BAD_REQUEST
        );
        let send =
            |id: &str, kind: &str| push(State(streams.clone()), Path((id.into(), kind.into())));
        assert_eq!(send("unknown", "valid").await, StatusCode::NOT_FOUND);
        assert_eq!(
            send("sse-delivery", "unknown").await,
            StatusCode::BAD_REQUEST
        );
        let (_lease, mut receiver) = streams.connect("sse-delivery").unwrap();
        for _ in 0..4 {
            assert_eq!(send("sse-delivery", "valid").await, StatusCode::NO_CONTENT);
        }
        assert_eq!(send("sse-delivery", "valid").await, StatusCode::CONFLICT);
        for _ in 0..4 {
            receiver.recv().await.unwrap();
        }
        for _ in 4..MAX_PUSHES {
            assert_eq!(send("sse-delivery", "valid").await, StatusCode::NO_CONTENT);
            receiver.recv().await.unwrap();
        }
        assert_eq!(send("sse-delivery", "valid").await, StatusCode::CONFLICT);
        assert_eq!(
            state(State(streams), Path("sse-delivery".into()))
                .await
                .unwrap()
                .0["pushed"],
            MAX_PUSHES
        );
    }

    #[tokio::test]
    async fn controls_are_isolated_bounded_and_shutdown_releases_sse() {
        let streams = Streams::new();
        assert!(streams.connect("unknown").is_err());
        let (lease, mut receiver) = streams.connect("sse-delivery").unwrap();
        let (second, _) = streams.connect("sse-delivery").unwrap();
        assert!(streams.connect("sse-delivery").is_err());
        drop(second);
        assert_eq!(
            push(
                State(streams.clone()),
                Path(("sse-delivery".into(), "valid".into()))
            )
            .await,
            StatusCode::NO_CONTENT
        );
        assert!(matches!(receiver.recv().await, Some(Output::Valid)));
        assert_eq!(
            state(State(streams.clone()), Path("sse-delivery".into()))
                .await
                .unwrap()
                .0["pushed"],
            1
        );
        assert_eq!(
            state(State(streams.clone()), Path("sse-nested".into()))
                .await
                .unwrap()
                .0["opened"],
            0
        );
        drop(lease);
        assert_eq!(
            state(State(streams.clone()), Path("sse-delivery".into()))
                .await
                .unwrap()
                .0["closed"],
            2
        );
        for _ in 2..MAX_OPENS {
            drop(streams.connect("sse-delivery").unwrap());
        }
        assert!(streams.connect("sse-delivery").is_err());
        let response = sse(State(streams.clone()), Path("sse-lifecycle".into()))
            .await
            .unwrap()
            .into_response();
        assert_eq!(response.headers()["content-type"], "text/event-stream");
        streams.shutdown();
        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), b": connected\n\n"); // Protocol greeting only; no application event.
        assert_eq!(
            state(State(streams.clone()), Path("sse-lifecycle".into()))
                .await
                .unwrap()
                .0["active"],
            0
        );
        assert!(streams.connect("sse-nested").is_err());
    }
}
