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
- [`src/lib.rs`](src/lib.rs) renders with **`heml::render(&Page { .. })`**.
  The templates were lowered when they compiled, so this render already contains
  the runtime markers and the form's hidden `__h` and build `__b`; there is no
  separate lowering step. The returned `Html` knows its build, and
  `PageResponse::full` stamps the page root with the matching `data-hemx-build`
  and sets the response header.
- The page composes its presentational child with
  **`{+ CounterValue::new(self.count) +}`**. The child implements `Hemplate`, so
  this writes its markup directly into the parent's buffer; ordinary text values
  passed through `{+ ... +}` are escaped. The child's `count` resource marker is
  already final in its compiled template. There is no render call inside the
  template and no raw `{+= ... =+}` wrapper. Raw interpolation
  is for already-produced, explicitly trusted HTML and rejects Hemplate values.
  `heml` names a generated Rust module, not another template operator.
- The template's script field is the typed `hemx_axum::RuntimeScript`, not an
  unchecked URL. The Router merges `hemx_axum::runtime_route()`, which serves
  the actual bundled runtime at `runtime_js_route_path()`.
- `POST /increment` is `hemx_axum::action(..)`: it extracts the interaction and
  dispatches the generated `heml::page::increment` handle to the checked
  `#[hemx::handler]`. Both native and enhanced requests take this path; the
  application never inspects `Accept` or other headers. The extractor rejects
  cross-site requests, dispatch rejects missing/stale builds, and the generated
  form decoder validates the amount **before mutation**. An Axum body limit
  caps submissions at 1 KiB.
- The handler changes the process-local count and returns
  `heml::counter_value::count.text(..)`. The runtime asks for `application/hemx`; that
  representation receives the effect. An ordinary form receives a 303 redirect
  to the declared `/` after the same handler runs. Enhancement failures are
  reported in the root's accessible error outlet. The route keeps a
  `MemoryLedger`, so after a lost or late response the runtime may send the same
  submission once more with its token; the ledger answers with the first result
  and the amount is never added twice. Without such a ledger nothing is resent. Native invalid input receives the page again (HTTP 422)
  with the rejected amount, `aria-invalid` and its associated problem; stale
  forms receive 409 and require reloading.

[`tests/router.rs`](tests/router.rs) drives the actual Router: it reads generated
hidden fields from the rendered document, verifies that the nested child is real
markup with its own final resource marker, fetches the runtime through its script
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
