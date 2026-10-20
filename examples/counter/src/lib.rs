//! A single server-owned counter, with native and enhanced form submissions.

use std::sync::{Arc, Mutex};

use axum::extract::{DefaultBodyLimit, State};
use axum::http::{header, HeaderMap};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use hemplate::Hemplate;
use hemx_axum::{
    interactions, DispatchRejection, Form, InteractionRequest, PageResponse, RuntimeScript,
};

#[hemx::surface]
pub mod heml {}

type Counter = Arc<Mutex<u64>>;

#[derive(Hemplate)]
struct Page {
    count: u64,
    runtime: RuntimeScript,
}

// The generated decoder validates the required integer's 0–255 range on the
// server too; native HTML constraints alone are not an input boundary.
#[hemx::form("increment_form")]
struct Increment {
    amount: u8,
}

// Both representations dispatch this handler, after decoding and request guards.
// No client-side counter or separate native mutation path exists.
#[hemx::handler]
fn increment(counter: Counter, form: Form<Increment>) -> impl hemx::IntoEffect {
    let mut count = counter.lock().expect("counter lock poisoned");
    *count += u64::from(form.amount);
    heml::page::count.text(*count)
}

/// Construct a fresh demonstration application. All its clients share one count.
pub fn app() -> Router {
    Router::new()
        .route("/", get(index))
        .route("/increment", post(submit))
        // Axum requires an async handler, not the synchronous runtime_js function.
        .route(
            hemx_axum::runtime_js_route_path(),
            get(|| async { hemx_axum::runtime_js() }),
        )
        .layer(DefaultBodyLimit::max(1024))
        .with_state(Counter::default())
}

async fn index(State(counter): State<Counter>) -> PageResponse {
    let count = *counter.lock().expect("counter lock poisoned");
    // Generated rendering lowers resource/action names and adds hidden __h / __b.
    // The Axum page response stamps each root with that same build fingerprint.
    PageResponse::full(
        heml::page::render(&Page {
            count,
            runtime: RuntimeScript,
        })
        .into_string(),
    )
    .fingerprint(heml::BUILD_FINGERPRINT)
}

async fn submit(
    State(counter): State<Counter>,
    headers: HeaderMap,
    request: InteractionRequest,
) -> Result<Response, DispatchRejection> {
    let enhanced = headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .any(|part| part.trim() == "application/hemx")
        });

    // The extractor rejects cross-site requests; registry dispatch rejects stale
    // builds before calling increment. Native submissions pass the same checks.
    let effects = request.dispatch(interactions(heml::BUILD_FINGERPRINT).on_state_form(
        heml::page::increment,
        counter,
        increment,
    ))?;
    Ok(if enhanced {
        effects.into_response()
    } else {
        Redirect::to("/").into_response()
    })
}
