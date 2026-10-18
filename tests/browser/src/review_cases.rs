//! Regression fixtures for the publication-readiness review.
use super::{batch, html, patch, target, wire, BuildFingerprint, Effect, EffectBatch, PatchMode};
use hemx::advanced::{fold_batches, order_hash, RowStep};
use serde_json::{json, Value};

fn rows(id: u32, order: &[&str], supplied: &[(&str, &str)]) -> Effect {
    Effect::Rows {
        target: target(id),
        order: order.iter().map(|key| (*key).into()).collect(),
        rows: supplied
            .iter()
            .map(|(key, value)| ((*key).into(), html(value)))
            .collect(),
    }
}
fn delta(
    id: u32,
    before: &[&str],
    after: &[&str],
    steps: Vec<RowStep>,
    supplied: &[(&str, &str)],
) -> Effect {
    Effect::RowsDelta {
        target: target(id),
        from: order_hash(before),
        to: order_hash(after),
        steps,
        rows: supplied
            .iter()
            .map(|(key, value)| ((*key).into(), html(value)))
            .collect(),
    }
}
fn place(key: &str) -> RowStep {
    RowStep::Place {
        key: key.into(),
        before: None,
    }
}

pub fn sequence(name: &str) -> Option<Vec<EffectBatch>> {
    let operations = match name {
        "morph" => vec![
            patch(PatchMode::Morph, "<input value=\"b\">"),
            patch(PatchMode::Morph, "<input value=\"a\">"),
        ],
        "replace" => vec![
            patch(
                PatchMode::Replace,
                "<section><div data-hemx-resource=\"1\">a</div></section>",
            ),
            patch(PatchMode::Replace, "<div data-hemx-resource=\"1\">b</div>"),
        ],
        "rows" => vec![
            rows(
                1,
                &["a"],
                &[("a", "<li data-hemx-key=\"a\"><input value=\"b\"></li>")],
            ),
            rows(
                1,
                &["a"],
                &[("a", "<li data-hemx-key=\"a\"><input value=\"a\"></li>")],
            ),
        ],
        "delta" => vec![
            delta(
                1,
                &["a"],
                &["a"],
                vec![],
                &[("a", "<li data-hemx-key=\"a\"><input value=\"b\"></li>")],
            ),
            delta(
                1,
                &["a"],
                &["a"],
                vec![],
                &[("a", "<li data-hemx-key=\"a\"><input value=\"a\"></li>")],
            ),
        ],
        _ => return None,
    };
    Some(
        operations
            .into_iter()
            .map(|op| batch(vec![op], BuildFingerprint(0)))
            .collect(),
    )
}

pub fn cases() -> Value {
    let folds: Vec<_> = ["morph", "replace", "rows", "delta"].into_iter().map(|name| {
        let sequence = sequence(name).unwrap();
        let encoded = |batches: Vec<EffectBatch>| batches.into_iter().map(|b| b.to_wire().unwrap()).collect::<Vec<_>>();
        json!({"name":name, "sequential":encoded(sequence.clone()), "merged":encoded(fold_batches(sequence))})
    }).collect();
    let encode = |op| wire(vec![op], BuildFingerprint(0));
    let mut movement = Vec::new();
    for (kind, content) in [
        ("media", "<audio></audio>"),
        ("dialog", "<dialog open>open</dialog>"),
    ] {
        let markup = format!("<li data-hemx-key=\"a\">changed{content}</li>");
        for is_delta in [false, true] {
            let op = if is_delta {
                delta(
                    1,
                    &["a", "b", "c"],
                    &["b", "c", "a"],
                    vec![place("a")],
                    &[("a", &markup)],
                )
            } else {
                rows(1, &["b", "c", "a"], &[("a", &markup)])
            };
            movement.push(json!({"name":format!("{}-{kind}", if is_delta {"delta"} else {"rows"}), "wire":encode(op)}));
        }
    }
    let nested_row = "<li data-hemx-key=\"a\"><ul data-hemx-resource=\"2\" data-hemx-collection><li data-hemx-key=\"z\">z</li></ul></li>";
    json!({"folds":folds,"movement":movement,"nested":{
        "seed": encode(delta(2, &["x"], &["x", "y"], vec![place("y")], &[("y", "<li data-hemx-key=\"y\">y</li>")])),
        "rows": encode(rows(1, &["a"], &[("a", nested_row)])),
        "delta": encode(delta(1, &["a"], &["a"], vec![], &[("a", nested_row)])),
        "stale": encode(delta(2, &["x", "y"], &["x", "y", "w"], vec![place("w")], &[("w", "<li data-hemx-key=\"w\">w</li>")])),
        "current": encode(delta(2, &["z"], &["z", "w"], vec![place("w")], &[("w", "<li data-hemx-key=\"w\">w</li>")]))
    }})
}
