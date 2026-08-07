# AGENTS.md

## Overview

Hemx is a Rust workspace for checked hypermedia applications. `README.md` is the
user entry point; public APIs and tests establish supported behavior. Keep these
sources consistent.

## Workspace layout

The main application path uses `hemx`, `hemx-build`, and `hemx-axum` on top of
`hemx-core` and `hemx-derive`. Browser and deployment concerns are isolated in
`hemx-js`, `hemx-wasm`, `hemx-host`, and the optional sync packages.

Keep behavior in the package that owns it. Core effect and protocol types belong
in `hemx-core`; framework-specific HTTP behavior belongs in `hemx-axum`.

## Build and test

Run the narrowest relevant test while editing. Before completing a change, run:

```console
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
cargo deny check licenses sources
```

Changes to publishable packages require an archive-based `cargo package` check.

## Code guidelines

- Keep generated APIs typed; avoid stringly typed target and event construction.
- Preserve root-scoped effect application and fail closed on incompatible builds.
- Keep routing, persistence, authentication, styling, and domain state outside core.
- Put custom browser behavior behind explicit leaf-island or host-capability boundaries.
- Add or update a focused test for behavior changes and regressions.
- Keep diagnostics actionable and update public docs when contracts change.

## Repository hygiene

Do not add machine-specific paths, private URLs, credentials, editor-local state,
or generated build output. Do not commit package archives, coverage reports,
benchmark output, `target/`, or temporary planning files.

## Licensing

New dependencies must pass `cargo deny check licenses sources`. Preserve license
and attribution files in source and package archives.
