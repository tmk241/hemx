# hemx-axum

Axum integration for Hemx pages, generated routes, assets, effects, and server push.

## Effect transport

Successful `EffectResponse` values carry canonical ABI-3 bytes with
`Content-Type: application/hemx` and `x-hemx-fingerprint`. An incompatible or
oversized batch produces HTTP 500 with a diagnostic naming the encoding failure,
without disclosing effect contents or sending invalid wire bytes.

`encode_sse_batch(&batch)` returns `Result<String, hemx_core::WireError>`;
`sse_event(batch)` returns `Result<Event, hemx_core::WireError>`. SSE carries the
same canonical bytes as unpadded base64url in `hemx` events. `sse(stream)` maps
both upstream and encoding failures to `axum::BoxError` and fails the body
stream. It does not invent an error-event protocol; applications decide how to
recover from a failed connection. Already-sent events cannot be recalled.


When a client reads slower than updates arrive, `sse(stream)` coalesces waiting
batches using the core folding rules: batches with the same ABI and fingerprint
and no side effects may share one frame, with **every operation preserved in
order**. No intermediate patch, row declaration, or delta is discarded or
rewritten. Batches with side effects and differing builds remain barriers. A
run exceeding `MAX_EFFECTS` or `MAX_BATCH_BYTES` stays in its original frames.
This saves envelopes and SSE framing, not HTML payload or browser DOM work, and
does not make a slow client skip a backlog.

An upstream error is emitted after the batches that preceded it. Items may be
owned `EffectBatch` values or shared ones such as `Arc<EffectBatch>`: a broadcast
to many connections then coalesces and encodes per connection, cloning
operations only for runs that fit the combined wire budget.

A connection that missed updates (a new page that raced a change, a reconnect,
or a lagging broadcast receiver) needs the current state, not the missed steps:
send a `set` declaration of every collection it shows, or a `Visit` when that
would exceed the batch budget.

## Including the runtime


Serve the browser runtime with `runtime_js()` at `runtime_js_route_path()` and
include it with the typed `RuntimeScript` value. Hemplate does not accept a plain
string in `<script src>` (even an `https:` URL there loads code), and
`RuntimeScript` is a same-origin, content-addressed path that needs no unchecked
constructor:

```html
<script src="{+ self.runtime +}" defer></script>
```

```rust,ignore
struct Page { runtime: hemx_axum::RuntimeScript }
```

`runtime_js_script_src()` still returns the path as a `&'static str` for code that
builds responses without Hemplate.

## Live updates

`Live` does that bookkeeping. Every published batch gets the next `Cursor`. A
connection resumes from the cursor the browser last saw (`Last-Event-ID` when
`EventSource` reconnects, or the cursor the page was rendered with) by
replaying what it missed from a window of recent batches. When it cannot (first
contact without a cursor, a server restart, a gap older than the window, or a
connection that fell behind) it starts from a snapshot the application
provides. Coalesced replay frames retain every missed operation and carry the
last cursor in that frame; unfolded frames retain their individual cursors.
Each batch is encoded once at publication and that text is shared by every
connection that sends it unfolded.

```rust,ignore
// Publish under the lock that guards the state the batch describes.
let mut store = app.store.lock().unwrap();
let before = store.keys();
store.add(todo.clone());
app.live.publish(ui::todos::rows.transition(before, store.keys(), [&todo]))?;

// Render the page under the same lock, with the cursor it shows:
// <main data-hemx-root data-hemx-sse="/events?after={+ self.cursor +}">
let cursor = app.live.cursor().to_string();

// The stream: a snapshot reads its state and cursor under that lock too.
async fn events(State(app): State<App>, resume: hemx_axum::Resume) -> impl IntoResponse {
    let live = app.live.clone();
    live.sse(resume, move || {
        let store = app.store.lock().unwrap();
        (app.live.cursor(), ui::todos::rows.set(&store.todos))
    })
}
```

The contract that makes this exact is the lock: publish in the order the state
changes, and read `cursor()` together with the state it describes. A snapshot
must fit the batch budget; for state that may not, return a `Visit` of the page.
`Live::new` replays the last 1,024 batches; `Live::with_replay` sets another
window.

## Native forms first, optional enhancement

Start with an ordinary form: a real application `action`, a `method`, named
controls, and a submit button. GET actions are read-only queries with URL state;
POST actions perform mutations. Keep native constraint validation and submit
listeners: calling `preventDefault()` on `submit` remains the application's veto.
Generated form/action helpers add Hemx markers and the hidden `__h` handle, but
those markers do not create an HTTP endpoint or a native HTML response.

The application must route every native action, including submitter
`formaction`/`formmethod` overrides. A native GET returns a complete HTML page.
A successful native POST normally returns **303 See Other** to an application
GET page; validation failure returns HTML (for example, HTTP 422), with
`aria-invalid` fields and associated error messages. Never serve canonical effect
bytes as the document for an ordinary browser submission.

Use the **same validation and mutation operation** for native and enhanced
requests. Decode and authorize the input, validate it, and mutate once; only then
choose a representation of the result. Do not maintain a second enhanced-only
implementation of application rules. One application-owned POST route can
negotiate the response, or separate native and interaction routes can call the
same operation:

```text
POST /items
  decode input; authenticate; check authorization and CSRF policy
  outcome = save_item(input)       # shared validation and one mutation point
  if this request accepts application/hemx:
      outcome -> generated resource effects -> EffectResponse
  else if outcome is valid:
      outcome -> 303 Location: /items/<id>
  else:
      outcome -> complete HTML form with accessible errors (422)
```

`InteractionRequest`/`InteractionForm` can decode a generated POST form's hidden
handle and URL-encoded or multipart controls; ordinary Axum extractors also work
for app-owned native endpoints. Apply the application's body limit before
handling either representation. Negotiating `application/hemx` is not
an authorization or CSRF boundary. `EffectResponse::new(effects, fingerprint)`
encodes the generated effects; it does not choose the application's native
route, render its validation page, or implement its redirect. For enhanced
validation errors, return a successful canonical effect response that renders
the accessible errors through generated resources, rather than an HTTP failure
that the runtime cannot apply. GET enhancement can return the same HTML as the
native GET endpoint.

The browser runtime enhances supported cancelable same-origin navigation after
native submit vetoes. It does **not** require `precommitHandler`. Without the
runtime or the required navigation capability, the form submits natively. This
makes the native endpoint a baseline, not an error-recovery trick. Enhanced
POST effects leave URL/history alone unless an effect explicitly navigates.
If an enhanced POST fails after admission, Hemx reports the failure and **never
automatically replays it as a native POST**: the server may already have mutated.
The application must offer an explicit recovery action and use its own
idempotency policy where retrying a mutation is appropriate.

## Bounded multipart input


Keep Axum's whole-request limit. Add `MultipartLimits` when text/name bytes and
file contents need separate cumulative budgets:

```rust,no_run
use axum::{extract::DefaultBodyLimit, routing::MethodRouter, Extension, Router};
use hemx_axum::MultipartLimits;

// Pass your POST route using the normal InteractionRequest extractor.
fn upload_router(upload: MethodRouter) -> Router {
    Router::new()
        .route("/upload", upload)
        .layer(DefaultBodyLimit::max(66 * 1024 * 1024))
        .layer(Extension(MultipartLimits {
            text_bytes: 1024 * 1024,
            file_bytes: 64 * 1024 * 1024,
        }))
}
```

`text_bytes` includes every field name (including file names **as form keys**,
`__h` and `__b`) plus text values, measured in UTF-8 bytes. `file_bytes` sums
contents across every file part. Parts without a form name count too, although
Hemx does not store them. A part with a filename, including `filename=""`, is a
file. Filename metadata, content types, other headers and boundaries count only
against the whole-body limit. Equality is allowed; zero permits no bytes in its
category. Without the extension the existing whole-body limit remains the only
size policy. URL-encoded requests are unaffected by `MultipartLimits`.

Excess returns `InteractionFormRejection::BodyTooLarge` (HTTP 413) as soon as a
field name or delivered chunk exceeds its remaining budget: Hemx neither copies
that chunk into its payload buffer nor requests the next chunk. It does not wait
for EOF. The original request headers and extensions stay intact, including
Fetch Metadata, `AllowSameSite` and Axum's body-limit configuration.

Accepted files are still buffered in memory. This is bounded reading, **not** a
streaming file sink or a hard heap limit: the transport/parser may buffer data,
and allocations and per-part metadata have overhead. File count, file-format
validation, concurrency and storage policy remain application responsibilities.
Use the normal `InteractionRequest` or `InteractionForm` extractor; manually
extracting `Multipart` and calling `parse_multipart` does not apply the extension.
There is no reason to serialize a parsed upload into another multipart request.

## Interaction integrity

Two checks run before an application handler can mutate anything. Neither is an
authentication, authorization, or session boundary; they remove two common ways
for a request to arrive that the page never meant to send.

**Cross-site requests.** Extracting an `InteractionRequest` or `InteractionForm`
reads `Sec-Fetch-Site` before it reads the body. `cross-site` is rejected with
403. `same-site` (for example a sibling subdomain) is also rejected unless the
route adds `axum::Extension(AllowSameSite)`; the opt-in never admits cross-site.
`same-origin`, `none`, and a missing header pass: older browsers and non-browser
clients send no header, so absence is not a decision. This protects browsers
that send the header; it does not replace your own CSRF or authorization policy.

**Stale pages.** Generated action forms carry the build fingerprint in a hidden
`__b` field next to `__h`, so native and enhanced submissions are both checked.
Registry dispatch (`interactions(..)`, `dispatch`, `dispatch_async`) answers a
missing or different `__b` with 409 and the server's `x-hemx-fingerprint`,
**before** any handler runs. If you read an `InteractionForm` yourself instead of
using the registry, call `form.require_build(heml::BUILD_FINGERPRINT)?` first;
skipping it bypasses the guard. A hand-written form that bypasses the generated
markup must render the same hidden field.

The browser runtime treats any response whose fingerprint differs from its page
as a stale page, reports it once, shows "This page is out of date. Reload to
continue." in `data-hemx-error` outlets, and never replays the request. A native
form without the runtime receives the plain 409 text; render your own reload page
if that matters. A 409 without the server's fingerprint header is treated as an
ordinary application conflict.

Both checks are unreleased: Hemx 0.4.0 applies neither.

## License

MIT
