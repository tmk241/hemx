//! Canonical failure fixtures shared by HTTP, public applyBatch, and SSE.
use super::{batch, patch, wire, BuildFingerprint, Effect, EffectBatch, PatchMode};
use serde_json::{json, Value};

pub fn failed_batch() -> EffectBatch {
    batch(
        vec![
            patch(PatchMode::Morph, "<p>before failure</p>"),
            // Duplicate identities are an application-rendering failure, not a
            // transport failure. The first patch remains; the third must stop.
            patch(
                PatchMode::Morph,
                "<p id=\"duplicate\">one</p><p id=\"duplicate\">two</p>",
            ),
            patch(PatchMode::Morph, "must not apply"),
        ],
        BuildFingerprint(0),
    )
}

pub fn recovery_batch() -> EffectBatch {
    batch(
        vec![Effect::Dispatch {
            event: "error-probe-done".into(),
            payload: vec![],
        }],
        BuildFingerprint(0),
    )
}

pub fn cases() -> Value {
    let mut abi = wire(vec![], BuildFingerprint(0));
    abi[4] = 2;
    json!({
        "batch": failed_batch().to_wire().unwrap(),
        "decode": [0],
        "stale": wire(vec![], BuildFingerprint(1)),
        "recovery": recovery_batch().to_wire().unwrap(),
        // Keep the published ABI-specific code distinct from general decode errors.
        "abi": abi
    })
}
