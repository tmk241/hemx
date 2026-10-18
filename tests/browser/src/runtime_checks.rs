use std::collections::BTreeSet;

use serde_json::{json, Value};

use crate::checks::case;

pub fn assert_runtime(report: &Value, checked: &mut BTreeSet<String>, origin: &str) {
    let observed = case(report, checked, "runtime-confirm");
    assert_eq!(observed["prompts"], json!(["Delete?", "Delete?"]));
    assert_eq!(
        observed["requests"], 1,
        "a declined confirmation must not dispatch"
    );
    assert_eq!(observed["credentials"], json!(["same-origin"]));
    assert_eq!(observed["diagnostics"], json!([]));

    let observed = case(report, checked, "runtime-pagehide");
    assert_eq!(observed["busy"], "true");
    assert_eq!(observed["reason"], "AbortError");
    assert_eq!(
        observed["diagnostics"],
        json!([]),
        "page-hide cancellation is not an application error"
    );

    let observed = case(report, checked, "runtime-drag-drop");
    // The dragged key travels as reserved __drag; only data-hemx-param-* adds
    // fields, so the ordinary data-tooltip stays DOM metadata.
    assert_eq!(
        observed,
        &json!({"overAccepted": true, "text": "7", "entries": [["__h", "9"], ["__drag", "7"], ["list", "inbox"]]})
    );

    let observed = case(report, checked, "runtime-page-swap");
    assert_eq!(
        observed["afterNavigation"],
        json!({"path": "/page-two", "historyDelta": 1, "title": "Page two", "content": "page two", "nav": "two"})
    );
    // Back navigation shows the server's current page one, not a stored copy.
    assert_eq!(
        observed["afterBack"],
        json!({"title": "Page one", "content": "page one, refetched", "nav": "one"})
    );
    let stored = observed["storedState"].as_str().unwrap();
    assert!(
        !stored.contains("page"),
        "no page HTML in history state: {stored}"
    );
    let request = |path: &str| {
        json!({"url": format!("{origin}{path}"), "credentials": "same-origin",
            "headers": {"X-HEMX-Partial": "1", "Accept": "text/html"}})
    };
    assert_eq!(
        observed["requests"],
        json!([request("/page-two"), request("/frame")])
    );
    assert_eq!(observed["diagnostics"], json!([]));

    let observed = case(report, checked, "runtime-native-navigation");
    assert_eq!(
        observed["handled"],
        json!({"ctrl": false, "blank": false, "download": false, "external": false, "prevented": true, "plain": true})
    );
    assert_eq!(observed["requests"], json!([format!("{origin}/plain")]));
    assert_eq!(observed["diagnostics"], json!([]));

    for name in ["integrity-stale-action", "integrity-stale-form"] {
        let observed = case(report, checked, name);
        assert_eq!(observed["requests"], 1, "{name}: never replayed");
        let errors = observed["errors"].as_array().unwrap();
        assert_eq!(errors.len(), 1, "{name}: one diagnostic: {errors:?}");
        assert!(
            errors[0].as_str().unwrap().contains("reload this page"),
            "{name}"
        );
        assert_eq!(
            observed["outlet"],
            json!({"text": "This page is out of date. Reload to continue.", "hidden": false}),
            "{name}"
        );
        assert_eq!(observed["unchanged"], true, "{name}");
        assert_eq!(observed["sameUrl"], true, "{name}");
        assert_eq!(observed["sameHistory"], true, "{name}");
        assert_eq!(observed["busy"], 0, "{name}");
    }
    // A 409 without the server's build header is the application's own conflict.
    let observed = case(report, checked, "integrity-app-conflict");
    assert_eq!(observed["requests"], 1);
    assert_eq!(observed["errors"].as_array().unwrap().len(), 1);
    assert_eq!(
        observed["outlet"],
        json!({"text": "Request failed (409)", "hidden": false})
    );

    let observed = case(report, checked, "runtime-popstate-fallback");
    assert_eq!(
        observed,
        &json!({"reloaded": true, "path": "/frame", "requests": [format!("{origin}/frame")]})
    );
}
