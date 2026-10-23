# Axum + Hemplate counter

One page with a nested component, one generated action, one server-owned counter. No JavaScript
application code, database, external service, or authentication integration.

From the repository root (Rust 1.91+ and the workspace's Hemplate checkout):

```console
cargo run -p hemx-counter-example -j4
```

Open <http://127.0.0.1:3000>. Add an amount from 0 to 255. With supported browser
navigation enhancement, the counter updates in place. Disable JavaScript and
submit again: the same form POST updates the same counter, then redirects to `/`.
Browsers without the required navigation capability also submit natively. Stop
the server with Ctrl-C; restarting resets the count.

Run the integration tests without starting a server:

```console
cargo test -p hemx-counter-example -j4
```

## Follow the request


- [`build.rs`](build.rs) runs `hemx-build` against
  [`templates/page.heml`](templates/page.heml) and
  [`templates/counter_value.heml`](templates/counter_value.heml). `#[hemx::surface]` includes the
  generated typed resources and handle. `#[hemx::form]` binds the `amount: u8`
  model to the template's form, validating required integer input on the server,
  not just with the browser's `min`/`max` attributes.
- [`src/lib.rs`](src/lib.rs) renders with **`heml::page::render(&Page { .. })`**.
  This generated helper first renders Hemplate and then lowers the complete
  application resource/handle table into runtime markers, including the form's
  hidden `__h` and build `__b`. Raw `Hemplate::render` only renders the template:
  it is **not** the normal Hemx page path. `PageResponse::fingerprint` then stamps
  the page root with the matching `data-hemx-build` and sets the response header.
- The page composes its presentational child with
  **`{+ CounterValue::new(self.count) +}`**. The child implements `Hemplate`, so
  this writes its markup directly into the parent's buffer; ordinary text values
  passed through `{+ ... +}` are escaped. The outer `heml::page::render` also
  lowers the child's `count` resource. There is no intermediate `heml::render`
  call inside the template and no raw `{+= ... =+}` wrapper. Raw interpolation
  is for already-produced, explicitly trusted HTML and rejects Hemplate values.
  `heml` names a generated Rust module, not another template operator.
- The template's script field is the typed `hemx_axum::RuntimeScript`, not an
  unchecked URL. The Router serves the actual bundled runtime at
  `runtime_js_route_path()` using `get(|| async { hemx_axum::runtime_js() })`;
  Axum needs the async wrapper around the synchronous asset helper.
- `POST /increment` extracts an `InteractionRequest` and dispatches the
  generated `heml::page::increment` handle to the checked `#[hemx::handler]`.
  Both native and enhanced requests take this path. The extractor rejects
  cross-site requests, dispatch rejects missing/stale builds, and the generated
  form decoder validates the amount **before mutation**. An Axum body limit
  caps submissions at 1 KiB.
- The handler changes the process-local count and returns
  `heml::counter_value::count.text(..)`. The runtime asks for `application/hemx`; that
  representation receives the effect. An ordinary form receives a 303 redirect
  after the same handler runs. Enhancement failures are reported in the root's
  accessible error outlet and are **never automatically replayed as POSTs**.
  Native invalid input receives plain HTTP 400; stale forms receive 409 and
  require reloading. This small example does not add custom error pages.

[`tests/router.rs`](tests/router.rs) drives the actual Router: it reads generated
hidden fields from the rendered document, verifies that the nested child is real
markup with its own lowered resource, fetches the runtime through its script
URL, checks the decoded effect against that same child resource, exercises both
response representations, and proves rejected submissions leave state unchanged.
These are server integration tests, not a browser automation suite.

## Deliberate limits

The server binds only `127.0.0.1:3000`. The count is shared by every client of that
process, is not durable, and has no sessions, authentication, authorization, or
application CSRF policy. Fetch Metadata and build checks are not substitutes for
those policies. The demo accepts clients without Fetch Metadata headers, just as
the adapter does. Do not expose it as a production service or treat the counter
as durable storage. This workspace member is `publish = false`.
