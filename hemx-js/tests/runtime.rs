//! Rust-visible runtime asset contract.
//!
//! Runtime behavior is checked in real browser documents by
//! `tests/browser` (`cargo run --locked -p hemx-browser-test`). Searching the
//! bundled source for code fragments is not behavioral evidence, so these tests
//! cover only what Rust callers receive.

#[test]
fn runtime_abi_version_is_explicit_and_stable() {
    assert_eq!(hemx_js::RUNTIME_ABI_VERSION, 3);
}

#[test]
fn runtime_asset_is_content_addressed() {
    use sha2::{Digest, Sha256};

    let hash = format!("{:x}", Sha256::digest(hemx_js::RUNTIME_JS.as_bytes()));
    assert_eq!(hemx_js::RUNTIME_JS_HASH, hash);
    assert_eq!(hemx_js::RUNTIME_JS_PATH, format!("/hemx.{hash}.js"));
}

#[test]
fn runtime_ships_typescript_definitions_for_the_public_api() {
    let source = hemx_js::RUNTIME_D_TS;

    assert!(source.contains("export interface EffectBatch"));
    assert!(source.contains("fingerprint: bigint"));
    assert!(source.contains("export type Effect ="));
    assert!(source.contains("interface Window"));
    // The browser suite checks that window.hemx exposes exactly these members.
    for member in [
        "readonly runtimeAbiVersion: number",
        "roots(): Element[]",
        "rootOf(node: Element | null): Element | null",
        "applyHtml(",
        "applyBatch(",
        "decodeBatch(buffer: ArrayBuffer): EffectBatch",
    ] {
        assert!(source.contains(member), "missing {member}");
    }
}
