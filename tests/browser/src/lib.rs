mod lifecycle_cases;
pub mod review_cases;
mod wire_cases;

use std::collections::BTreeMap;

use hemplate::Hemplate;
use hemx::advanced::{
    BuildFingerprint, Effect, EffectBatch, HistoryMode, PatchMode, ResourceId, ResourceKind,
    ResourceRef, SafeHtml, ScopeKey, ScrollBehavior, ScrollBlock, EFFECT_BATCH_ABI_VERSION,
};
use serde_json::{json, Value};

#[hemx::surface]
pub mod heml {}

#[derive(Hemplate)]
pub struct List {
    todos: Vec<Row>,
}

#[derive(Hemplate)]
pub struct Row {
    id: String,
    title: String,
}

impl hemx::KeyedPartial for Row {
    fn hemx_key(&self) -> String {
        self.id.clone()
    }
}

#[derive(Hemplate)]
pub struct ConflictingRow;

#[derive(Hemplate)]
pub struct MultipleRows;

#[derive(Hemplate)]
pub struct Parent;

#[derive(Hemplate)]
pub struct Child {
    grandchild: Grandchild,
}

#[derive(Hemplate)]
pub struct Grandchild;

struct Markup<'a>(&'a str);

impl Hemplate for Markup<'_> {
    fn render_into(&self, out: &mut dyn std::fmt::Write) -> Result<(), hemplate::RenderError> {
        out.write_str(self.0)?;
        Ok(())
    }
}

fn html(value: &str) -> SafeHtml {
    hemx::page(&Markup(value)).into()
}

// Low-level protocol fixtures use fixed resource bindings. The keyed and
// composition cases below separately exercise real generated application APIs.
fn target(id: u32) -> ResourceRef {
    ResourceRef {
        resource: ResourceId {
            kind: ResourceKind::Slot,
            id,
        },
        scope: None,
    }
}

fn batch(ops: Vec<Effect>, fingerprint: BuildFingerprint) -> EffectBatch {
    EffectBatch {
        abi_version: EFFECT_BATCH_ABI_VERSION,
        fingerprint,
        ops,
    }
}

fn wire(ops: Vec<Effect>, fingerprint: BuildFingerprint) -> Vec<u8> {
    let batch = batch(ops, fingerprint);
    let encoded = batch.to_wire().expect("valid browser fixture wire");
    assert_eq!(EffectBatch::from_wire(&encoded).unwrap(), batch);
    encoded
}

fn patch(mode: PatchMode, content: &str) -> Effect {
    Effect::Patch {
        target: target(1),
        mode,
        html: html(content),
    }
}

fn row(id: &str) -> Row {
    Row {
        id: id.into(),
        title: format!("Row {id}"),
    }
}

pub fn cases() -> Value {
    let mut wires = BTreeMap::new();
    let fingerprint = BuildFingerprint(0);
    wires.insert("empty", wire(vec![], fingerprint));
    for (name, ops) in [
        (
            "focus-control",
            vec![Effect::Focus {
                target: target(1),
                prevent_scroll: true,
            }],
        ),
        (
            "focus-radio",
            vec![Effect::Focus {
                target: ResourceRef {
                    resource: ResourceId {
                        kind: ResourceKind::Form,
                        id: 2,
                    },
                    scope: Some(ScopeKey::Field("choice".into())),
                },
                prevent_scroll: true,
            }],
        ),
        (
            "scroll-native",
            vec![Effect::Scroll {
                target: target(1),
                block: ScrollBlock::Start,
                behavior: ScrollBehavior::Instant,
            }],
        ),
        (
            "visit-push",
            vec![Effect::Visit {
                url: "/effects-visit".into(),
                history: HistoryMode::Push,
            }],
        ),
        (
            "batch-prefix-failure",
            vec![
                patch(PatchMode::Morph, "prefix"),
                Effect::Focus {
                    target: target(99),
                    prevent_scroll: false,
                },
                patch(PatchMode::Morph, "unreachable"),
            ],
        ),
    ] {
        wires.insert(name, wire(ops, fingerprint));
    }
    // Rows on a fixed collection (resource 1) holding keys a, b, c.
    let rows = |order: &[&str], supplied: &[(&str, &str)]| Effect::Rows {
        target: target(1),
        order: order.iter().map(|key| key.to_string()).collect(),
        rows: supplied
            .iter()
            .map(|(key, markup)| (key.to_string(), html(markup)))
            .collect(),
    };
    for (name, effect) in [
        ("rows-reorder", rows(&["c", "a", "b"], &[])),
        (
            "rows-declare",
            rows(
                &["a", "d"],
                &[("a", "<li id=\"a\">A2</li>"), ("d", "<li id=\"d\">D</li>")],
            ),
        ),
        ("rows-missing", rows(&["a", "z"], &[])),
        ("rows-unkeyed-child", rows(&["a"], &[])),
        // a must move behind b and c; b and c keep their relative order.
        ("rows-move-last", rows(&["b", "c", "a"], &[])),
    ] {
        wires.insert(name, wire(vec![effect], fingerprint));
    }
    // RowsDelta on the same collection; the browser must agree with
    // hemx_core::order_hash on every starting order, including non-ASCII keys.
    let delta = |before: &[&str], after: &[&str], supplied: &[(&str, &str)]| {
        hemx::advanced::Slot::<()>::new(1).rows_delta_html(
            before.iter().copied(),
            after.iter().copied(),
            supplied.iter().map(|(key, markup)| (*key, html(markup))),
        )
    };
    let twenty_keys: Vec<String> = (0..20).map(|n| format!("k{n}")).collect();
    let twenty: Vec<&str> = twenty_keys.iter().map(String::as_str).collect();
    let twenty_reversed: Vec<&str> = twenty.iter().rev().copied().collect();
    let named = |from: &[&str], to: &[&str]| Effect::RowsDelta {
        target: target(1),
        from: hemx::advanced::order_hash(from.iter().copied()),
        to: hemx::advanced::order_hash(to.iter().copied()),
        steps: vec![hemx::advanced::RowStep::Remove("a".into())],
        rows: vec![],
    };
    for (name, effect) in [
        (
            "delta-apply",
            delta(
                &["a", "b", "c"],
                &["c", "a", "d"],
                &[("a", "<li id=\"a\">A2</li>"), ("d", "<li id=\"d\">D</li>")],
            ),
        ),
        ("delta-step2", delta(&["c", "a", "d"], &["c", "d"], &[])),
        // Already at its result: skipped. Elsewhere: rejected.
        ("delta-skip", named(&["x"], &["a", "b", "c"])),
        ("delta-drift", named(&["x"], &["y"])),
        (
            "delta-move-last",
            delta(&["a", "b", "c"], &["b", "c", "a"], &[]),
        ),
        (
            "delta-from-x",
            delta(&["x"], &["x", "y"], &[("y", "<li id=\"y\">Y</li>")]),
        ),
        (
            "delta-unicode",
            delta(&["todo-7", "\u{e4}"], &["\u{e4}"], &[]),
        ),
        // Enough steps to take the indexed lookup path.
        ("delta-reverse", delta(&twenty, &twenty_reversed, &[])),
        // A reconciled row whose tag changes is a new element: placement must use it.
        (
            "rows-retag",
            rows(
                &["a", "b"],
                &[
                    ("a", "<li id=\"a\">A</li>"),
                    ("b", "<section id=\"b\">B2</section>"),
                ],
            ),
        ),
        (
            "delta-retag",
            delta(
                &["b"],
                &["a", "b"],
                &[
                    ("a", "<li id=\"a\">A</li>"),
                    ("b", "<section id=\"b\">B2</section>"),
                ],
            ),
        ),
        // A valid update for row a, an unusable one for row b: nothing may change.
        (
            "rows-preflight",
            rows(
                &["a", "b"],
                &[
                    ("a", "<li id=\"a\">A2</li>"),
                    ("b", "<li id=\"b\"><i id=\"d\">1</i><i id=\"d\">2</i></li>"),
                ],
            ),
        ),
        (
            "delta-preflight",
            delta(
                &["a", "b", "c"],
                &["b", "c", "a"],
                &[
                    ("a", "<li id=\"a\">A2</li>"),
                    ("b", "<li id=\"b\"><i id=\"d\">1</i><i id=\"d\">2</i></li>"),
                ],
            ),
        ),
        // A leading U+FEFF is part of a key, in the wire and in the order hash.
        ("delta-bom", delta(&["\u{feff}x", "a"], &["a"], &[])),
        (
            "patch-rows-bca",
            patch(
                PatchMode::Morph,
                "<li id=\"b\" data-hemx-key=\"b\">B</li><li id=\"c\" data-hemx-key=\"c\">C</li><li id=\"a\" data-hemx-key=\"a\"><input value=\"server\"></li>",
            ),
        ),
        (
            "patch-rows-ba",
            patch(
                PatchMode::Morph,
                "<li id=\"b\" data-hemx-key=\"b\"><video></video></li><li id=\"a\" data-hemx-key=\"a\"><input value=\"server\"></li>",
            ),
        ),
        (
            "patch-rows-x",
            patch(PatchMode::Morph, "<li id=\"x\" data-hemx-key=\"x\">X</li>"),
        ),
    ] {
        wires.insert(name, wire(vec![effect], fingerprint));
    }
    for (name, mode, markup) in [
        ("island-top", PatchMode::Morph, "<div data-hemx-island=\"editor\"><input value=\"replacement\"></div>"),
        ("island-nested", PatchMode::Morph, "<article><div data-hemx-island=\"editor\"><input value=\"replacement\"></div></article>"),
        ("input-unchanged", PatchMode::Morph, "<input id=\"edit\" value=\"server\" aria-invalid=\"true\">"),
        ("input-changed", PatchMode::Morph, "<input id=\"edit\" value=\"changed\" aria-invalid=\"true\">"),
        ("input-replace", PatchMode::Replace, "<section data-hemx-resource=\"1\"><input id=\"edit\" value=\"server\"></section>"),
    ] {
        wires.insert(name, wire(vec![patch(mode, markup)], fingerprint));
    }

    let initial = heml::page(&List {
        todos: vec![row("old")],
    })
    .into_string();
    let empty = heml::page(&List { todos: vec![] }).into_string();
    // Low-level Rows on the generated collection id, for rendered rows that a
    // `KeyedPartial` view cannot express (invalid shapes, conflicting keys).
    let raw_rows = |order: &[&str], key: &str, view: &dyn Fn() -> hemx::Html| {
        hemx::advanced::Slot::<String>::new(
            hemx::GeneratedTarget::__hemx_resource_id(heml::list::row).id,
        )
        .rows_html(order.iter().copied(), [(key, view())])
    };
    let none: Vec<&str> = Vec::new();
    let unchanged: Vec<&Row> = Vec::new();
    let keyed: Vec<_> = [
        (
            "append",
            vec![heml::list::row.transition(["old"], ["old", "new"], [&row("new")])],
        ),
        (
            "prepend",
            vec![heml::list::row.transition(["old"], ["new", "old"], [&row("new")])],
        ),
        (
            "append-empty",
            vec![heml::list::row.transition(none.clone(), ["new"], [&row("new")])],
        ),
        ("set", vec![heml::list::row.set([&row("a"), &row("b")])]),
        ("set-none", vec![heml::list::row.set(Vec::<&Row>::new())]),
        (
            "remove-all",
            vec![heml::list::row.transition(["old"], none.clone(), unchanged.clone())],
        ),
        (
            "conflicting-key",
            vec![raw_rows(&["old", "new"], "new", &|| {
                heml::page(&ConflictingRow)
            })],
        ),
        (
            "multiple-elements",
            vec![raw_rows(&["old", "new"], "new", &|| {
                heml::page(&MultipleRows)
            })],
        ),
        (
            "missing-element",
            vec![hemx::advanced::Slot::<String>::new(
                hemx::GeneratedTarget::__hemx_resource_id(heml::list::row).id,
            )
            .rows_html(["old", "ghost"], Vec::<(String, hemx::Html)>::new())],
        ),
        (
            // The page holds "old", so a change starting from "ghost" missed something.
            "drifted",
            vec![heml::list::row.transition(["ghost"], ["ghost", "new"], [&row("new")])],
        ),
        (
            // The same change twice: the second finds its result and is skipped.
            "delta-duplicate",
            vec![
                heml::list::row.transition(["old"], ["old", "new"], [&row("new")]),
                heml::list::row.transition(["old"], ["old", "new"], [&row("new")]),
            ],
        ),
        (
            "insert-remove",
            vec![
                heml::list::row.transition(["old"], ["old", "new"], [&row("new")]),
                heml::list::row.transition(["old", "new"], ["old"], unchanged.clone()),
            ],
        ),
        (
            "insert-remove-empty",
            vec![
                heml::list::row.transition(none.clone(), ["new"], [&row("new")]),
                heml::list::row.transition(["new"], none.clone(), unchanged.clone()),
            ],
        ),
        (
            "insert-move-before",
            vec![
                heml::list::row.transition(["old"], ["old", "new"], [&row("new")]),
                heml::list::row.transition(["old", "new"], ["new", "old"], unchanged.clone()),
            ],
        ),
        (
            "replace",
            vec![heml::list::row.replace(Row {
                id: "old".into(),
                title: "Changed".into(),
            })],
        ),
        (
            "replace-conflicting-key",
            vec![heml::list::row.replace_keyed("old", &ConflictingRow)],
        ),
    ]
    .into_iter()
    .map(|(name, mut ops)| {
        ops.push(heml::list::status.set("after"));
        let initial = if name.ends_with("-empty") {
            &empty
        } else {
            &initial
        };
        json!({"name": name, "initial": initial, "wire": wire(ops, heml::BUILD_FINGERPRINT)})
    })
    .collect();
    wires.insert(
        "composition",
        wire(
            vec![
                heml::parent::host.put(&Child {
                    grandchild: Grandchild,
                }),
                heml::grandchild::nested_status.set("updated"),
            ],
            heml::BUILD_FINGERPRINT,
        ),
    );
    wires.insert("generated-empty", wire(vec![], heml::BUILD_FINGERPRINT));
    wires.insert(
        "text",
        wire(
            vec![heml::grandchild::nested_status.set("<b data-x=\"&\">é'</b>")],
            heml::BUILD_FINGERPRINT,
        ),
    );
    wires.insert(
        "clear-field",
        wire(
            vec![hemx::Form::<String>::new(1).clear_field("field")],
            fingerprint,
        ),
    );
    for (name, content) in [
        ("textarea-unchanged", "server"),
        ("textarea-changed", "changed"),
        ("select-unchanged", "<option id=\"a\" value=\"a\" selected>A</option><option id=\"b\" value=\"b\">B</option>"),
        ("select-changed", "<option id=\"a\" value=\"a\">A</option><option id=\"b\" value=\"b\" selected>B</option>"),
        ("select-reordered", "<option id=\"b\" value=\"b\">B</option><option id=\"a\" value=\"a\" selected>A</option>"),
    ] {
        wires.insert(name, wire(vec![patch(PatchMode::Morph, content)], fingerprint));
    }

    json!({
        "wire": wires,
        "review": review_cases::cases(),
        "wireBoundary": wire_cases::cases(),
        "lifecycle": lifecycle_cases::cases(),
        "keyed": keyed,
        "compositionInitial": heml::page(&Parent).into_string(),
        "textInitial": heml::page(&Grandchild).into_string(),
        "generatedFingerprint": heml::BUILD_FINGERPRINT.0.to_string(),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn generated_browser_cases_have_valid_wire() {
        let cases = super::cases();
        for bytes in cases["wire"].as_object().unwrap().values() {
            let bytes: Vec<u8> = serde_json::from_value(bytes.clone()).unwrap();
            assert!(super::EffectBatch::from_wire(&bytes).is_ok());
        }
        for case in cases["keyed"].as_array().unwrap() {
            let bytes: Vec<u8> = serde_json::from_value(case["wire"].clone()).unwrap();
            assert!(super::EffectBatch::from_wire(&bytes).is_ok());
        }
    }
}
