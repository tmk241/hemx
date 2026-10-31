use std::collections::BTreeSet;

use serde_json::{json, Value};

use crate::checks::case;

pub fn assert_admission(report: &Value, checked: &mut BTreeSet<String>) {
    let nested = case(report, checked, "form-admission-nested");
    assert_eq!(
        nested["urls"],
        json!(["/nested-a", "/nested-b"]),
        "a submission started inside another's submit listener keeps both enhanced"
    );
    assert_eq!(nested["url"], nested["before"]);
    for name in [
        "navigate-listener",
        "manual-data",
        "manual-data-canceled",
        "multipart",
        "image-get",
        "no-navigation-api",
        "base-target",
    ] {
        let observed = case(report, checked, &format!("form-admission-{name}"));
        let canceled = name == "manual-data-canceled";
        // Native submissions Hemx leaves alone: no request, no serialization by Hemx.
        let native = name == "base-target";
        let requests = observed["requests"].as_array().unwrap();
        assert_eq!(requests.len(), usize::from(!canceled && !native), "{name}");
        assert_eq!(observed["diagnostics"], json!([]), "{name}");
        // Hemx takes a submission over by serializing it exactly once itself;
        // the browser never serializes it, so `formdata` fires once per
        // serialization. Only the app's own `new FormData` in "manual-data"
        // adds one; a vetoed submission is serialized by nobody.
        let app_serializations = usize::from(name.starts_with("manual-data"));
        let taken = usize::from(!canceled && !native);
        // The browser serializes a native submission itself.
        assert_eq!(observed["serialized"], app_serializations + taken, "{name}");
        let data_events = observed["dataEvents"].as_u64().unwrap() as usize;
        assert!(
            native || data_events == app_serializations + taken,
            "{name}"
        );

        // No document navigation starts, so navigate listeners never see the
        // submission itself.
        assert_eq!(observed["navigations"], 0, "{name}");
        assert_eq!(observed["vetoes"], usize::from(canceled), "{name}");
        if name == "no-navigation-api" {
            assert_eq!(observed["navigationApi"], "undefined");
        }
        if name != "image-get" {
            assert_eq!(observed["url"], observed["before"]["url"], "{name}");
            assert_eq!(observed["history"], observed["before"]["history"], "{name}");
            assert_eq!(
                observed["state"], observed["before"]["state"],
                "history state: {name}"
            );
        }
        if canceled || native {
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
