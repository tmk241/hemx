use std::collections::BTreeSet;

use serde_json::{json, Value};

use crate::checks::case;

pub fn assert_admission(report: &Value, checked: &mut BTreeSet<String>) {
    for name in [
        "late-navigate-veto",
        "manual-data",
        "manual-data-canceled",
        "multipart",
        "image-get",
        "no-precommit",
    ] {
        let observed = case(report, checked, &format!("form-admission-{name}"));
        let canceled = name == "manual-data-canceled";
        let requests = observed["requests"].as_array().unwrap();
        assert_eq!(requests.len(), usize::from(!canceled), "{name}");
        assert_eq!(observed["diagnostics"], json!([]), "{name}");
        // Hemx copies the entries the browser serialized for the navigation and
        // never serializes the form again; only the app's own `new FormData`
        // in "manual-data" counts. Browsers may fire `formdata` more than once
        // for one navigation (Chromium also builds `NavigateEvent.formData`),
        // so the event count is only a lower bound.
        let app_serializations = usize::from(name.starts_with("manual-data"));
        assert_eq!(observed["serialized"], app_serializations, "{name}");
        let native = usize::from(!canceled);
        assert!(
            observed["dataEvents"].as_u64().unwrap() >= (app_serializations + native) as u64,
            "{name}"
        );
        assert_eq!(
            observed["vetoes"],
            usize::from(name == "late-navigate-veto" || canceled)
        );
        if name == "no-precommit" {
            assert_eq!(observed["precommit"], "undefined");
        }
        if name != "image-get" {
            assert_eq!(observed["url"], observed["before"]["url"], "{name}");
            assert_eq!(observed["history"], observed["before"]["history"], "{name}");
            assert_eq!(
                observed["state"], observed["before"]["state"],
                "history state: {name}"
            );
        }
        if canceled {
            continue;
        }
        let request = &requests[0];
        if name == "image-get" {
            assert_eq!(request["method"], "GET");
            assert_eq!(
                request["entries"],
                json!([]),
                "GET carries data in its URL, not a body"
            );
            let url = request["url"].as_str().unwrap();
            let (_, query) = url.split_once('?').unwrap();
            let mut fields: Vec<_> = query.split('&').collect();
            fields.sort_unstable();
            assert_eq!(
                fields,
                ["__h=8", "hook=once", "pick.x=0", "pick.y=0", "value=native"]
            );
            assert_eq!(observed["url"], request["url"]);
        } else {
            assert_eq!(request["method"], "POST");
            let mut expected = vec![
                json!(["value", "native"]),
                json!(["__h", "8"]),
                json!(["choice", "save"]),
                json!(["hook", "once"]),
            ];
            if name == "multipart" {
                expected.insert(
                    2,
                    json!(["attachment", {"name":"proof.txt", "size":10, "type":"text/plain"}]),
                );
                assert!(request["headers"].get("Content-Type").is_none());
            }
            assert_eq!(request["entries"], json!(expected), "{name}");
        }
    }
}
