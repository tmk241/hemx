# Native browser conformance

This unpublished fixture exercises the shipped `hemx-js` bundle in real,
same-origin iframe documents. Rust produces the effect bytes and generated
resource metadata. Rust assertions check the browser's observations; compiling
this crate alone does **not** execute those assertions.

```console
cargo run --locked -p hemx-browser-test
```

Open the printed loopback URL in a fresh, disposable browser profile within
60 seconds. A reused profile can serve a cached runtime from an earlier run;
only a fresh profile tests the bundle this build serves. The suite passes in
current Firefox, Chromium and WebKit (Playwright's Safari 27.2 engine), for
example headless:
`firefox --headless --no-remote --profile "$(mktemp -d)" URL` or
`chromium --headless=new --user-data-dir="$(mktemp -d)" URL`.
Without native `moveBefore` (Safari today) the positive movement case checks
the refusal path instead, so run a browser that has it to cover both.
Browsers differ in how often they fire `formdata` for one navigation
(Chromium also builds `NavigateEvent.formData`, even for GET), so the cases
assert what Hemx sends and that it never serializes a form itself, not the
event count. Once the browser requests its cases, the suite has a separate
60-second execution deadline; startup does not consume that budget. The page
shows the current case and completed count. The fixture changes DOM and history
state and creates synthetic files; it needs no account, external service, or real user data. Use a browser
with native `moveBefore` to exercise the positive movement path. A separate
case removes that capability before loading the runtime and checks failure
without mutation. This tests both branches, not every older browser engine.

The runner prints JSON records for readiness, execution start, assertion groups,
and the final result, then closes its server. Only the first report is accepted; further
submissions are rejected. Missing, incomplete, or failing reports do not count
as success. Independent assertion groups continue after a
failure to expose other defects, but any failed group makes the run fail. The
asserted case set must exactly match the observation set; this comparison runs
even after a behavior group fails and exposes cases skipped by that failure.

Most fixtures intercept `fetch` to inspect native successful controls and
return Rust-produced wire bytes. Four groups use this server instead:
`src/native_forms.rs` serves real HTML/redirect and effect responses through
`hemx_axum::InteractionForm` and `EffectResponse`, `src/streams.rs` serves
real SSE connections, and `src/errors.rs` serves real failing HTTP interactions
and SSE batches. `src/timeouts.rs` exercises real delayed headers, response-body
chunks and multipart consumption, plus queued requests. No replacement clock or
fetch result is used for deadline checks. They include an eleven-second success
with an explicit longer budget and the existing real ten-second default expiry.
The keyed-row cases cover collection targets and explicit row resources. The
explicit-row case applies the same full keyed wrapper twice, then a content-only
patch; it checks retained nodes, no nested wrapper, and no ambiguous-target error.
The error cases assert synchronous translation after the final outlet update,
stable codes, one diagnostic, root isolation, and no POST replay.
`src/ledger.rs` serves real `action_with_ledger` and plain `action` routes and
holds a completed response past the deadline: the ledger route's one token
resend receives the recorded result, a resend during the first run gets 425,
and the plain route is never resent.
None proves an application's database or external delivery. History, validation, form-data events, node identity, file
selection, selection ranges, and ownership checks use actual browser APIs, not a
replacement DOM or history implementation. Where a native capability cannot be
driven from page script (`confirm`, `pagehide`, drag gestures), the probe says so
in a comment and tests the runtime's handling, not the browser's gesture.

Observations live in `probe.js` and the topic modules it loads
(`runtime-probe.js`, `forms-probe.js`, `wire-probe.js`, `native-forms.js`,
`lifecycle-probe.js`, `streams-probe.js`, `errors-probe.js`, `timeouts-probe.js`). Rust assertions live in
`src/checks.rs` and its `*_checks.rs` modules; wire producers live in
`src/lib.rs` and its `*_cases.rs` modules. Wait for observable asynchronous
completion; a single zero-delay timer does not establish that a streamed
response or navigation has finished.

`futures-util` is a dependency of this unpublished
fixture only; it is not a dependency of the published Hemx packages.
