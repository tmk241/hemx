use std::collections::BTreeSet;

use serde_json::{json, Value};

fn overlay(busy: Value, loading: bool, disabled: Value, hidden: Value) -> Value {
    json!({ "busy": busy, "loading": loading, "disabled": disabled, "hidden": hidden })
}

// Each observation has an independent assertion boundary. A broken case does
// not prevent later cases from being checked or conceal report-set drift.
pub fn assert_lifecycle_report(report: &Value, checked: &mut BTreeSet<String>) {
    let mut failures = Vec::new();
    let mut check = |name: &str, assertion: &dyn Fn(&Value)| {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let observed = crate::checks::case(report, checked, name);
            assertion(observed);
        }));
        if result.is_err() {
            failures.push(name.to_owned());
        }
    };

    for policy in ["latest", "queue", "drop", "parallel"] {
        check(&format!("lifecycle-policy-{policy}"), &|observed| {
            let (admitted, captures, values, final_status) = match policy {
                "latest" => (2, 2, json!([["A"], ["B"]]), "fresh"),
                "queue" => (1, 3, json!([["A"], ["B"], ["C"]]), "third"),
                "drop" => (1, 1, json!([["A"], ["C"]]), "fresh"),
                "parallel" => (2, 2, json!([["A"], ["B"]]), "first"),
                _ => unreachable!(),
            };
            assert_eq!(observed["admitted"], admitted, "{policy}: {observed}");
            assert_eq!(observed["capturedAtAdmission"], captures);
            assert_eq!(
                observed["values"], values,
                "capture once, including queued values"
            );
            let count = values.as_array().unwrap().len();
            assert_eq!(
                observed["captures"], count,
                "one native FormData capture per admitted interaction"
            );
            assert_eq!(observed["handles"], json!(vec![vec!["7"]; count]));
            assert_eq!(
                observed["status"], final_status,
                "{policy}: response ordering"
            );
            assert_eq!(
                observed["final"],
                overlay(Value::Null, false, json!(false), Value::Null)
            );
            assert_eq!(observed["diagnostics"], json!([]));
            if policy == "latest" {
                assert_eq!(observed["aborted"], json!([true, false]));
                assert_eq!(
                    observed["intermediate"],
                    json!({ "status": "initial", "busy": "true", "loading": true, "disabled": false, "hidden": null }),
                    "stale response must not apply or release its successor's overlay"
                );
            } else if policy == "parallel" {
                assert_eq!(observed["aborted"], json!([false, false]));
                assert_eq!(
                    observed["intermediate"],
                    json!({ "status": "second", "busy": "true", "loading": true, "disabled": false, "hidden": null }),
                    "first release must retain the other request's overlay"
                );
            } else {
                assert_eq!(observed["aborted"], json!([false]));
                assert_eq!(
                    observed["intermediate"],
                    json!({ "status": "first", "requests": 2 })
                );
            }
        });
    }
    check("lifecycle-queue-removal", &|observed| {
        assert_eq!(
            observed,
            &json!({ "requests": 1, "connected": false, "busy": null, "loading": false, "diagnostics": [] }),
            "detached queued work must not execute"
        );
    });

    for baseline in ["preserved", "disabled", "enabled"] {
        check(&format!("lifecycle-pending-{baseline}"), &|observed| {
            assert_eq!(
                observed["initial"],
                overlay(
                    json!("false"),
                    baseline == "preserved",
                    json!(baseline == "enabled"),
                    json!("until-found")
                )
            );
            let active = overlay(json!("true"), true, json!(true), Value::Null);
            assert_eq!(observed["overlapping"], active);
            assert_eq!(
                observed["between"], active,
                "one request remains after the server baseline changes"
            );
            let expected = match baseline {
                "preserved" => overlay(json!("false"), true, json!(false), json!("until-found")),
                "disabled" => overlay(json!("server"), false, json!(true), Value::Null),
                "enabled" => overlay(json!("server"), false, json!(false), json!("until-found")),
                _ => unreachable!(),
            };
            assert_eq!(
                observed["final"], expected,
                "restore the current server baseline, not an old snapshot"
            );
            assert_eq!(observed["same"], json!([true, true, true]));
            assert_eq!(observed["requests"], 2);
            assert_eq!(observed["diagnostics"], json!([]));
        });
    }
    for failure in ["http", "decode", "network", "removed"] {
        check(&format!("lifecycle-pending-error-{failure}"), &|observed| {
            assert_eq!(
                observed["during"],
                overlay(json!("true"), true, json!(true), Value::Null)
            );
            assert_eq!(
                observed["final"],
                overlay(json!("false"), false, json!(false), json!("until-found")),
                "failure releases every overlay, including detached controls"
            );
            assert_eq!(observed["connected"], failure != "removed");
            let diagnostics = observed["diagnostics"].as_array().unwrap();
            assert_eq!(
                diagnostics.len(),
                1,
                "one diagnostic per failure: {observed}"
            );
            assert_eq!(diagnostics[0]["owned"], true);
            assert!(!diagnostics[0]["message"].as_str().unwrap().is_empty());
            assert_eq!(observed["outlet"]["hidden"], false);
            assert_eq!(
                observed["outlet"]["text"],
                if failure == "http" {
                    "Request failed (503)"
                } else {
                    "Request failed"
                }
            );
        });
    }

    check("lifecycle-scheduling", &|observed| {
        assert_eq!(
            observed["debounced"],
            json!({ "timers": [70], "requests": 0 })
        );
        assert_eq!(
            observed["retained"], true,
            "moving a connected root retains its scheduled work"
        );
        assert_eq!(observed["configCancelled"], true);
        assert_eq!(observed["debounceCancelled"], true);
        assert_eq!(observed["throttled"], 1);
        assert_eq!(observed["throttleCancelled"], true);
        assert_eq!(observed["finalTimers"], json!([]));
        assert_eq!(observed["debounceRequests"], 2);
        assert_eq!(observed["throttleRequests"], 3);
        let bindings = observed["bindings"].as_object().unwrap();
        assert!(
            !bindings.is_empty(),
            "native listener observation is required"
        );
        for (event, count) in bindings {
            let message = format!("duplicate listener after root movement: {event}");
            assert_eq!(count, 1, "{message}");
        }
        assert_eq!(observed["diagnostics"], json!([]));
    });
    check("lifecycle-polling", &|observed| {
        for (stage, expected) in [
            ("initial", vec![40, 55]),
            ("rescanned", vec![40, 55]),
            ("changed", vec![55, 65]),
            ("precedence", vec![55, 75]),
            ("removed", vec![]),
            ("reattached", vec![55]),
            ("final", vec![]),
        ] {
            assert_eq!(
                observed[stage],
                json!(expected),
                "polling {stage}: {observed}"
            );
        }
        assert_eq!(observed["diagnostics"], json!([]));
    });
    check("lifecycle-polling-ownership", &|observed| {
        for (stage, expected) in [
            ("initial", vec![40, 55]),
            ("island", vec![55]),
            ("restored", vec![40, 55]),
            ("moved", vec![40, 55]),
            ("unowned", vec![55]),
            ("reclaimed", vec![40, 55]),
            ("removedOwner", vec![40]),
            ("final", vec![]),
        ] {
            assert_eq!(
                observed[stage],
                json!(expected),
                "polling ownership {stage}: {observed}"
            );
        }
        assert_eq!(
            observed["island"],
            json!([55]),
            "an island stops its enclosing root's adapter but retains the nested root"
        );
        assert_eq!(observed["diagnostics"], json!([]));
    });
    check("lifecycle-revealed", &|observed| {
        assert_eq!(
            observed["initial"],
            json!({ "reveal": 0, "ignored": 0, "nested": 1, "watching": 1, "ignoredWatching": 0 })
        );
        assert_eq!(observed["duplicateObservations"], 0);
        assert_eq!(observed["first"], 1);
        assert_eq!(
            observed["afterResize"],
            json!({ "requests": 1, "watching": 0 })
        );
        assert_eq!(observed["islandWatching"], 0);
        assert_eq!(observed["reveals"], 4);
        assert_eq!(observed["inserted"], 1);
        assert_eq!(observed["finalWatching"], 0);
        assert_eq!(observed["disconnected"], true);
        assert_eq!(observed["diagnostics"], json!([]));
    });
    assert!(failures.is_empty(), "failed lifecycle cases: {failures:?}");
}
