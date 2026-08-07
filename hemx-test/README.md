# hemx-test

Test support for Hemx applications and generated resources.

`hemx-test` keeps assertions at the same typed boundary as application code. Use
an `EffectInspector` with generated targets instead of copying resource IDs or
matching raw wire operations.

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
    "<!doctype html><html><body><main id=app>Ready</main></body></html>",
);
page.assert_text("main#app", "Ready");
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

## License

MIT
