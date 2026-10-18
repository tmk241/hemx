use super::*;
use crate::artifact::{Resources, SlotRole};
use std::process::{Command, Output};

fn inspect(source: &str) -> io::Result<Resources> {
    let path = Path::new("templates/roles.heml");
    let surface = surface_for_heml_source(path, source.to_owned())?;
    let mut resources = Resources::default();
    resources.add_surface(Path::new("templates"), path, &surface)?;
    Ok(resources)
}

#[test]
fn slot_roles_are_explicit_and_conflicts_fail_closed() {
    for (source, role) in [
        (
            r#"<ul data-hemx-slot="rows"><template h-for="row in &self.rows" h-key="row.id"><li></li></template></ul>"#,
            SlotRole::Slot,
        ),
        (
            r#"<section data-hemx-slot="rows"></section><aside><template h-for="row in &self.rows" h-key="row.id"><li></li></template></aside>"#,
            SlotRole::Slot,
        ),
        (
            r#"<section data-hemx-slot="rows" data-hemx-key="7"></section>"#,
            SlotRole::Row,
        ),
        (
            r#"<section data-hemx-slot="rows" +data-hemx-key="self.id"></section>"#,
            SlotRole::Row,
        ),
        (
            r#"<template h-for="row in &self.rows" h-key="row.id"><section data-hemx-slot="rows"></section></template>"#,
            SlotRole::Row,
        ),
        (
            r#"<section data-hemx-collection="rows"></section>"#,
            SlotRole::Collection,
        ),
        (
            r#"<template h-for="root in &self.roots"><main data-hemx-root><section data-hemx-collection="rows"></section></main></template>"#,
            SlotRole::Collection,
        ),
        (
            r#"<main data-hemx-root><div data-hemx-collection="rows"></div></main><main data-hemx-root><div data-hemx-collection="rows"></div></main>"#,
            SlotRole::Collection,
        ),
    ] {
        let resources = inspect(source).unwrap();
        assert_eq!(resources.slots["rows"].role, role, "{source}");
    }

    for source in [
        r#"<section data-hemx-slot="rows"></section><li data-hemx-slot="rows" data-hemx-key="7"></li>"#,
        r#"<li data-hemx-slot="rows" data-hemx-key="7"></li><section data-hemx-slot="rows"></section>"#,
        r#"<section data-hemx-collection="rows"></section><li data-hemx-slot="rows" data-hemx-key="7"></li>"#,
        r#"<section data-hemx-slot="rows"></section><div data-hemx-collection="rows"></div>"#,
    ] {
        let error = inspect(source).err().expect("conflicting roles must fail");
        assert!(error.to_string().contains("conflicting slot roles"));
    }
    for (source, reason) in [
        (
            r#"<section data-hemx-slot="row" data-hemx-collection="rows"></section>"#,
            "cannot declare both",
        ),
        (r#"<input data-hemx-collection="rows">"#, "non-void element"),
        (r#"<img data-hemx-collection="rows">"#, "non-void element"),
        (
            r#"<div data-hemx-collection=""></div>"#,
            "expected a Rust identifier",
        ),
    ] {
        let error = inspect(source).err().expect("invalid collection must fail");
        assert!(error.to_string().contains(reason), "{error}");
    }

    let repeated_roots = r#"<template h-for="root in &self.roots"><main data-hemx-root><div data-hemx-collection="rows"></div></main></template>"#;
    assert!(diagnostics_for_heml_source("roots.heml", repeated_roots)
        .unwrap()
        .is_empty());
    assert_eq!(
        generated_targets_for_heml_source("roots.heml", repeated_roots).unwrap(),
        vec![GeneratedTarget {
            kind: "collection".into(),
            name: "rows".into()
        }]
    );

    // Complete-table lowering depends on the existing global collision contract.
    let mut collisions = inspect(r#"<section data-hemx-slot="shared"></section>"#).unwrap();
    let other = Path::new("templates/other.heml");
    let surface = surface_for_heml_source(
        other,
        r#"<section data-hemx-slot="shared"></section>"#.into(),
    )
    .unwrap();
    let error = collisions
        .add_surface(Path::new("templates"), other, &surface)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "duplicate generated identifier `shared` for `roles.heml::shared` and `other.heml::shared`"
    );

    let ordinary = inspect(r#"<section data-hemx-slot="rows"></section>"#).unwrap();
    let row = inspect(r#"<section data-hemx-slot="rows" data-hemx-key="7"></section>"#).unwrap();
    let collection = inspect(r#"<section data-hemx-collection="rows"></section>"#).unwrap();
    assert_eq!(ordinary.slots["rows"].id, row.slots["rows"].id);
    assert_eq!(row.slots["rows"].id, collection.slots["rows"].id);
    assert_ne!(ordinary.fingerprint_parts(), row.fingerprint_parts());
    assert_ne!(row.fingerprint_parts(), collection.fingerprint_parts());
    assert_ne!(ordinary.fingerprint_parts(), collection.fingerprint_parts());
}

#[test]
fn resource_marker_conflicts_report_the_source_and_both_declarations() {
    for (first, second) in [
        ("data-hemx-slot", "data-hemx-collection"),
        ("data-hemx-slot", "data-hemx-form"),
        ("data-hemx-collection", "data-hemx-form"),
    ] {
        for declarations in [
            format!(r#"{first}="one" {second}="two""#),
            format!(r#"{second}="two" {first}="one""#),
        ] {
            let source = format!(r#"<form data-hemx-handle="save" {declarations}></form>"#);
            let error = inspect(&source)
                .err()
                .expect("conflicting markers must fail");
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert_eq!(
                error.to_string(),
                format!("templates/roles.heml: invalid {second} placement; an element cannot declare both `{first}` and `{second}`; both lower to the single `data-hemx-resource` marker; use separate elements")
            );
        }
    }

    // A slot's host tag alone does not establish a conflicting declaration.
    for source in [
        r#"<input data-hemx-slot="value">"#,
        r#"<form data-hemx-slot="body"></form>"#,
        r#"<form data-hemx-slot="body" data-hemx-handle="save"></form>"#,
        r#"<form data-hemx-form="editor" data-hemx-handle="save"></form>"#,
    ] {
        assert!(inspect(source).is_ok(), "valid host rejected: {source}");
    }
}

fn assert_success(output: Output, operation: &str) {
    assert!(
        output.status.success(),
        "{operation} failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn generated_composition_and_collection_capabilities_execute_against_real_crates() {
    let base =
        std::env::temp_dir().join(format!("hemx-build-real-contract-{}", std::process::id()));
    let templates = base.join("templates");
    let out = base.join("out");
    std::fs::create_dir_all(&templates).unwrap();
    for (name, source) in [
        (
            "parent",
            r#"<main data-hemx-root><section data-hemx-slot="outlet"></section><ul data-hemx-collection="rows"></ul><template h-for="row in &self.rows" h-key="row.id"><li data-hemx-slot="repeated"></li></template></main>"#,
        ),
        (
            "child",
            r#"<article><span data-hemx-slot="child_text"></span></article>"#,
        ),
        (
            "grandchild",
            r#"<section data-hemx-slot="grandchild_text"><form data-hemx-form="editor" data-hemx-handle="save"><button>Save</button></form></section>"#,
        ),
    ] {
        std::fs::write(templates.join(format!("{name}.heml")), source).unwrap();
    }
    app()
        .template_dir(&templates)
        .out_dir(&out)
        .global_exports(true)
        .run()
        .unwrap();
    let generated = std::fs::read_to_string(out.join("hemx.generated.rs")).unwrap();
    assert_eq!(generated.matches("const __HEMX_LOWERING_TABLE:").count(), 1);
    assert!(!generated.contains("pub struct Component;"));
    assert!(!generated.contains("__HEMX_LOWERING_TABLE_CHILD"));

    // Build the actual facade and renderer with the workspace lock; no fixture lockfile,
    // installed dependency, guessed rlib hash, or fake effect model is needed.
    let target = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();
    assert_success(
        Command::new("cargo")
            .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap())
            .args([
                "build",
                "--locked",
                "--offline",
                "-p",
                "hemx",
                "-p",
                "hemplate",
                "--target-dir",
            ])
            .arg(&target)
            .output()
            .unwrap(),
        "build real contract dependencies",
    );
    let debug = target.join("debug");
    let compile = |path: &Path, bin: &Path| {
        Command::new("rustc")
            .args([
                "--edition=2021",
                "--crate-name",
                "generated_contract",
                "--extern",
            ])
            .arg(format!("hemx={}", debug.join("libhemx.rlib").display()))
            .arg("--extern")
            .arg(format!(
                "hemplate={}",
                debug.join("libhemplate.rlib").display()
            ))
            .arg("-L")
            .arg(format!("dependency={}", debug.join("deps").display()))
            .arg(path)
            .arg("-o")
            .arg(bin)
            .output()
            .unwrap()
    };
    let common = format!(
        r###"
#![allow(dead_code)]
mod ui {{ {generated} }}
use hemplate::Hemplate;
use hemx::GeneratedTarget;
use hemx::advanced::{{Effect, PatchMode, ResourceRef, ScopeKey}};
struct Parent;
struct Child;
struct Grandchild;
impl Hemplate for Parent {{
    fn render_into(&self, out: &mut dyn std::fmt::Write) -> Result<(), hemplate::RenderError> {{
        out.write_str("<main data-hemx-root><section data-hemx-slot=\"outlet\">")?;
        Child.render_into(out)?;
        out.write_str("</section><ul data-hemx-collection=\"rows\"></ul></main>")?;
        Ok(())
    }}
}}
impl Hemplate for Child {{
    fn render_into(&self, out: &mut dyn std::fmt::Write) -> Result<(), hemplate::RenderError> {{
        out.write_str("<article><span data-hemx-slot=\"child_text\">child</span>")?;
        Grandchild.render_into(out)?;
        out.write_str("</article>")?;
        Ok(())
    }}
}}
impl Hemplate for Grandchild {{
    fn render_into(&self, out: &mut dyn std::fmt::Write) -> Result<(), hemplate::RenderError> {{
        out.write_str("<section data-hemx-slot='grandchild_text'><form data-hemx-form='editor' data-hemx-handle='save'><button>Save</button></form></section>")?;
        Ok(())
    }}
}}
impl hemx::KeyedPartial for Child {{ fn hemx_key(&self) -> String {{ "new-row".into() }} }}
fn assert_lowered(html: &str) {{
    assert!(html.contains(&format!("data-hemx-resource=\"{{}}\"", ui::child::child_text.__hemx_resource_id().id)));
    assert!(html.contains(&format!("data-hemx-resource='{{}}'", ui::grandchild::grandchild_text.__hemx_resource_id().id)));
    assert!(html.contains(&format!("data-hemx-action='{{}}'", ui::grandchild::save.id().id)));
    assert!(!html.contains("data-hemx-slot="));
    assert!(!html.contains("data-hemx-handle="));
    assert!(!html.contains("data-hemx-form="));
    let form = html.split("<form ").nth(1).unwrap().split('>').next().unwrap();
    assert_eq!(form.matches("data-hemx-resource=").count(), 1);
    assert!(form.contains(&format!("data-hemx-resource='{{}}'", ui::grandchild::forms::editor.id().id)));
    assert!(form.contains(&format!("data-hemx-action='{{}}'", ui::grandchild::save.id().id)));
    assert_eq!(html.matches("name=\"__h\"").count(), 1);
}}
"###
    );
    let source = format!(
        r###"{common}
fn main() {{
    let page = ui::parent::page(&Parent);
    assert_lowered(page.as_str());
    assert!(page.as_str().contains(&format!("data-hemx-resource=\"{{}}\" data-hemx-collection=\"\"", ui::parent::rows.__hemx_resource_id().id)));
    assert_eq!(ui::parent::lower(page.as_str()), page.as_str());
    let single = ui::parent::lower("<ul data-hemx-collection='rows'></ul>");
    assert!(single.contains("data-hemx-collection=''"));
    assert!(!single.contains("data-hemx-collection='rows'"));
    let Effect::Patch {{ target, mode: PatchMode::Morph, html }} = ui::parent::outlet.put(&Child) else {{ panic!("slot must patch") }};
    assert_eq!(target, ResourceRef::unscoped(ui::parent::outlet.__hemx_resource_id()));
    assert_lowered(html.as_str());
    let Effect::Rows {{ target, order, rows }} = ui::parent::rows.set([&Child]) else {{ panic!("collection must declare rows") }};
    assert_eq!(target, ResourceRef::unscoped(ui::parent::rows.__hemx_resource_id()));
    assert_eq!(order, vec!["new-row".to_string()]);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "new-row");
    assert_lowered(rows[0].1.as_str());
    let Effect::RowsDelta {{ target, from, to, steps, rows }} = ui::parent::rows.transition(["old-row"], ["old-row", "new-row"], [&Child]) else {{ panic!("transition must declare a delta") }};
    assert_eq!(target, ResourceRef::unscoped(ui::parent::rows.__hemx_resource_id()));
    assert_eq!((from, to), (hemx::advanced::order_hash(["old-row"]), hemx::advanced::order_hash(["old-row", "new-row"])));
    assert_eq!(steps, vec![hemx::advanced::RowStep::Place {{ key: "new-row".into(), before: None }}]);
    assert_eq!(rows.iter().map(|(key, _)| key.as_str()).collect::<Vec<_>>(), ["new-row"]);
    assert_lowered(rows[0].1.as_str());
    for effect in [ui::parent::rows.replace(Child), ui::parent::rows.replace_keyed("new-row", &Child)] {{
        let Effect::Patch {{ target, html, .. }} = effect else {{ panic!("row must patch") }};
        assert_eq!(target, ResourceRef::scoped(ui::parent::rows.__hemx_resource_id(), ScopeKey::KeyValue("new-row".into())));
        assert_lowered(html.as_str());
    }}
    let Effect::Patch {{ target, html, .. }} = ui::parent::repeated.replace_keyed("old-row", &Child) else {{ panic!("row must patch") }};
    assert_eq!(target, ResourceRef::scoped(ui::parent::repeated.__hemx_resource_id(), ScopeKey::KeyValue("old-row".into())));
    assert_lowered(html.as_str());
    let batch = hemx::IntoEffect::into_batch(
        (ui::parent::rows.set([&Child]), ui::parent::outlet.put(&Child)),
        ui::BUILD_FINGERPRINT,
    );
    let wire = batch.to_wire().unwrap();
    assert_eq!(hemx::advanced::EffectBatch::from_wire(&wire).unwrap(), batch);
    assert_eq!(batch.to_wire().unwrap(), wire);
    assert_lowered(ui::page(&Parent).as_str());
    assert_lowered(ui::child::page(&Child).as_str());
}}
"###
    );
    let source_path = base.join("contract.rs");
    let bin = base.join("contract");
    std::fs::write(&source_path, source).unwrap();
    assert_success(
        compile(&source_path, &bin),
        "compile generated composition and capabilities",
    );
    assert_success(
        Command::new(&bin).output().unwrap(),
        "execute generated composition and capabilities",
    );

    for expression in [
        // Rows and ordinary slots cannot declare collections, and the
        // commanding operators no longer exist on collections either.
        "ui::parent::repeated.set([&Child])",
        "ui::parent::repeated.transition([\"a\"], [\"b\"], [&Child])",
        "ui::parent::outlet.transition([\"a\"], [\"b\"], [&Child])",
        "ui::parent::rows.sync([\"key\"], [&Child])",
        "ui::parent::rows.order([\"key\"])",
        "ui::parent::rows.append(Child)",
        "ui::parent::rows.remove(\"key\")",
    ] {
        std::fs::write(
            &source_path,
            format!("{common}\nfn main() {{ let _ = {expression}; }}"),
        )
        .unwrap();
        let result = compile(&source_path, &bin);
        assert!(
            !result.status.success(),
            "forbidden capability compiled: {expression}"
        );
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(
            stderr.contains("no method named"),
            "unexpected compile failure: {stderr}"
        );
    }
    std::fs::remove_dir_all(base).unwrap();
}
