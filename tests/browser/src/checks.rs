use std::collections::BTreeSet;

use serde_json::{json, Value};

pub(crate) fn case<'a>(report: &'a Value, checked: &mut BTreeSet<String>, name: &str) -> &'a Value {
    assert!(
        checked.insert(name.to_owned()),
        "duplicate assertion group: {name}"
    );
    let value = report
        .get(name)
        .unwrap_or_else(|| panic!("missing browser case: {name}"));
    assert!(value.get("probeError").is_none(), "{name}: {value}");
    value
}

fn single_request(observed: &Value) -> &Value {
    let requests = observed
        .as_array()
        .expect("request observations must be an array");
    assert_eq!(requests.len(), 1, "expected one request: {observed}");
    &requests[0]
}

fn assert_get(request: &Value, origin: &str, path: &str, pairs: &[&str], location: &Value) {
    assert_eq!(request["method"], "GET");
    let url = request["url"].as_str().unwrap();
    let (destination, query) = url.split_once('?').expect("GET form must have a query");
    assert_eq!(destination, format!("{origin}{path}"));
    let mut actual: Vec<_> = query.split('&').collect();
    let mut expected = pairs.to_vec();
    actual.sort_unstable();
    expected.sort_unstable();
    assert_eq!(
        actual, expected,
        "successful controls must occur exactly once"
    );
    assert_eq!(
        location, url,
        "GET submission must update native history URL state"
    );
}

fn check_group(name: &str, failures: &mut Vec<String>, check: impl FnOnce()) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(check));
    println!(
        "{}",
        json!({"phase": "checks", "group": name, "success": result.is_ok()})
    );
    if result.is_err() {
        failures.push(name.to_owned());
    }
}

pub fn assert_report(report: &Value, origin: &str) {
    let mut checked = BTreeSet::new();
    let mut failures = Vec::new();
    check_group("request deadlines", &mut failures, || {
        crate::timeouts::assert_timeouts(report, &mut checked);
    });
    check_group("runtime entrypoint", &mut failures, || {
        let observed = case(report, &mut checked, "runtime-entrypoint");
        assert_eq!(observed["abi"], hemx_js::RUNTIME_ABI_VERSION);
        assert_eq!(
            observed["keys"],
            json!([
                "applyBatch",
                "applyHtml",
                "decodeBatch",
                "rootOf",
                "roots",
                "runtimeAbiVersion"
            ])
        );
        assert_eq!(observed["frozen"], true);
        assert_eq!(observed["roots"], 1);
        assert_eq!(observed["owner"], true);
        let bindings = observed["bindingReadiness"].as_array().unwrap();
        assert!(!bindings.is_empty() && bindings.iter().all(|ready| ready == true));
        assert_eq!(observed["after"], observed["before"]);
        assert_eq!(observed["requests"], 0);
        assert_eq!(observed["storage"], 0);
        assert_eq!(observed["diagnostics"], json!([]));
    });
    check_group("runtime requests and navigation", &mut failures, || {
        crate::runtime_checks::assert_runtime(report, &mut checked, origin);
    });
    check_group("closed effects", &mut failures, || {
        let observed = case(report, &mut checked, "effect-focus-control");
        assert_eq!(
            observed,
            &json!({"accepted": true, "focused": "target", "scrollY": 0, "diagnostics": []})
        );
        let observed = case(report, &mut checked, "effect-focus-radio");
        assert_eq!(
            observed,
            &json!({"accepted": true, "focused": "checked", "diagnostics": []})
        );
        let observed = case(report, &mut checked, "effect-scroll");
        assert_eq!(
            observed,
            &json!({"accepted": true, "aligned": true, "focused": "focus", "diagnostics": []})
        );
        let observed = case(report, &mut checked, "effect-visit");
        assert_eq!(observed["accepted"], true);
        assert_eq!(observed["path"], "/effects-visit");
        assert_eq!(observed["historyDelta"], 1);
        let request = single_request(&observed["requests"]);
        assert_eq!(request["method"], "GET");
        assert_eq!(
            request["url"], "/effects-visit",
            "server-authored relative Visit URL"
        );
        assert_eq!(request["headers"]["X-HEMX-Partial"], "1");
        assert_eq!(observed["credentials"], json!(["same-origin"]));
        assert_eq!(observed["diagnostics"], json!([]));
        let observed = case(report, &mut checked, "effect-batch-prefix");
        // Effects are ordered, not transactional: the prefix remains, the
        // failed target stops the batch, and the later patch never runs.
        assert_eq!(
            observed,
            &json!({"accepted": false, "text": "prefix", "diagnostics": ["missing target"]})
        );
    });
    check_group("DOM effects", &mut failures, || {
        // Rows leave exactly the declared keys, in order, keeping retained rows.
        let observed = case(report, &mut checked, "rows-reorder");
        assert_eq!(observed["ids"], json!(["c", "a", "b"]));
        assert_eq!(observed["retained"], json!([true, true, true]));
        assert_eq!(observed["diagnostics"], 0);
        let observed = case(report, &mut checked, "rows-declare");
        assert_eq!(observed["keys"], json!(["a", "d"]));
        assert_eq!(
            observed["retained"],
            json!([true, false]),
            "a reconciled in place, d created"
        );
        assert_eq!(observed["text"], "A2D");
        // A delta moves, creates and reconciles exactly as declared, from the
        // order the browser hashes itself.
        let observed = case(report, &mut checked, "delta-apply");
        assert_eq!(observed["accepted"], true);
        assert_eq!(observed["ids"], json!(["c", "a", "d"]));
        assert_eq!(observed["retained"], json!([true, true, false]));
        assert_eq!(observed["text"], "CA2D");
        assert_eq!(observed["diagnostics"], 0);
        let observed = case(report, &mut checked, "delta-unicode");
        assert_eq!(observed["keys"], json!(["\u{e4}"]));
        assert_eq!(observed["diagnostics"], 0);
        let observed = case(report, &mut checked, "delta-reverse");
        let reversed: Vec<String> = (0..20).rev().map(|n| format!("k{n}")).collect();
        assert_eq!(observed["ids"], json!(reversed));
        assert_eq!(observed["retained"], json!(vec![true; 20]));
        assert_eq!(observed["diagnostics"], 0);
        let observed = case(report, &mut checked, "delta-skip");
        assert_eq!(
            (
                &observed["accepted"],
                &observed["mutations"],
                &observed["diagnostics"]
            ),
            (&json!(true), &json!(0), &json!(0)),
            "a delta at its result is a duplicate"
        );
        let observed = case(report, &mut checked, "delta-drift");
        assert_eq!(
            (
                &observed["accepted"],
                &observed["mutations"],
                &observed["diagnostics"]
            ),
            (&json!(false), &json!(0), &json!(0)),
            "a delta from another order changes nothing; it heals rather than reports"
        );
        // The refetched page brings the rows the client missed; a delta that
        // still finds another order afterwards is reported, never refetched again.
        let observed = case(report, &mut checked, "delta-heal");
        assert_eq!(observed["ids"], json!(["x", "y"]));
        assert_eq!(observed["requests"], json!(["GET"]));
        assert_eq!(observed["resyncs"], 1);
        assert_eq!(observed["diagnostics"], 0);
        assert_eq!(observed["afterHeal"], true, "a delta applies after healing");
        let observed = case(report, &mut checked, "delta-heal-repeat");
        assert_eq!(observed["requests"], json!(["GET"]), "one refetch only");
        assert_eq!(observed["diagnostics"], 1);
        assert_eq!(observed["ids"], json!(["a", "b", "c"]));
        // Only a delta that applied clears the guard. A skipped duplicate does not,
        // so the same drift is reported instead of refetched again; and a refetch
        // finishing after a delta applied must not put the guard back.
        let observed = case(report, &mut checked, "delta-heal-skip");
        assert_eq!(observed["requests"], json!(["GET"]));
        assert_eq!(observed["diagnostics"], 1);
        let observed = case(report, &mut checked, "delta-heal-applied");
        assert_eq!(observed["applied"], true);
        assert_eq!(observed["requests"], json!(["GET", "GET"]));
        assert_eq!(observed["diagnostics"], 0);
        // A reconciled row that changes tag is a new element and must be placed.
        for name in ["rows-retag", "delta-retag"] {
            let observed = case(report, &mut checked, name);
            assert_eq!(observed["accepted"], true, "{name}");
            assert_eq!(observed["tags"], json!(["li#a", "section#b"]), "{name}");
            assert_eq!(observed["text"], "AB2", "{name}");
            assert_eq!(observed["diagnostics"], 0, "{name}");
        }
        // One unusable row among valid ones changes nothing.
        for name in ["rows-preflight", "delta-preflight"] {
            let observed = case(report, &mut checked, name);
            assert_eq!(
                (
                    &observed["accepted"],
                    &observed["mutations"],
                    &observed["diagnostics"]
                ),
                (&json!(false), &json!(0), &json!(1)),
                "{name}: nothing may change"
            );
            assert_eq!(observed["text"], "ABC", "{name}");
        }
        let observed = case(report, &mut checked, "delta-bom");
        assert_eq!(observed["ids"], json!(["a"]));
        assert_eq!(observed["diagnostics"], 0);
        for (name, ids) in [
            ("delta-sequence", json!(["c", "d"])),
            ("delta-invalidated", json!(["x", "y"])),
        ] {
            let observed = case(report, &mut checked, name);
            assert_eq!(observed["ids"], ids, "{name}");
            assert_eq!(observed["diagnostics"], 0, "{name}");
        }
        // Natively, or by reinsertion when the row holds nothing reinsertion
        // would lose: same node, same unsent value.
        for name in [
            "delta-move-native",
            "delta-move-reinserted",
            "morph-move-reinserted",
        ] {
            let observed = case(report, &mut checked, name);
            assert_eq!(observed["ids"], json!(["b", "c", "a"]), "{name}");
            assert_eq!(observed["same"], true, "{name}");
            assert_eq!(observed["value"], "unsent edit", "{name}");
            assert_eq!(observed["diagnostics"], 0, "{name}");
        }
        // Morphing may reach the order by moving the stateless rows instead:
        // the focused row stays put and keeps its focus.
        let observed = case(report, &mut checked, "morph-move-focused");
        assert_eq!(observed["ids"], json!(["b", "c", "a"]));
        assert_eq!(observed["focused"], true);
        assert_eq!(observed["value"], "unsent edit");
        assert_eq!(observed["diagnostics"], 0);
        // A delta that must move a row holding focus, a selection or media
        // changes nothing at all.
        for name in [
            "delta-move-focused",
            "delta-move-selected",
            "delta-move-selectednode",
            "delta-move-media",
            "morph-move-stateful",
        ] {
            let observed = case(report, &mut checked, name);
            assert_eq!(
                (
                    &observed["accepted"],
                    &observed["mutations"],
                    &observed["diagnostics"]
                ),
                (&json!(false), &json!(0), &json!(1)),
                "{name}: state would be lost, so no change at all"
            );
        }
        for name in ["rows-missing", "rows-unkeyed-child"] {
            let observed = case(report, &mut checked, name);
            assert_eq!(observed["accepted"], false, "{name}");
            assert_eq!(
                observed["mutations"], 0,
                "{name}: rejected before any change"
            );
            assert_eq!(observed["ids"], json!(["a", "b", "c"]), "{name}");
            assert_eq!(observed["diagnostics"], 1, "{name}");
        }
        for name in ["island-top", "island-nested"] {
            let observed = case(report, &mut checked, name);
            assert_eq!(
                observed,
                &json!({"same": true, "connected": true, "value": "client edit"}),
                "{name}"
            );
        }
        let observed = case(report, &mut checked, "html-island");
        assert_eq!(observed, &json!({"same": true, "value": "client edit"}));

        for (mode, value, same) in [
            ("unchanged", "unsent edit", true),
            ("changed", "changed", true),
            ("replace", "server", false),
        ] {
            let observed = case(report, &mut checked, &format!("input-{mode}"));
            assert_eq!(observed["beforeFocused"], true, "focus precondition");
            assert_eq!(observed["same"], same, "input identity: {mode}");
            assert_eq!(observed["value"], value, "input value: {mode}");
            assert_eq!(observed["focused"], same, "input focus: {mode}");
            assert_eq!(
                observed["defaultValue"],
                if mode == "changed" {
                    "changed"
                } else {
                    "server"
                }
            );
            if mode == "unchanged" {
                assert_eq!(observed["selection"], json!([2, 5, "backward"]));
                assert_eq!(
                    observed["invalid"], "true",
                    "unrelated server attributes still update"
                );
            }
        }
        for mode in ["native", "unavailable"] {
            let observed = case(report, &mut checked, &format!("move-focus-{mode}"));
            for field in ["beforeFocused", "same", "focused"] {
                assert_eq!(observed[field], true, "Move {mode}: {field}");
            }
            assert_eq!(observed["value"], "unsent edit");
            assert_eq!(observed["selection"], json!([2, 5, "backward"]));
            if mode == "unavailable" {
                assert_eq!(observed["capability"], "undefined");
                assert_eq!(observed["moved"], false);
                assert_eq!(observed["mutations"], 0);
                assert_eq!(
                    observed["diagnostics"]
                        .as_array()
                        .expect("diagnostics")
                        .len(),
                    1
                );
            } else {
                assert_eq!(observed["capability"], "function");
                assert_eq!(observed["moved"], true);
                assert_eq!(observed["diagnostics"], json!([]));
            }
        }
    });
    check_group("native control reconciliation", &mut failures, || {
        for host in ["inner", "host"] {
            for mode in ["unchanged", "changed"] {
                let observed = case(report, &mut checked, &format!("textarea-{host}-{mode}"));
                for field in ["accepted", "same", "focused"] {
                    assert_eq!(observed[field], true, "{field}");
                }
                assert_eq!(
                    observed["value"],
                    if mode == "changed" {
                        "changed"
                    } else {
                        "unsent edit"
                    }
                );
                assert_eq!(
                    observed["defaultValue"],
                    if mode == "changed" {
                        "changed"
                    } else {
                        "server"
                    }
                );
                if mode == "unchanged" {
                    assert_eq!(observed["selection"], json!([2, 5, "backward"]));
                }
                assert_eq!(
                    observed["invalid"],
                    if host == "host" {
                        json!("true")
                    } else {
                        Value::Null
                    }
                );
            }
        }
        for variant in [
            "single",
            "none",
            "multiple",
            "multiple-none",
            "changed",
            "reordered",
            "duplicate-values",
        ] {
            let observed = case(report, &mut checked, &format!("select-{variant}"));
            for field in ["accepted", "same", "sameOptions", "focused"] {
                assert_eq!(observed[field], true, "select {variant}: {field}");
            }
            assert_eq!(
                observed["selected"],
                if variant.ends_with("none") {
                    json!([])
                } else {
                    json!(["b"])
                },
                "select {variant}"
            );
            assert_eq!(
                observed["defaults"],
                if variant == "changed" {
                    json!(["b"])
                } else {
                    json!(["a"])
                }
            );
            assert_eq!(
                observed["order"],
                if ["reordered", "duplicate-values"].contains(&variant) {
                    json!(["b", "a"])
                } else {
                    json!(["a", "b"])
                }
            );
        }
        let observed = case(report, &mut checked, "file-morph");
        assert_eq!(
            observed,
            &json!({
                "accepted": true, "same": true, "focused": true, "count": 1,
                "name": "proof.txt", "content": "local!", "label": "updated"
            })
        );
        for mode in ["unchanged", "changed"] {
            let observed = case(report, &mut checked, &format!("radio-{mode}"));
            assert_eq!(observed["accepted"], true);
            assert_eq!(observed["same"], true);
            assert_eq!(
                observed["checked"],
                if mode == "changed" {
                    json!([])
                } else {
                    json!(["b"])
                }
            );
            assert_eq!(
                observed["defaults"],
                if mode == "changed" {
                    json!([])
                } else {
                    json!(["a"])
                }
            );
        }
        let observed = case(report, &mut checked, "radio-ownership");
        assert_eq!(
            observed,
            &json!({"accepted": false, "mutations": 0, "owned": false, "foreign": true, "diagnostics": 1})
        );
    });
    check_group("native field clearing", &mut failures, || {
        for kind in [
            "text", "textarea", "checkbox", "radio", "select", "multiple", "file", "external",
        ] {
            let observed = case(report, &mut checked, &format!("clear-{kind}"));
            assert_eq!(
                observed,
                &json!({
                    "accepted": true, "same": true, "markupUnchanged": true, "cleared": true,
                    "selected": if ["select", "multiple"].contains(&kind) { json!([-1]) } else { json!([]) },
                    "files": if kind == "file" { json!(0) } else { Value::Null }
                }),
                "clear {kind}"
            );
        }
        let observed = case(report, &mut checked, "clear-ownership");
        assert_eq!(
            observed,
            &json!({"accepted": false, "value": "keep", "selected": 0, "options": 2, "diagnostics": 1})
        );
    });
    check_group("ownership and lifecycle", &mut failures, || {
        for (name, expected) in [("island-events-owned", 0), ("island-events-nested-root", 1)] {
            let observed = case(report, &mut checked, name);
            assert_eq!(observed, &json!(expected), "{name}");
        }
        let observed = case(report, &mut checked, "moved-root");
        assert_eq!(observed["before"], 1, "request precondition");
        assert_eq!(
            observed["after"], 1,
            "one interaction after moving an existing root"
        );
        let bindings = observed["bindings"].as_object().unwrap();
        assert!(bindings.contains_key("nested:click"));
        for (event, count) in bindings {
            assert_eq!(count, 1, "duplicate listener: {event}");
        }
        let observed = case(report, &mut checked, "throttle");
        assert_eq!(observed, &json!({"burst": 1, "afterInterval": 2}));
        let observed = case(report, &mut checked, "removed-debounce");
        assert_eq!(observed, 0);
    });
    check_group("submit cancellation", &mut failures, || {
        let mut observations = serde_json::Map::new();
        let mut expected = serde_json::Map::new();
        for guard in ["form", "document", "window"] {
            for button in [false, true] {
                let name = format!("submit-cancel-{guard}-{button}");
                observations.insert(name.clone(), case(report, &mut checked, &name).clone());
                expected.insert(name, json!({"submissions": 2, "requests": 0}));
            }
        }
        assert_eq!(observations, expected);
    });
    check_group("native form data and navigation", &mut failures, || {
        let observed = case(report, &mut checked, "native-validation");
        assert_eq!(observed, &json!({"invalid": 1, "requests": 0}));
        let observed = case(report, &mut checked, "submitter");
        assert_eq!(
            observed["invalid"], 0,
            "formnovalidate must use the native submit lifecycle"
        );
        assert_eq!(observed["formdata"], 1);
        assert_eq!(observed["diagnostics"], json!([]));
        assert_get(
            single_request(&observed["requests"]),
            origin,
            "/alternate",
            &[
                "action=shadow",
                "method=shadow",
                "enctype=shadow",
                "target=shadow",
                "empty=",
                "choice=save",
                "hook=once",
                "__h=7",
            ],
            &observed["location"],
        );
        let request = single_request(case(report, &mut checked, "named-form-properties"));
        assert_eq!(request["url"], format!("{origin}/submit"));
        assert_eq!(request["method"], "POST");
        assert_eq!(
            request["entries"],
            json!([
                ["action", "action-value"],
                ["method", "method-value"],
                ["enctype", "enctype-value"],
                ["target", "target-value"],
                ["__h", "7"]
            ])
        );
        let observed = case(report, &mut checked, "clobbered-root-form");
        for field in [
            "accepted",
            "shadowed",
            "same",
            "sameFields",
            "sibling",
            "focused",
        ] {
            assert_eq!(observed[field], true, "clobbered form: {field}");
        }
        assert_eq!(observed["className"], "after");
        assert_eq!(observed["value"], "unsent edit");
        assert_eq!(observed["selection"], json!([2, 5, "backward"]));
        assert_eq!(observed["diagnostics"], json!([]));
        let request = single_request(&observed["requests"]);
        assert_eq!(request["url"], format!("{origin}/submit"));
        assert_eq!(request["method"], "POST");
        let mut entries: Vec<_> = [
            "action",
            "method",
            "enctype",
            "target",
            "attributes",
            "children",
            "childNodes",
            "nodeType",
            "nodeName",
            "tagName",
            "localName",
            "namespaceURI",
            "ownerDocument",
            "isConnected",
            "parentNode",
            "parentElement",
            "firstChild",
            "nextSibling",
            "getAttribute",
            "hasAttribute",
            "setAttribute",
            "removeAttribute",
            "classList",
            "innerHTML",
            "addEventListener",
            "dispatchEvent",
            "querySelectorAll",
            "elements",
            "id",
            "submit",
            "requestSubmit",
            "reset",
            "focus",
            "contains",
            "replaceWith",
            "scrollIntoView",
        ]
        .into_iter()
        .map(|name| json!([name, name]))
        .collect();
        entries.push(json!(["__h", "7"]));
        assert_eq!(request["entries"], json!(entries));
        let observed = case(report, &mut checked, "submitter-handle");
        assert_eq!(observed["submissions"], 1);
        assert_eq!(
            single_request(&observed["requests"])["entries"],
            json!([["__h", "8"], ["choice", "alternate"]])
        );
        let observed = case(report, &mut checked, "external-submitter");
        assert_eq!(observed["diagnostics"], json!([]));
        assert_get(
            single_request(&observed["requests"]),
            origin,
            "/external",
            &["value=ok", "choice=external", "__h=7"],
            &observed["location"],
        );
    });
    check_group("generated resources", &mut failures, || {
        let observed = case(report, &mut checked, "escaped-text");
        assert_eq!(
            observed,
            &json!({"accepted": true, "text": "<b data-x=\"&\">é'</b>", "elements": 0})
        );
        let observed = case(report, &mut checked, "composition");
        assert_eq!(
            observed,
            &json!({
                "symbolicAttributes": 0, "numericAction": true, "dispatchMatches": true,
                "nestedStatus": "updated", "diagnostics": []
            })
        );
        for (name, keys, success) in [
            ("append", vec!["old", "new"], true),
            ("prepend", vec!["new", "old"], true),
            ("append-empty", vec!["new"], true),
            ("set", vec!["a", "b"], true),
            ("set-none", vec![], true),
            ("remove-all", vec![], true),
            ("conflicting-key", vec!["old"], false),
            ("multiple-elements", vec!["old"], false),
            ("missing-element", vec!["old"], false),
            ("drifted", vec!["old"], false),
            ("delta-duplicate", vec!["old", "new"], true),
            ("insert-remove", vec!["old"], true),
            ("insert-remove-empty", vec![], true),
            ("insert-move-before", vec!["new", "old"], true),
            ("replace", vec!["old"], true),
            ("replace-conflicting-key", vec!["old"], false),
        ] {
            let observed = case(report, &mut checked, &format!("keyed-{name}"));
            assert_eq!(
                observed["nested"], 0,
                "no keyed row inside a keyed row: {name}"
            );
            assert_eq!(observed["keys"], json!(keys), "keyed insertion: {name}");
            assert_eq!(
                observed["status"],
                if success { "after" } else { "before" },
                "batch continuation: {name}"
            );
            // A drifted delta reports nothing yet: it refetches the page once.
            let healing = name == "drifted";
            assert_eq!(
                observed["diagnostics"].as_array().unwrap().len(),
                usize::from(!success && !healing),
                "{name}"
            );
            assert_eq!(observed["requests"], usize::from(healing), "{name}");
        }
        // The row is the keyed element itself: same node, new content, no nesting.
        let observed = &report["keyed-replace"];
        assert_eq!(observed["sameFirst"], true);
        assert_eq!(observed["text"], "Changed");
        let observed = &report["keyed-replace-conflicting-key"];
        assert_eq!(
            observed["text"], "Row old",
            "a rejected replacement changes nothing"
        );
    });
    check_group("native form admission", &mut failures, || {
        crate::form_checks::assert_admission(report, &mut checked);
    });
    check_group("wire boundaries", &mut failures, || {
        crate::wire_checks::assert_wire_report(report, &mut checked);
    });
    check_group("localized errors", &mut failures, || {
        crate::errors::assert_errors(report, &mut checked);
    });
    check_group("native HTTP forms", &mut failures, || {
        crate::native_form_checks::assert_native_forms(report, &mut checked);
    });
    check_group("request policy and lifecycle", &mut failures, || {
        crate::lifecycle_checks::assert_lifecycle_report(report, &mut checked);
    });
    check_group("native SSE and WebSocket", &mut failures, || {
        crate::stream_checks::assert_stream_report(report, &mut checked, origin);
    });
    check_group("publication review regressions", &mut failures, || {
        crate::review_checks::assert_review(report, &mut checked);
    });
    check_group("report coverage", &mut failures, || {
        let actual: BTreeSet<_> = report.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            checked, actual,
            "every executed observation must have an assertion; no missing, extra, or fatal cases"
        );
    });
    assert!(failures.is_empty(), "failed browser groups: {failures:?}");
}

#[cfg(test)]
mod tests {
    #[test]
    fn incomplete_browser_reports_fail() {
        assert!(std::panic::catch_unwind(|| super::assert_report(
            &serde_json::json!({}),
            "http://127.0.0.1:1"
        ))
        .is_err());
    }
}
