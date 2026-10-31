# Hemx

Hemx is checked hypermedia for Rust. Applications render Hemplate views, handle
events in typed Rust functions, and return generated UI effects. The browser runs
a small effect interpreter instead of a virtual DOM, hydration framework, or
client-side expression language. The same core compiles for native servers and
server-side Wasm isolates; platform storage, sockets, and lifecycle remain
application concerns.

## How it works

1. `.heml` templates declare page roots, slots, collections, forms, handles, and keyed rows.
2. `hemx-build` generates typed Rust helpers from that surface.
3. `#[hemx::handler]` functions accept ordinary Rust inputs and return typed effects.
4. `hemx-axum` serves pages, assets, handler routes, and effect responses.
5. The browser runtime validates the build fingerprint and applies effects within the current root.

```rust,ignore
#[hemx::handler]
async fn add_todo(app: App, form: hemx::Form<NewTodo>) -> impl hemx::IntoEffect {
    let todos: Vec<TodoRow> = app.add(form.0).await;
    ui::todos::rows.set(&todos)
}
```

```html
<form action="/todos" method="post" data-hemx-form="new_todo" data-hemx-handle="add_todo">
  <input name="title" required>
  <button type="submit">Add</button>
</form>
<ul data-hemx-collection="rows"></ul>
```

This excerpt uses a `todos.heml` template, application state `App`, and an
application-defined `TodoRow` implementing `Hemplate` and `hemx::KeyedPartial`.
Effects declare state rather than issue commands. A collection either receives
its complete content (`set`) or a transition from one key order to the next
(`transition(&before, &after, &changed)`): only the steps between the two
orders and the changed rows travel, and the browser applies them only when its
rows are in the `before` order. A delta already at its resulting order is
skipped. Merging preserves every intermediate operation in order: an intervening
update can affect unsent form values even when the final defaults look the same.
Never discard an unsent declaration in favor of a newer one. Keyed row targets
replace one existing row; ordinary slots remain HTML/text update hosts. Templates
are lowered when they compile, so every render already carries the final
markers of nested components.

Wire ABI and runtime ABI are version 3; the surface schema is version 2.
Encoding is fallible: `batch.to_wire()?` rejects incompatible or oversized
batches before emitting bytes. Earlier wire ABIs are rejected, not translated.

## Build-time formatting

`hemx_build::app().run()` requires canonical Hemplate layout by default. A build
error names every noncanonical template and the fix: `hemplate-fmt templates`.
Use `hemplate-fmt --check templates` to check without rewriting. Formatting
preserves rendered output, attribute order, expressions, and raw/preformatted
content.

An application may explicitly opt out in `build.rs`:

```rust,ignore
hemx_build::app().check_format(false).run()?;
```

Custom template directories and precomputed surfaces do not disable the check.
See [hemx-build](hemx-build/README.md) for the build contract.

## Run a complete example

The [Axum + Hemplate counter](examples/counter/README.md) has a page with a nested component,
generated typed form/resource/handle APIs, the actual browser runtime, and one
mutation path for enhanced effects and native form redirects:

```console
cargo run -p hemx-counter-example -j4
# Open http://127.0.0.1:3000; Ctrl-C to stop.
cargo test -p hemx-counter-example -j4
```

Run from this repository with its Hemplate checkout. The example binds loopback
only and uses shared process-local state, not production authentication or
storage. Disable JavaScript to try the native fallback.

There is one render path. `hemx-build` hands Hemplate the generated resource IDs
while templates compile, so rendering a view already produces the final
`data-hemx-resource` / `data-hemx-action` markers and the hidden `__h` / `__b`
fields of handled forms. Nothing rewrites HTML after rendering: interpolated
text and trusted raw HTML reach the page unchanged. Resource names must be
literal (`data-hemx-slot="count"`, never `+data-hemx-slot`).

Render any view with the generated `heml::render(&view)` and pass its `Html`
to `hemx_axum::PageResponse::full(..)`. No template name is needed: every
template was lowered when it compiled. That `Html`
knows the build that rendered it; the response stamps it on each root and in
its header. `PageResponse` does not accept plain strings. Inside the template,
compose children with `{+ Child::new(...) +}`: Hemplate renders a component
directly into the parent's buffer, while ordinary text values are escaped.
Raw `{+= ... =+}` is for already-produced, explicitly trusted HTML, not
component composition; it accepts Hemx's `SafeHtml` and rendered `Html` (or
Hemplate's `TrustedHtml`), never a plain string. `heml` is the generated Rust module, not another
template operator.

`hemx_axum::action` answers both representations of a form action from one
handler: effects for the runtime, a 303 to a declared same-origin page or a
422 feedback page for a plain form. `hemx_axum::runtime_route()` serves the
browser runtime; the template includes it through the typed `RuntimeScript`.

## Migrating from 0.5 to 0.6

Use Hemplate 0.5 (hemplate-core and hemplate-runtime 0.4). Rebuild templates,
server and runtime together. Two applications migrated with mechanical edits
of about 20 files each and kept all their tests passing; the steps below are
the ones they needed, roughly in the order the compiler asks for them.

1. **Build script.** Remove `.global_exports(true)`; `hemx_build::app().run()`
   is the whole build step.
2. **One generated module.** Replace a root
   `include!(concat!(env!("OUT_DIR"), "/hemx.generated.rs"))` with
   `#[hemx::surface] pub mod heml {}`. Generated names move under it:
   `crate::item_list::targets::x` becomes `crate::heml::item_list::targets::x`,
   `BUILD_FINGERPRINT` becomes `heml::BUILD_FINGERPRINT`. The old root
   `targets` module is gone; name the template, as in
   `heml::article_page::targets::status`.
3. **Render.** Templates are lowered when they compile, so every view renders
   through one function: `heml::render(&view) -> Html`. It replaces
   `heml::page(..)`, `heml::<template>::render(..)`, `view.render()?` followed
   by `lower(..)`, and `hemx::page`/`render_html`. Rendering no longer returns
   a `Result`: prepare fallible values (for example `Url::parse`) before
   building the view.
4. **Responses.** `PageResponse::full(heml::render(&page))` takes the typed
   `Html` and stamps its build itself; drop `.fingerprint(..)` and any
   `String` argument; `PageRequest::page` likewise takes and returns `Html`. A
   helper such as `fn page_response(html: Html)` keeps one place for shared
   headers.
5. **Fragments.** `Html` is a finished render of this build. Markup you store
   and compose before rendering (the old `Html::join`) is `SafeHtml`:
   `SafeHtml::join(..)`, and `heml::render(&view).into()` to keep a rendered
   view as a fragment. Prefer composing child views with `{+ child +}` where
   the structure allows it.
6. **Raw output.** `{+= x =+}` takes `SafeHtml`, `Html` or Hemplate's
   `TrustedHtml`, never a string: `{+= self.x.as_str() =+}` becomes
   `{+= self.x =+}`. A sanitizer's output type implements
   `hemplate::HtmlValue` (`impl hemplate::HtmlValue for ArticleContent {}`)
   to be inserted raw.
7. **Templates.** Resource names (`data-hemx-slot`, `-collection`, `-handle`,
   `-form`) must be literal attributes; a handled form needs
   `<form ...></form>`, not `<form .../>`. Markers inside interpolated text or
   raw HTML are no longer rewritten.
8. **Forms.** A `#[hemx::form]` field without a control in the declaring
   template now fails with a diagnostic naming the form, field and template.

New in 0.6: `hemx_axum::action` with `Native` answers native and enhanced
submissions from one route; `runtime_route()` serves the runtime;
`action_with_ledger` with `MemoryLedger` makes the runtime's single token
resend safe, and the browser reports `HEMX_SUBMISSION_PENDING` for a resend
that finds its first request still running.

New in 0.6.1 (`hemx-js`, `hemx-axum`): forms are taken over at their submit
event instead of by canceling a navigation, so Playwright and other automation
no longer wait for a navigation that never finishes, and enhancement no longer
needs the Navigation API. `hemx_axum::sse_until_done` ends a stream for
finished work without the browser reconnecting.

## Migrating from 0.4 to 0.5

Rust 1.91 or newer is required. Use Hemplate 0.4 with hemplate-runtime 0.3;
older runtime crates do not define the context traits used by `RuntimeScript`.

- Wire ABI 3 replaces insertion/removal/movement commands with collection
  `set` and `transition(before, after, changed)` declarations. Rebuild templates,
  server and browser assets together; ABI 2 is rejected rather than translated.
- Generated action forms carry `__b`. Handler dispatch rejects stale builds
  with 409 and cross-site requests with 403 before handlers run. Manual dispatch
  must use `require_build`; an enhanced failed POST is resent only once with its
  token, and only to an `action_with_ledger` route that deduplicates it.
- Use `hemx_axum::RuntimeScript` for an interpolated Hemplate `<script src>`.
  Hemplate now requires checked or explicitly trusted context-specific values
  where ordinary text escaping cannot prevent execution.
- `data-hemx-ws` is rejected. Use SSE or an application-owned socket feeding
  `window.hemx.applyBatch`. Only controls and `data-hemx-param-*` become request
  fields; drag keys use `__drag`. History navigation refetches server HTML.
- Text helpers escape values; only explicit `SafeHtml` introduces markup.
- Batch coalescing retains **every intermediate operation**. It reduces SSE
  envelopes, not DOM operations. A coalesced frame is one batch: an application
  failure stops its remaining effects. Apps must supply valid transitions and
  use replay/snapshot recovery when continuity is lost.

## Design boundary

Hemx owns checked UI effects and their browser runtime. It does not own routing,
databases, authentication, CSS, or application state. Those remain ordinary
Rust and web concerns. Browser-specific behavior belongs in explicit leaf islands
rather than a second application model.

The normal application path is:

- `hemx` for handler and effect APIs;
- `hemx-build` for generated resources;
- `hemx-axum` for Axum integration;
- `hemx-js` for the browser effect runtime.

Server-side Wasm uses the same core crates and canonical effect batches; it does
not require a separate Hemx Wasm runtime package.

## Workspace

| Package | Purpose |
| --- | --- |
| `hemx` | Application-facing facade and handler macro export |
| `hemx-core` | Effect types, protocol values, validation, and runtime primitives |
| `hemx-derive` | Procedural macros |
| `hemx-build` | Build-time template analysis and generated resources |
| `hemx-axum` | Axum routes, responses, assets, and server push |
| `hemx-js` | Browser runtime source |
| `hemx-test` | Test support for applications |

## Agent skill

The optional [`idiomatic-hemx`](https://github.com/tmk241/hemx-skills) skill
helps coding agents apply Hemx's generated-resource and server-owned effect
model:

```console
npx skills@latest add tmk241/hemx-skills --skill idiomatic-hemx
```

## Development

```console
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
cargo deny check licenses sources
```

## Related projects and acknowledgements

[htmx](https://htmx.org/) helped popularize the HTML-over-the-wire,
hypermedia-driven approach that inspired Hemx. Hemx is an independent
implementation and does not bundle htmx.

Hemx builds on [Hemplate](https://github.com/tmk241/hemplate),
[Tokio](https://github.com/tokio-rs/tokio),
[Axum](https://github.com/tokio-rs/axum), and the Rust procedural-macro
ecosystem. Thank you to their maintainers and contributors.

## License

MIT
