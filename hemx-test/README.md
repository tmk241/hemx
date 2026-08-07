# hemx-test

Test support for Hemx applications and generated resources.

`hemx-test` keeps assertions at the same typed boundary as application code. Use
an `EffectInspector` with generated targets instead of copying resource IDs or
matching raw wire operations.

## Choose the owning boundary

| Behavior under test | Smallest authoritative proof |
| --- | --- |
| Domain invariants and state transitions | Ordinary Rust unit tests; no Hemx harness |
| Handler output and generated-target effects | `run`, `run_async`, `run_result`, or `run_async_result` plus `EffectInspector` |
| Static document or effect-fragment structure | `HtmlInspector`, reached directly or through `target_html_document` / `target_html_fragment` |
| Axum extraction, middleware, status, headers, and response body | The opt-in `hemx_test::axum` router harness |
| Real process startup, readiness, sockets, logs, and cleanup | `TestProcess::builder` |
| JavaScript, focus, history, layout, or runtime reconciliation | A focused real-browser test outside `hemx-test` |

## Effects and structural HTML

```rust
use hemx_core::{GeneratedTarget, SafeHtml, Slot};
# use hemx_core::{Effect, ResourceId};
# #[derive(Clone, Copy)]
# struct GeneratedTodos(Slot<()>);
# impl GeneratedTarget for GeneratedTodos {
#     fn __hemx_resource_id(self) -> ResourceId { self.0.id() }
# }
# impl GeneratedTodos {
#     fn html(self, value: SafeHtml) -> Effect { self.0.html(value) }
# }

// Application build output supplies generated target types like this one.
let todos = GeneratedTodos(Slot::new(7));
let effect = todos.html(SafeHtml::trusted(
    r#"<ul><li class="todo" data-state="open">Write tests</li></ul>"#,
));

let inspected = hemx_test::inspect(effect);
assert!(inspected.updates_html(todos));

let html = inspected.target_html_fragment(todos)?;
html.assert_count("li.todo", 1);
html.assert_text("li.todo", "Write tests");
html.assert_attribute("li.todo", "data-state", "open");
# Ok::<(), hemx_test::HtmlInspectionError>(())
```

Complete server-rendered pages can be inspected directly:

```rust
let page = hemx_test::inspect_html_document(
    "<!doctype html><html><body><main id=app data-sid=7>Ready</main></body></html>",
);
page.assert_text("main#app", "Ready");
# use hemx_core::{GeneratedTarget, ResourceId, Slot};
# #[derive(Clone, Copy)]
# struct GeneratedApp(Slot<()>);
# impl GeneratedTarget for GeneratedApp {
#     fn __hemx_resource_id(self) -> ResourceId { self.0.id() }
# }
page.assert_target(GeneratedApp(Slot::new(7)));
```

Structural inspection is backed internally by an HTML parser, but its types are
not part of the public API. `HtmlInspector`, `HtmlSelection`, and `HtmlElement`
own the observable source and selected data. Structural inspection does not run
JavaScript or prove browser-owned behavior such as focus, history, layout, or
runtime reconciliation.

A target inspector requires exactly one `Put`, `Insert`, or `Prepend` HTML
payload for that generated target. Missing, non-HTML, and ambiguous target
effects return `HtmlInspectionError` with the relevant effects in the
diagnostic.

## Handlers

`run` and `run_async` invoke typed synchronous and asynchronous handlers and
return the same `EffectInspector`:

```rust
use hemx_core::{Effect, GeneratedTarget, ResourceId, Slot};
# #[derive(Clone, Copy)]
# struct GeneratedCount(Slot<u32>);
# impl GeneratedTarget for GeneratedCount {
#     fn __hemx_resource_id(self) -> ResourceId { self.0.id() }
# }

async fn load_count(value: u32) -> Effect {
    Slot::<u32>::new(1).text(value)
}

# async fn example() {
let count = GeneratedCount(Slot::new(1));
let inspected = hemx_test::run_async(load_count, 42).await;
assert!(inspected.updates_text_containing(count, "42"));
# }
```

Fallible handlers use `run_result` or `run_async_result`. Their concrete error
is returned unchanged and is never converted into an empty or success-looking
effect batch:

```rust
use hemx_core::{Effect, Slot};

#[derive(Debug, Eq, PartialEq)]
struct Rejected;

async fn save(accepted: bool) -> Result<Effect, Rejected> {
    accepted
        .then(|| Slot::<()>::new(1).text("saved"))
        .ok_or(Rejected)
}

# async fn example() {
let error = hemx_test::run_async_result(save, false)
    .await
    .unwrap_err();
assert_eq!(error, Rejected);
# }
```

`IntoEffect` conversion itself is infallible in the current public contract, so
handler errors and inspected success effects remain distinct.

## Axum routers

Enable the `axum` feature to send owned requests through a real `axum::Router`.
The response always preserves status, headers, and raw body bytes; parse it as a
Hemx effect batch or structural HTML only when that is the response contract.

```rust
# #[cfg(feature = "axum")]
# async fn example() -> Result<(), hemx_test::axum::RouterTestError> {
use axum::{response::Html, routing::get, Router};
use axum::http::StatusCode;

let app = Router::new().route(
    "/",
    get(|| async { Html("<main id=app>Ready</main>") }),
);
let response = hemx_test::axum::get("/").send(app).await?;
assert_eq!(response.status(), StatusCode::OK);
response.html_fragment()?.assert_text("main#app", "Ready");
# Ok(())
# }
```

`post(...).form(handle, fields)` builds the URL-encoded interaction body from a
typed Hemx handle. Authentication, CSRF, sessions, persistence, middleware, and
test providers remain application-owned. This in-process harness does not prove
real sockets, process startup, or browser behavior.

## Processes

Use `TestProcess` only when the process and socket lifecycle are part of the
behavior under test. Readiness is explicit and bounded:

```rust,no_run
use hemx_test::TestProcess;
use std::process::Command;
use std::time::Duration;

let process = TestProcess::builder(Command::new("target/debug/my-app"))
    .label("application server")
    .arg("serve")
    .env("APP_ADDR", "127.0.0.1:4100")
    .http("127.0.0.1:4100", "/health")
    .timeout(Duration::from_secs(5))
    .start()?;

assert!(process.id().is_some());
# Ok::<(), hemx_test::ProcessError>(())
```

TCP readiness proves only that something accepts the address; HTTP readiness
requires a 2xx or 3xx response from the selected path. Startup errors include
the process label, readiness attempts, exit status when available, and bounded
stdout/stderr. Readers keep draining after the capture limit so noisy children
do not deadlock. Explicit `shutdown` and `Drop` both kill, wait for, and reap a
running child and are safe to call more than once.

`TestProcess::start` remains available as the compact compatibility entry point
for TCP readiness. The harness intentionally does not reserve ports or claim
that a reserve-then-bind handoff is atomic.

## Migrating selector helpers

The original `0.1.0` crate exposed app-specific CSS builders and island/browser
probe helpers. They duplicated CSS syntax, made application structure look like
a framework contract, and could not prove browser behavior. They are removed
from the next release rather than preserved as a second testing vocabulary.

- Replace semantic selector builders such as `article_selector`,
  `class_selector`, and `nav_link_selector` with the CSS selector that expresses
  the application's own HTML contract in `HtmlInspector`.
- Replace `assert_rendered_target` and `assert_rendered_handle` with
  `HtmlInspector::assert_target` and `HtmlInspector::assert_handle`; generated
  resources remain the assertion vocabulary and raw runtime IDs stay private.
- Replace `target_selector`, `handle_selector`, and keyed selector builders with
  structural HTML assertions. Browser tests that genuinely need a selector
  should keep that selector in their browser-test adapter.
- Replace island probe scripts, readout selectors, event-name helpers, and SSE
  marker strings with a focused real-browser journey; `hemx-test` does not
  emulate JavaScript or runtime behavior.

The generated-resource helpers for effect inspection and typed form bodies
remain supported.

## License

MIT
