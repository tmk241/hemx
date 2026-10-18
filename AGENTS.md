# Working on Hemx

## Where things belong

- `hemx-core`: effects, typed resources and wire encoding.
- `hemx-build` / `hemx-derive`: template inspection, generated resources and macros.
- `hemx-js`: browser runtime; `hemx-axum`: HTTP and SSE integration.
- `hemx`: application facade; `hemx-test`: public testing helpers.

Keep application state, routing, authentication, persistence and styling outside
core. Use generated resource types rather than copied IDs or selectors. Effects
must stay within their owning root and reject incompatible builds. Custom
browser widgets belong in explicit islands.

## Checks

Add a focused regression test for changed behavior. Before finishing, run:

```console
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
cargo deny check licenses sources
cargo check -p hemx-server-wasm-test --target wasm32-unknown-unknown
```

Verify changed publishable packages with `cargo package`, including its archive
build. Update the relevant README and Rustdoc when APIs or compatibility change.

Keep credentials, machine-local files, temporary plans and build/test output out
of Git. Preserve license and attribution files in both sources and packages.
