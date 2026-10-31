use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use axum::response::Response;
use axum::Router;
use hemx_axum::{
    action, action_with_ledger, interactions, Claim, Ledger, LedgerLimits, MemoryLedger, Native,
    Recorded, Token, HEMX_LEDGER_HEADER, HEMX_TOKEN_HEADER,
};
use hemx_core::{BuildFingerprint, Handle, Html};
use tokio::sync::Notify;
use tower::ServiceExt;

const BUILD: BuildFingerprint = BuildFingerprint(9);
const SAVE: Handle<()> = Handle::new(7);
const OTHER: Handle<()> = Handle::new(8);
const TOKEN: &str = "0123456789abcdef0123456789abcdef";

fn feedback(_: &hemx_axum::InteractionForm, problem: &str) -> Html {
    Html::__rendered(format!("<p>{problem}</p>"), BUILD)
}

fn limits() -> LedgerLimits {
    LedgerLimits {
        entries: 64,
        bytes: 1 << 20,
        age: Duration::from_secs(600),
    }
}

/// Both actions count their runs; `save` waits for `gate` when one is given.
fn app(ledger: Option<MemoryLedger>, runs: Arc<AtomicUsize>, gate: Option<Arc<Notify>>) -> Router {
    let other_runs = Arc::clone(&runs);
    let registry = interactions(BUILD)
        .on_async(SAVE, move |_| {
            let runs = Arc::clone(&runs);
            let gate = gate.clone();
            async move {
                runs.fetch_add(1, Ordering::SeqCst);
                if let Some(gate) = gate {
                    gate.notified().await;
                }
            }
        })
        .on(OTHER, move |_| {
            other_runs.fetch_add(1, Ordering::SeqCst);
        });
    let native = Native::new("/", feedback);
    let route = match ledger {
        Some(ledger) => action_with_ledger(registry, native, ledger),
        None => action(registry, native),
    };
    Router::new().route("/save", route)
}

fn post(handle: u32, token: Option<&str>, enhanced: bool) -> Request<Body> {
    let mut request = Request::post("/save")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(
            header::ACCEPT,
            if enhanced {
                "application/hemx, text/html"
            } else {
                "text/html"
            },
        );
    if let Some(token) = token {
        request = request.header(HEMX_TOKEN_HEADER, token);
    }
    request
        .body(Body::from(format!("__h={handle}&__b=9")))
        .unwrap()
}

async fn parts(response: Response) -> (StatusCode, Option<String>, Vec<u8>) {
    let status = response.status();
    let ledger = response
        .headers()
        .get(HEMX_LEDGER_HEADER)
        .map(|value| value.to_str().unwrap().to_owned());
    let body = to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap()
        .to_vec();
    (status, ledger, body)
}

#[tokio::test]
async fn recorded_read_returns_body_errors_instead_of_empty_success() {
    let body = Body::from_stream(futures_util::stream::iter([
        Ok(axum::body::Bytes::from_static(b"partial response")),
        Err(std::io::Error::other("broken response body")),
    ]));
    let result = Recorded::read(Response::new(body)).await;
    let error = result.expect_err("incomplete responses must not be recorded as success");
    assert!(error.to_string().contains("broken response body"));
}

#[tokio::test]
async fn recorded_read_preserves_the_complete_response() {
    let response = axum::response::IntoResponse::into_response((StatusCode::CREATED, "saved"));
    let recorded = Recorded::read(response).await.unwrap();
    assert_eq!(recorded.status, StatusCode::CREATED);
    assert_eq!(recorded.body, "saved");
    assert!(recorded
        .headers
        .iter()
        .any(|(name, value)| name == header::CONTENT_TYPE && value == "text/plain; charset=utf-8"));
}

#[tokio::test]
async fn a_resent_token_returns_its_first_response_without_running_again() {
    let runs = Arc::new(AtomicUsize::new(0));
    let app = app(Some(MemoryLedger::new(limits())), Arc::clone(&runs), None);
    let first = parts(
        app.clone()
            .oneshot(post(7, Some(TOKEN), true))
            .await
            .unwrap(),
    )
    .await;
    let again = parts(
        app.clone()
            .oneshot(post(7, Some(TOKEN), true))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(first.0, StatusCode::OK);
    assert_eq!(first.1.as_deref(), Some("1")); // the browser may resend this action
    assert_eq!(again, first);
    assert_eq!(runs.load(Ordering::SeqCst), 1);

    // A fresh token is a new submission.
    let other = "fedcba9876543210fedcba9876543210";
    let _ = app.oneshot(post(7, Some(other), true)).await.unwrap();
    assert_eq!(runs.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_token_still_running_is_refused_and_the_first_request_finishes_once() {
    let runs = Arc::new(AtomicUsize::new(0));
    let gate = Arc::new(Notify::new());
    let app = app(
        Some(MemoryLedger::new(limits())),
        Arc::clone(&runs),
        Some(Arc::clone(&gate)),
    );
    let first = tokio::spawn(app.clone().oneshot(post(7, Some(TOKEN), true)));
    while runs.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    let (status, ledger, body) = parts(
        app.clone()
            .oneshot(post(7, Some(TOKEN), true))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_EARLY);
    assert_eq!(ledger.as_deref(), Some("1"));
    assert_eq!(body, b"this submission is still being processed");
    gate.notify_one();
    assert_eq!(first.await.unwrap().unwrap().status(), StatusCode::OK);
    assert_eq!(runs.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_dropped_first_request_still_completes_and_is_recorded() {
    let runs = Arc::new(AtomicUsize::new(0));
    let gate = Arc::new(Notify::new());
    let app = app(
        Some(MemoryLedger::new(limits())),
        Arc::clone(&runs),
        Some(Arc::clone(&gate)),
    );
    // The client gives up (like a browser deadline) while the handler runs.
    let first = tokio::spawn(app.clone().oneshot(post(7, Some(TOKEN), true)));
    while runs.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    first.abort();
    gate.notify_one();
    let (status, ..) = loop {
        let answer = parts(
            app.clone()
                .oneshot(post(7, Some(TOKEN), true))
                .await
                .unwrap(),
        )
        .await;
        if answer.0 != StatusCode::TOO_EARLY {
            break answer;
        }
        tokio::task::yield_now().await;
    };
    assert_eq!(status, StatusCode::OK);
    assert_eq!(runs.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn stale_builds_are_rejected_before_claiming_or_replaying_tokens() {
    for completed in [false, true] {
        for build in [None, Some("8"), Some("invalid")] {
            let ledger = MemoryLedger::new(limits());
            let token = Token::parse(TOKEN).unwrap();
            assert_eq!(ledger.claim(&token, 7).await, Claim::New);
            if completed {
                ledger.record(&token, recorded("first response")).await;
            }
            let runs = Arc::new(AtomicUsize::new(0));
            let app = app(Some(ledger.clone()), Arc::clone(&runs), None);
            let mut request = post(7, Some(TOKEN), true);
            *request.body_mut() = Body::from(match build {
                Some(build) => format!("__h=7&__b={build}"),
                None => "__h=7".to_owned(),
            });
            let response = app.oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::CONFLICT);
            assert_eq!(response.headers()[hemx_axum::HEMX_FINGERPRINT_HEADER], "9");
            assert_eq!(runs.load(Ordering::SeqCst), 0);
            let expected = if completed {
                Claim::Done(recorded("first response"))
            } else {
                Claim::Running
            };
            assert_eq!(ledger.claim(&token, 7).await, expected);
        }
    }

    // Rejected new requests must not consume a ledger entry either.
    for (body, status) in [
        ("__h=7", StatusCode::CONFLICT),
        ("__h=7&__b=8", StatusCode::CONFLICT),
        ("__h=999&__b=9", StatusCode::NOT_FOUND),
    ] {
        let ledger = MemoryLedger::new(limits());
        let app = app(Some(ledger.clone()), Arc::new(AtomicUsize::new(0)), None);
        let mut request = post(7, Some(TOKEN), true);
        *request.body_mut() = Body::from(body);
        assert_eq!(app.oneshot(request).await.unwrap().status(), status);
        assert_eq!(ledger.usage(), (0, 0));
    }
}

#[tokio::test]
async fn a_disconnected_panicking_action_records_a_terminal_failure() {
    let runs = Arc::new(AtomicUsize::new(0));
    let gate = Arc::new(Notify::new());
    let registry = interactions(BUILD).on_async::<_, (), _>(SAVE, {
        let runs = Arc::clone(&runs);
        let gate = Arc::clone(&gate);
        move |_| {
            let runs = Arc::clone(&runs);
            let gate = Arc::clone(&gate);
            async move {
                runs.fetch_add(1, Ordering::SeqCst);
                gate.notified().await;
                panic!("handler failed after the client disconnected");
            }
        }
    });
    let app = Router::new().route(
        "/save",
        action_with_ledger(
            registry,
            Native::new("/", feedback),
            MemoryLedger::new(limits()),
        ),
    );
    let first = tokio::spawn(app.clone().oneshot(post(7, Some(TOKEN), true)));
    while runs.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    gate.notify_one();
    let replay = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let response = parts(
                app.clone()
                    .oneshot(post(7, Some(TOKEN), true))
                    .await
                    .unwrap(),
            )
            .await;
            if response.0 != StatusCode::TOO_EARLY {
                break response;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("panicked action must not remain Running");
    assert_eq!(replay.0, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(replay.1.as_deref(), Some("1"));
    assert_eq!(replay.2, b"the action failed");
    let again = parts(app.oneshot(post(7, Some(TOKEN), true)).await.unwrap()).await;
    assert_eq!(again, replay);
    assert_eq!(runs.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_token_cannot_be_reused_for_another_action() {
    let runs = Arc::new(AtomicUsize::new(0));
    let app = app(Some(MemoryLedger::new(limits())), Arc::clone(&runs), None);
    let _ = app
        .clone()
        .oneshot(post(7, Some(TOKEN), true))
        .await
        .unwrap();
    let (status, ..) = parts(app.oneshot(post(8, Some(TOKEN), true)).await.unwrap()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(runs.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn without_a_ledger_tokens_change_nothing() {
    let runs = Arc::new(AtomicUsize::new(0));
    let plain = app(None, Arc::clone(&runs), None);
    for _ in 0..2 {
        let (status, ledger, _) = parts(
            plain
                .clone()
                .oneshot(post(7, Some(TOKEN), true))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(ledger, None);
    }
    assert_eq!(runs.load(Ordering::SeqCst), 2);

    // Native submissions never use the ledger, even when one is configured.
    let runs = Arc::new(AtomicUsize::new(0));
    let ledgered = app(Some(MemoryLedger::new(limits())), Arc::clone(&runs), None);
    for _ in 0..2 {
        let (status, ledger, _) = parts(
            ledgered
                .clone()
                .oneshot(post(7, Some(TOKEN), false))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        assert_eq!(ledger, None);
    }
    assert_eq!(runs.load(Ordering::SeqCst), 2);

    let (status, ..) = parts(
        ledgered
            .oneshot(post(7, Some("short"), true))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

fn recorded(body: &str) -> Recorded {
    Recorded {
        status: StatusCode::OK,
        headers: Vec::new(),
        body: body.as_bytes().to_vec().into(),
    }
}

fn token(index: usize) -> Token {
    Token::parse(&format!("token-{index:0>16}")).unwrap()
}

#[tokio::test]
async fn memory_ledger_bounds_entries_bytes_and_age_without_forgetting_live_tokens() {
    // Entries: a full ledger refuses new tokens instead of forgetting one.
    let ledger = MemoryLedger::new(LedgerLimits {
        entries: 2,
        bytes: 1000,
        age: Duration::from_secs(600),
    });
    assert_eq!(ledger.claim(&token(1), 7).await, Claim::New);
    assert_eq!(ledger.claim(&token(2), 7).await, Claim::New);
    ledger.record(&token(1), recorded("one")).await;
    assert_eq!(ledger.claim(&token(3), 7).await, Claim::Unavailable);
    assert_eq!(ledger.usage().0, 2);
    assert_eq!(
        ledger.claim(&token(1), 7).await,
        Claim::Done(recorded("one"))
    );

    // Bytes: kept responses and running reserves stay within budget; a
    // response that does not fit is kept as a 410, and that 410 is also what
    // the first caller receives.
    let ledger = MemoryLedger::new(LedgerLimits {
        entries: 8,
        bytes: 300,
        age: Duration::from_secs(600),
    });
    assert_eq!(ledger.claim(&token(1), 7).await, Claim::New);
    let first = ledger.record(&token(1), recorded(&"x".repeat(120))).await;
    assert_eq!(first, recorded(&"x".repeat(120)));
    assert_eq!(ledger.claim(&token(2), 7).await, Claim::New);
    let first = ledger.record(&token(2), recorded(&"y".repeat(500))).await;
    assert_eq!(first.status, StatusCode::GONE);
    assert_eq!(ledger.claim(&token(2), 7).await, Claim::Done(first));
    assert!(ledger.usage().1 <= 300);
    for index in 3..8 {
        if ledger.claim(&token(index), 7).await == Claim::Unavailable {
            break;
        }
        ledger.record(&token(index), recorded("z")).await;
        assert!(ledger.usage().1 <= 300);
    }
    assert_eq!(
        ledger.claim(&token(1), 7).await,
        Claim::Done(recorded(&"x".repeat(120)))
    );

    // A budget too small for even the 410 answer refuses every token.
    let ledger = MemoryLedger::new(LedgerLimits {
        entries: 8,
        bytes: 0,
        age: Duration::from_secs(600),
    });
    assert_eq!(ledger.claim(&token(1), 7).await, Claim::Unavailable);
    assert_eq!(ledger.usage(), (0, 0));

    // Age: finished tokens older than the limit are forgotten.
    let ledger = MemoryLedger::new(LedgerLimits {
        entries: 1,
        bytes: 1000,
        age: Duration::from_millis(20),
    });
    assert_eq!(ledger.claim(&token(1), 7).await, Claim::New);
    ledger.record(&token(1), recorded("one")).await;
    assert_eq!(ledger.claim(&token(2), 7).await, Claim::Unavailable);
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert_eq!(ledger.claim(&token(2), 7).await, Claim::New);
}

#[tokio::test]
async fn a_resend_under_ledger_pressure_never_runs_its_handler_twice() {
    let runs = Arc::new(AtomicUsize::new(0));
    let ledger = MemoryLedger::new(LedgerLimits {
        entries: 1,
        bytes: 1 << 20,
        age: Duration::from_secs(600),
    });
    let app = app(Some(ledger), Arc::clone(&runs), None);
    let first = parts(
        app.clone()
            .oneshot(post(7, Some(TOKEN), true))
            .await
            .unwrap(),
    )
    .await;
    let other = "fedcba9876543210fedcba9876543210";
    let (status, ..) = parts(
        app.clone()
            .oneshot(post(7, Some(other), true))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let again = parts(
        app.clone()
            .oneshot(post(7, Some(TOKEN), true))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(again, first);
    assert_eq!(runs.load(Ordering::SeqCst), 1);
}
