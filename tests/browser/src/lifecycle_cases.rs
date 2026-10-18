use std::collections::BTreeMap;

use hemx::advanced::{BuildFingerprint, Effect, PatchMode};
use serde_json::{json, Value};

// These use the same low-level fixture resource bindings as the main producer;
// every successful response is nevertheless encoded and decoded by Rust.
pub fn cases() -> Value {
    let mut wires = BTreeMap::new();
    for text in ["stale", "fresh", "first", "second", "third"] {
        wires.insert(
            text,
            super::wire(
                vec![Effect::Patch {
                    target: super::target(2),
                    mode: PatchMode::Morph,
                    html: super::html(text),
                }],
                BuildFingerprint(0),
            ),
        );
    }
    for (name, input, indicator) in [
        ("baseline-disabled", "disabled", ""),
        ("baseline-enabled", "", "hidden=\"until-found\""),
    ] {
        let content = format!(
            "<form id=\"work\" method=\"post\" data-hemx-policy=\"parallel\" \
             data-hemx-pending-class=\"loading\" data-hemx-disable-while-pending \
             class=\"server\" aria-busy=\"server\">\
             <input id=\"field\" name=\"value\" value=\"server\" {input}></form>\
             <button id=\"trigger\" type=\"button\" form=\"work\" data-hemx-action=\"7\">Action</button>\
             <span id=\"indicator\" data-hemx-indicator {indicator}>Working</span>\
             <output data-hemx-error hidden></output>"
        );
        wires.insert(
            name,
            super::wire(
                vec![super::patch(PatchMode::Morph, &content)],
                BuildFingerprint(0),
            ),
        );
    }
    json!({ "wire": wires })
}

#[cfg(test)]
mod tests {
    #[test]
    fn lifecycle_responses_round_trip_through_the_real_codec() {
        for value in super::cases()["wire"].as_object().unwrap().values() {
            let bytes: Vec<u8> = serde_json::from_value(value.clone()).unwrap();
            let batch = hemx::advanced::EffectBatch::from_wire(&bytes).unwrap();
            assert_eq!(batch.to_wire().unwrap(), bytes);
        }
    }
}
