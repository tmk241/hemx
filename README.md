# Hemx

Hemx is checked hypermedia for Rust. Applications render Hemplate views, handle
events in typed Rust functions, and return generated UI effects. The browser runs
a small effect interpreter instead of a virtual DOM, hydration framework, or
client-side expression language.

## How it works

1. `.heml` templates declare page roots, slots, forms, handles, and keyed targets.
2. `hemx-build` generates typed Rust helpers from that surface.
3. `#[hemx::handler]` functions accept ordinary Rust inputs and return typed effects.
4. `hemx-axum` serves pages, assets, handler routes, and effect responses.
5. The browser runtime validates the build fingerprint and applies effects within the current root.

```rust,ignore
#[hemx::handler]
async fn add_todo(form: NewTodo) -> impl IntoEffect {
    ui::todos().append(TodoRow::from(form))
}
```

```html
<form data-hemx-form="new_todo">
  <input name="title" required>
  <button type="submit">Add</button>
</form>
<ul data-hemx-slot="todos"></ul>
```

## Design boundary

Hemx owns checked UI effects and their browser runtime. It does not own routing,
databases, authentication, CSS, or application state. Those remain ordinary
Rust and web concerns. Browser-specific behavior belongs in explicit leaf islands
rather than a second application model.

The normal application path is:

- `hemx` for handler and effect APIs;
- `hemx-build` for generated resources;
- `hemx-axum` for Axum integration;
- `hemx-sync` when an application needs durable command/event/projection sync;
- `hemx-host` for explicit browser or native-shell capabilities.

## Workspace

| Package | Purpose |
| --- | --- |
| `hemx` | Application-facing facade and handler macro export |
| `hemx-core` | Effect types, protocol values, validation, and runtime primitives |
| `hemx-derive` | Procedural macros |
| `hemx-build` | Build-time template analysis and generated resources |
| `hemx-axum` | Axum routes, responses, assets, and server push |
| `hemx-js` | Browser runtime source |
| `hemx-wasm` | Client-local WASM integration |
| `hemx-host` | Explicit host capability boundary |
| `hemx-sync` | Optional durable sync primitives |
| `hemx-sync-macros` | Derives for sync domain types |
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

Hemx builds on [Hemplate](https://github.com/tmk241/hemplate),
[Tokio](https://github.com/tokio-rs/tokio),
[Axum](https://github.com/tokio-rs/axum), and the Rust procedural-macro
ecosystem. Thank you to their maintainers and contributors.

## License

MIT
