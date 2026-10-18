use crate::checks::case;
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub fn assert_review(report: &Value, checked: &mut BTreeSet<String>) {
    for name in ["morph", "replace", "rows", "delta"] {
        for mode in ["sequential", "merged", "sse"] {
            let value = case(report, checked, &format!("review-fold-{name}-{mode}"));
            assert!(value["accepted"]
                .as_array()
                .unwrap()
                .iter()
                .all(|v| v == true));
            assert_eq!(value["diagnostics"], json!([]));
            if name == "replace" {
                assert_eq!(value["wrapper"], true);
                assert_eq!(value["text"], "b");
            } else {
                assert_eq!(
                    value["value"], "a",
                    "{name} {mode}: intermediate defaults must not be discarded"
                );
            }
        }
    }
    for name in ["rows-media", "delta-media", "rows-dialog", "delta-dialog"] {
        let value = case(report, checked, &format!("review-move-{name}"));
        assert_eq!(value["accepted"], false);
        assert_eq!(value["unchanged"], true);
        assert_eq!(value["mutations"], 0);
        assert_eq!(value["diagnostics"], 1);
    }
    for mode in ["rows", "delta"] {
        let value = case(report, checked, &format!("review-nested-{mode}"));
        assert_eq!(value["retained"], true);
        assert_eq!(value["accepted"], json!([true, true, false, true]));
        assert_eq!(value["afterStale"], json!(["z"]));
        assert_eq!(value["resyncs"], 1);
        assert_eq!(value["keys"], json!(["z", "w"]));
        assert_eq!(value["diagnostics"], json!([]));
    }
}
