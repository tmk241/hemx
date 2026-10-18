use std::collections::BTreeSet;

use hemx::advanced::{MAX_BATCH_BYTES, MAX_EFFECTS, MAX_FIELD_BYTES};
use serde_json::Value;

use crate::checks::case;

fn success(observed: &Value, text_length: usize, dispatches: usize, payload_bytes: usize) {
    assert_eq!(observed["textLength"], text_length);
    assert_eq!(
        observed["textPrefix"],
        if text_length == 5 {
            "after"
        } else {
            "xxxxxxxxxxxxxxxx"
        }
    );
    assert_eq!(observed["dispatches"], dispatches);
    assert_eq!(observed["payloadBytes"], payload_bytes);
    assert_eq!(observed["unchanged"], false);
    assert!(observed["mutations"].as_u64().unwrap() > 0);
    assert_eq!(observed["diagnostics"].as_array().unwrap().len(), 0);
    assert_eq!(observed["documentDiagnostics"], 0);
}

fn failure(observed: &Value) {
    assert_eq!(observed["unchanged"], true);
    assert_eq!(observed["mutations"], 0);
    assert_eq!(observed["textPrefix"], "before");
    assert_eq!(observed["textLength"], 6);
    assert_eq!(observed["dispatches"], 0);
    let diagnostics = observed["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0]["rootScoped"], true);
    assert!(!diagnostics[0]["message"].as_str().unwrap().is_empty());
    assert_eq!(observed["documentDiagnostics"], 1);
}

pub fn assert_wire_report(report: &Value, checked: &mut BTreeSet<String>) {
    // ABI envelope 20 + the 17-byte marker patch + four Dispatch envelopes
    // (tag + length + one-byte event name + payload length). Payload is the
    // remaining exact batch budget, not a JS-authored approximation.
    let batch_payload = MAX_BATCH_BYTES - (20 + 17 + 4 * 10);
    for (name, length, dispatches, payload) in [
        ("small", 5, 0, 0),
        ("string-exact", MAX_FIELD_BYTES, 0, 0),
        ("bytes-exact", 5, 1, MAX_FIELD_BYTES),
        ("batch-exact", 5, 4, batch_payload),
        ("effects-exact", 5, MAX_EFFECTS - 1, 0),
    ] {
        let observed = case(report, checked, &format!("wire-{name}"));
        assert_eq!(observed["accepted"], true, "{name}");
        success(observed, length, dispatches, payload);
    }
    for name in [
        "magic",
        "abi",
        "effect-tag",
        "resource-tag",
        "scope-tag",
        "patch-tag",
        "utf8",
        "truncated-header",
        "truncated-payload",
        "trailing",
        "declared-length",
        "fingerprint",
        "string-over",
        "bytes-over",
        "batch-over",
        "effects-over",
    ] {
        let observed = case(report, checked, &format!("wire-{name}"));
        assert_eq!(observed["accepted"], false, "{name}");
        failure(observed);
        if name == "abi" {
            let code = &observed["diagnostics"][0]["code"];
            assert_eq!(code, "HEMX_UNSUPPORTED_ABI");
        }
        if name == "fingerprint" {
            assert!(observed["diagnostics"][0]["message"]
                .as_str()
                .unwrap()
                .contains("fingerprint"));
        }
    }

    for (name, pulls, cancels, used) in [
        ("small", 3, 0, true),
        ("batch-exact", 3, 0, true),
        ("oversize", 2, 1, true),
        ("length-oversize", 0, 1, true),
        ("length-understated", 2, 1, true),
        ("decode", 3, 0, true),
        ("wire-fingerprint", 3, 0, true),
        // Header/HTTP rejection never reads the body; it must still release it.
        ("header-fingerprint", 0, 1, true),
        ("http-error", 0, 1, true),
        ("stream-error", 1, 0, true),
    ] {
        let observed = case(report, checked, &format!("wire-stream-{name}"));
        assert_eq!(observed["requests"], 1);
        assert_eq!(observed["pulls"], pulls, "stream demand: {name}");
        assert_eq!(observed["cancels"], cancels, "stream cancellation: {name}");
        assert_eq!(observed["locked"], false, "reader released: {name}");
        assert_eq!(observed["bodyUsed"], used, "body consumption: {name}");
        assert_eq!(observed["locationUnchanged"], true);
        match name {
            "small" => success(observed, 5, 0, 0),
            "batch-exact" => success(observed, 5, 4, batch_payload),
            _ => failure(observed),
        }
        let message = observed["diagnostics"][0]["message"].as_str().unwrap_or("");
        match name {
            "http-error" => {
                assert_eq!(observed["diagnostics"][0]["status"], 503);
                assert!(message.contains("503"));
            }
            "header-fingerprint" | "wire-fingerprint" => {
                assert!(message.contains("fingerprint"));
            }
            "oversize" | "length-oversize" | "length-understated" => {
                assert!(message.contains(&MAX_BATCH_BYTES.to_string()));
            }
            "stream-error" => assert!(message.contains("wire stream failure")),
            _ => {}
        }
    }
}
