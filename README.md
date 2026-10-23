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
replace one existing row; ordinary slots remain HTML/text update hosts. Generated
rendering helpers lower the complete resource table, including nested components.

Wire ABI and runtime ABI are version 3; the surface schema is version 2.
Encoding is fallible: `batch.to_wire()?` rejects incompatible or oversized
batches before emitting bytes. Earlier wire ABIs are rejected, not translated.

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

Use the **generated** `heml::page::render(&view)` (where `page` is the template
module), then `hemx_axum::PageResponse::full(..).fingerprint(heml::BUILD_FINGERPRINT)`.
Generated rendering lowers the complete resource/handle table and adds native
form hidden fields; the Axum response stamps the root build marker. Raw
`Hemplate::render` alone does not lower those authoring names and is not the
normal Hemx rendering path. Inside the template, compose children with
`{+ Child::new(...) +}`: Hemplate renders a component directly into the parent's
buffer, while ordinary text values are escaped. The outer generated renderer
lowers the complete nested resource table; do not wrap each child in
`{+= heml::render(&child) =+}`. Raw `{+= ... =+}` is for already-produced,
explicitly trusted HTML, not component composition. `heml` is the generated Rust
module, not another template operator.

The example also shows the async Axum wrapper for `runtime_js()` and typed
`RuntimeScript` template interpolation.

## Migrating from 0.4 to 0.5

Rust 1.91 or newer is required. Use Hemplate 0.4 with hemplate-runtime 0.3;
older runtime crates do not define the context traits used by `RuntimeScript`.

- Wire ABI 3 replaces insertion/removal/movement commands with collection
  `set` and `transition(before, after, changed)` declarations. Rebuild templates,
  server and browser assets together; ABI 2 is rejected rather than translated.
- Generated action forms carry `__b`. Handler dispatch rejects stale builds
  with 409 and cross-site requests with 403 before handlers run. Manual dispatch
  must use `require_build`; an enhanced failed POST is never replayed.
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
