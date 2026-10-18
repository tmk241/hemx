use std::collections::BTreeSet;

use serde_json::{json, Value};

use crate::checks::case;

pub fn assert_native_forms(report: &Value, checked: &mut BTreeSet<String>) {
    for mode in ["no-runtime", "missing-navigation", "enhanced"] {
        let scenarios: &[(&str, bool, bool)] = if mode == "enhanced" {
            &[
                ("post", true, false),
                ("post", false, false),
                ("post", true, true),
            ]
        } else {
            &[
                ("get", true, false),
                ("get", false, false),
                ("post", true, false),
                ("post", false, false),
                ("get", true, true),
            ]
        };
        for &(method, valid, override_route) in scenarios {
            let id = format!(
                "{mode}-{method}-{}{}",
                if valid { "success" } else { "invalid" },
                if override_route { "-override" } else { "" }
            );
            let name = format!("native-http-{id}");
            let observed = case(report, checked, &name);
            let server = &observed["server"];
            let enhanced = mode == "enhanced";
            assert_eq!(
                server["submissions"], 1,
                "{name}: no duplicate submission or POST replay"
            );
            assert_eq!(
                server["mutations"],
                u32::from(method == "post" && valid),
                "{name}: exactly one valid POST mutation; validation and GET never mutate"
            );
            assert_eq!(server["enhanced"], u32::from(enhanced), "{name}");
            assert_eq!(server["native"], u32::from(!enhanced), "{name}");
            assert_eq!(server["method"], method.to_uppercase(), "{name}");
            assert_eq!(observed["runtimeLoaded"], mode != "no-runtime", "{name}");
            if mode == "missing-navigation" {
                assert_eq!(observed["navigationAvailable"], false, "{name}");
            }
            let message = if !valid {
                "Value is required"
            } else if method == "post" {
                "Saved"
            } else {
                "Found"
            };
            assert_eq!(observed["status"], message, "{name}");
            assert_eq!(server["message"], message, "{name}");
            assert_eq!(
                observed["contentType"], "text/html",
                "{name}: effect bytes must not become a native document"
            );
            assert_eq!(observed["diagnostics"], json!([]), "{name}");

            let mut fields: Vec<(String, String)> =
                serde_json::from_value(server["fields"].clone()).expect("successful controls");
            fields.sort();
            let mut expected = vec![
                ("__h".to_owned(), "7".to_owned()),
                ("choice".to_owned(), "save".to_owned()),
                (
                    "value".to_owned(),
                    if valid { "a & é" } else { "" }.to_owned(),
                ),
            ];
            for field in [
                "action",
                "method",
                "enctype",
                "target",
                "submit",
                "requestSubmit",
            ] {
                expected.push((field.to_owned(), format!("shadow-{field}")));
            }
            expected.sort();
            assert_eq!(
                fields, expected,
                "{name}: native successful controls exactly once despite named properties"
            );

            assert_eq!(
                observed["navigated"], !enhanced,
                "{name}: native document navigation versus effect update"
            );
            if enhanced {
                assert_eq!(
                    observed["location"], observed["before"],
                    "{name}: POST does not change URL"
                );
                assert_eq!(
                    observed["historyDelta"], 0,
                    "{name}: POST does not change history"
                );
            } else {
                let location = observed["location"].as_str().expect("native destination");
                let path = location
                    .split("/native-forms/")
                    .nth(1)
                    .expect("application-owned native route");
                let destination = if method == "post" && valid {
                    "result"
                } else if override_route {
                    "override"
                } else {
                    "submit"
                };
                assert_eq!(
                    path.split('?').next().unwrap(),
                    format!("{id}/{destination}"),
                    "{name}"
                );
                if method == "get" {
                    assert!(
                        location.contains('?'),
                        "{name}: native GET retains query URL state"
                    );
                }
                if !valid {
                    let accessible = &observed["validationAccessible"];
                    assert_eq!(accessible, true, "{name}");
                }
            }
        }
    }
}
