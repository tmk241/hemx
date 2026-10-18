use std::collections::BTreeSet;

use serde_json::{json, Value};

use crate::checks::case;
use crate::streams::batch;

fn state(observed: &Value, opened: usize, closed: usize, pushed: usize) {
    assert_eq!(
        observed,
        &json!({"opened": opened, "closed": closed, "active": opened - closed, "pushed": pushed})
    );
}

fn frames(observed: &Value, transport: &str, messages: &[(&str, u64)]) {
    let frames = observed.as_array().expect("native frames");
    assert_eq!(frames.len(), messages.len());
    for (frame, (text, fingerprint)) in frames.iter().zip(messages) {
        assert_eq!(frame["trusted"], true, "must be a native MessageEvent");
        assert_eq!(frame["binary"], transport == "ws");
        assert_eq!(
            frame["type"],
            if transport == "sse" && *text != "default" {
                "hemx"
            } else {
                "message"
            }
        );
        let bytes = if *text == "malformed" {
            vec![0]
        } else {
            batch(text, *fingerprint).to_wire().unwrap()
        };
        assert_eq!(
            frame["bytes"],
            json!(bytes),
            "native transport must preserve the complete canonical envelope"
        );
    }
}

fn dispatches(observed: &Value, owners: &[&str]) {
    let values = observed.as_array().expect("native Dispatch observations");
    assert_eq!(values.len(), owners.len());
    for (value, owner) in values.iter().zip(owners) {
        assert_eq!(value["owner"], *owner);
        assert_eq!(value["payload"], json!([0, 128, 255, 195, 40]));
    }
}

pub fn assert_stream_report(report: &Value, checked: &mut BTreeSet<String>, origin: &str) {
    let mut failures = Vec::new();
    let burst = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let observed = case(report, checked, "streams-sse-burst");
        assert_eq!(observed["text"], "200", "the final state arrives");
        let callbacks = observed["callbacks"].as_u64().unwrap();
        assert!(
            (1..=10).contains(&callbacks),
            "200 messages must drain in a few frames, saw {callbacks} observer callbacks"
        );
        assert_eq!(observed["diagnostics"], json!([]));
    }));
    if burst.is_err() {
        failures.push("streams-sse-burst".to_owned());
    }
    let deltas = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let observed = case(report, checked, "streams-sse-delta-burst");
        let mut expected = vec!["k100".to_string()];
        expected.extend((2..=98).step_by(2).map(|n| format!("k{n}")));
        assert_eq!(
            observed["keys"],
            json!(expected),
            "chained deltas reach the exact order"
        );
        let callbacks = observed["callbacks"].as_u64().unwrap();
        assert!(
            (1..=20).contains(&callbacks),
            "151 deltas must drain in a few frames, saw {callbacks} observer callbacks"
        );
        assert_eq!(observed["diagnostics"], json!([]));
    }));
    if deltas.is_err() {
        failures.push("streams-sse-delta-burst".to_owned());
    }
    let patches = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let observed = case(report, checked, "streams-sse-patch-fold");
        assert_eq!(
            observed["tag"], "section",
            "the replacement stays the element"
        );
        assert_eq!(observed["text"], "done");
        assert_eq!(observed["diagnostics"], json!([]));
    }));
    if patches.is_err() {
        failures.push("streams-sse-patch-fold".to_owned());
    }
    for transport in ["sse"] {
        for name in [
            "delivery",
            "lifecycle",
            "malformed",
            "fingerprint",
            "missing-api",
            "origin",
            "protocol",
            "credentials",
        ] {
            let id = format!("{transport}-{name}");
            let key = format!("streams-{id}");
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let observed = case(report, checked, &key);
                let connections = observed["connections"]
                    .as_array()
                    .expect("native connection observations");
                match name {
                    "delivery" => {
                        assert_eq!(
                            observed["first"],
                            json!({"outer": "delivered", "island": "island", "nested": "nested"})
                        );
                        assert_eq!(
                            observed["final"],
                            json!({"outer": if transport == "sse" { "default" } else { "delivered" }, "island": "island", "nested": "delivered"})
                        );
                        assert_eq!(
                            observed["ownership"],
                            json!({"outer": true, "island": true, "nested": true})
                        );
                        assert_eq!(connections.len(), 2);
                        for suffix in ["delivery", "nested"] {
                            let expected =
                                format!("{origin}/streams/{transport}-{suffix}/{transport}");
                            let expected = if transport == "ws" {
                                expected.replacen("http:", "ws:", 1)
                            } else {
                                expected
                            };
                            let connection = connections
                                .iter()
                                .find(|c| c["url"] == expected)
                                .expect("owned stream URL");
                            assert_eq!(connection["opens"], 1);
                            assert_eq!(connection["closes"], 1);
                            let mut messages = vec![("delivered", 0)];
                            if transport == "sse" && suffix == "delivery" {
                                messages.push(("default", 0));
                            }
                            frames(&connection["frames"], transport, &messages);
                        }
                        let outer = observed["outerDispatches"].as_array().unwrap();
                        let expected_texts = if transport == "sse" {
                            vec!["delivered", "default"]
                        } else {
                            vec!["delivered"]
                        };
                        assert_eq!(outer.len(), expected_texts.len());
                        for (event, text) in outer.iter().zip(expected_texts) {
                            assert_eq!(
                                event["text"], text,
                                "Patch precedes Dispatch within the owned root"
                            );
                            assert_eq!(event["payload"], json!([0, 128, 255, 195, 40]));
                        }
                        dispatches(
                            &observed["dispatches"],
                            if transport == "sse" {
                                &["owner", "nested", "owner"]
                            } else {
                                &["owner", "nested"]
                            },
                        );
                        state(
                            &observed["closed"],
                            1,
                            1,
                            if transport == "sse" { 2 } else { 1 },
                        );
                        state(&observed["nestedClosed"], 1, 1, 1);
                        assert_eq!(observed["diagnostics"], json!([]));
                    }
                    "lifecycle" => {
                        let stages = &observed["stages"];
                        for (stage, opened, closed, pushed) in [
                            ("moved", 1, 0, 0),
                            ("changed", 2, 1, 1),
                            ("removed", 2, 2, 1),
                            ("readded", 3, 2, 1),
                            ("unrooted", 3, 3, 1),
                            ("rerooted", 4, 3, 1),
                            ("detached", 4, 4, 1),
                            ("reinserted", 5, 4, 2),
                            ("final", 5, 5, 2),
                        ] {
                            state(&stages[stage], opened, closed, pushed);
                        }
                        assert_eq!(connections.len(), 5);
                        for connection in connections {
                            assert_eq!(connection["opens"], 1);
                            assert_eq!(connection["closes"], 1);
                        }
                        frames(&connections[0]["frames"], transport, &[("delivered", 0)]);
                        for connection in &connections[1..4] {
                            frames(&connection["frames"], transport, &[]);
                        }
                        frames(&connections[4]["frames"], transport, &[("recovered", 0)]);
                        dispatches(&observed["dispatches"], &["owner", "owner"]);
                        assert_eq!(observed["text"], "recovered");
                        assert_eq!(observed["diagnostics"], json!([]));
                    }
                    "malformed" | "fingerprint" => {
                        let rejection = &observed["rejection"];
                        assert_eq!(rejection["unchanged"], true);
                        assert_eq!(rejection["mutations"], 0);
                        assert_eq!(rejection["dispatches"], 0);
                        let diagnostics = rejection["diagnostics"].as_array().unwrap();
                        assert_eq!(diagnostics.len(), 1);
                        assert_eq!(diagnostics[0]["owner"], "owner");
                        assert_eq!(diagnostics[0]["type"], "hemx:error");
                        let message = diagnostics[0]["message"].as_str().unwrap();
                        assert!(!message.is_empty());
                        if name == "fingerprint" {
                            assert!(message.contains("fingerprint"));
                        }
                        assert_eq!(
                            observed["diagnostics"], rejection["diagnostics"],
                            "recovery must not add another error"
                        );
                        assert_eq!(observed["text"], "recovered");
                        dispatches(&observed["dispatches"], &["owner"]);
                        assert_eq!(connections.len(), 1);
                        assert_eq!(connections[0]["opens"], 1);
                        assert_eq!(connections[0]["closes"], 1);
                        frames(
                            &connections[0]["frames"],
                            transport,
                            &[
                                (
                                    if name == "malformed" {
                                        "malformed"
                                    } else {
                                        "delivered"
                                    },
                                    if name == "fingerprint" { 9 } else { 0 },
                                ),
                                ("recovered", 0),
                            ],
                        );
                        state(&observed["closed"], 1, 1, 2);
                    }
                    _ => {
                        assert!(
                            connections.is_empty(),
                            "invalid admission must never call the native constructor"
                        );
                        state(&observed["state"], 0, 0, 0);
                        assert_eq!(observed["text"], "before");
                        dispatches(&observed["dispatches"], &[]);
                        let diagnostics = observed["diagnostics"].as_array().unwrap();
                        assert_eq!(diagnostics.len(), 1);
                        assert_eq!(diagnostics[0]["owner"], "owner");
                        assert_eq!(
                            diagnostics[0]["type"],
                            if transport == "sse" {
                                "hemx:sse-error"
                            } else {
                                "hemx:websocket-error"
                            }
                        );
                        let message = diagnostics[0]["message"].as_str().unwrap();
                        assert!(message.contains(if name == "missing-api" {
                            if transport == "sse" {
                                "EventSource"
                            } else {
                                "WebSocket support"
                            }
                        } else {
                            "same-origin"
                        }));
                    }
                }
            }));
            println!(
                "{}",
                json!({"phase": "stream-check", "case": key, "success": result.is_ok()})
            );
            if result.is_err() {
                failures.push(key);
            }
        }
    }
    assert!(
        failures.is_empty(),
        "failed native stream cases: {failures:?}"
    );
}
