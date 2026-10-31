//! A single server-owned counter, with native and enhanced form submissions.

use std::sync::{Arc, Mutex};

use axum::extract::{DefaultBodyLimit, State};
use axum::routing::get;
use axum::Router;
use hemplate::Hemplate;
use hemx_axum::{
    interactions, Form, InteractionForm, LedgerLimits, MemoryLedger, Native, PageResponse,
    RuntimeScript,
};

#[hemx::surface]
pub mod heml {}

type Counter = Arc<Mutex<u64>>;

#[derive(Hemplate)]
struct Page {
    count: u64,
    runtime: RuntimeScript,
    /// The amount shown in the form: the default, or a rejected submission.
    amount: String,
    /// Why a native submission was rejected, announced beside its field.
    problem: Option<String>,
}

impl Page {
    fn new(count: u64) -> Self {
        Self {
            count,
            runtime: RuntimeScript,
            amount: "1".into(),
            problem: None,
        }
    }
}

// A presentational child is an ordinary Hemplate view, not a handler component.
#[derive(Hemplate)]
struct CounterValue {
    count: u64,
}

impl CounterValue {
    fn new(count: u64) -> Self {
        Self { count }
    }
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
    heml::counter_value::count.text(*count)
}

/// Construct a fresh demonstration application. All its clients share one count.
pub fn app() -> Router {
    let counter = Counter::default();
    // The adapter answers both representations of one action: the runtime's
    // enhanced request receives effects, a plain form a 303 back to the page,
    // or the page again with the rejected amount and its problem. The same
    // extraction guards and handler run either way.
    let feedback = {
        let counter = counter.clone();
        move |input: &InteractionForm, problem: &str| {
            let mut page = Page::new(*counter.lock().expect("counter lock poisoned"));
            page.amount = input.value("amount").unwrap_or_default().to_owned();
            page.problem = Some(format!("Enter a whole number from 0 to 255 ({problem})."));
            heml::render(&page)
        }
    };
    Router::new()
        .route("/", get(index))
        .route(
            "/increment",
            hemx_axum::action_with_ledger(
                interactions(heml::BUILD_FINGERPRINT).on_state_form(
                    heml::page::increment,
                    counter.clone(),
                    increment,
                ),
                Native::new("/", feedback),
                // Remembers recent enhanced submissions, so the runtime's one
                // resend after a lost response never adds the amount twice.
                // One process only: several would share a ledger in storage.
                MemoryLedger::new(LedgerLimits {
                    entries: 1024,
                    bytes: 1 << 20,
                    age: std::time::Duration::from_secs(600),
                }),
            ),
        )
        .merge(hemx_axum::runtime_route())
        .layer(DefaultBodyLimit::max(1024))
        .with_state(counter)
}

async fn index(State(counter): State<Counter>) -> PageResponse {
    let count = *counter.lock().expect("counter lock poisoned");
    // Templates are lowered when they compile: rendering already yields final
    // resource markers and the hidden __h / __b fields. The rendered Html knows
    // its build, which the page response stamps on each root.
    PageResponse::full(heml::render(&Page::new(count)))
}
