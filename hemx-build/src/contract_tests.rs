// These contract fixtures test semantics independently of canonical layout.
// Each builder explicitly disables formatting; lib.rs owns format enforcement tests.
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
fn templates_outside_derive_root_fail_before_emitting_artifacts() {
    let base = std::env::temp_dir().join(format!("hemx-build-custom-root-{}", std::process::id()));
    let templates = base.join("views");
    std::fs::create_dir_all(&templates).unwrap();
    let path = templates.join("panel.heml");
    let source = r#"<section data-hemx-slot="panel"></section>"#;
    std::fs::write(&path, source).unwrap();
    let surface = surface_for_heml_source(&path, source.to_owned()).unwrap();
    for supplied in [false, true] {
        let out = base.join(if supplied { "supplied" } else { "scanned" });
        let mut builder = app()
            .check_format(false)
            .template_dir(&templates)
            .manifest_dir(&base)
            .out_dir(&out);
        if supplied {
            builder = builder.surface(&path, surface.clone());
        }
        let error = builder.run().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        let message = error.to_string();
        assert!(message.contains(&path.display().to_string()), "{message}");
        assert!(
            message.contains("Hemplate") && message.contains("templates/"),
            "{message}"
        );
        assert!(message.contains("move"), "{message}");
        assert!(!out.join("hemx.generated.rs").exists());
    }
    // Selecting a subdirectory is supported: derive still finds its transform
    // relative to templates/, not relative to template_dir.
    let nested = base.join("templates/nested");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(nested.join("panel.heml"), source).unwrap();
    let out = base.join("nested-out");
    app()
        .check_format(false)
        .template_dir(&nested)
        .manifest_dir(&base)
        .out_dir(&out)
        .run()
        .unwrap();
    assert!(hemplate_core::transform::path_for(
        &out.join("hemplate-transform"),
        Path::new("nested/panel.heml")
    )
    .is_file());
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn timeout_static_values_fail_build_with_source_diagnostics() {
    let base =
        std::env::temp_dir().join(format!("hemx-build-timeout-invalid-{}", std::process::id()));
    let templates = base.join("templates");
    let out = base.join("out");
    std::fs::create_dir_all(&templates).unwrap();
    let path = templates.join("timeout.heml");
    for value in [
        None,
        Some(""),
        Some(" "),
        Some("0"),
        Some("0ms"),
        Some("0s"),
        Some("1.5s"),
        Some("1.0ms"),
        Some("1sec"),
        Some("1MS"),
        Some("ms"),
        Some("-1"),
        Some("+1"),
        Some("1 ms"),
        Some("\u{0085}1000ms"),
        Some("1000ms\u{0085}"),
        Some("١"),
        Some("2147483648"),
        Some("2147483648ms"),
        Some("2147484s"),
        Some("18446744073709551616ms"),
        Some("18446744073709551615s"),
    ] {
        let attr = value.map_or_else(
            || "data-hemx-timeout".to_owned(),
            |value| format!(r#"data-hemx-timeout="{value}""#),
        );
        std::fs::write(
            &path,
            format!(r#"<form data-hemx-handle="save" {attr}></form>"#),
        )
        .unwrap();
        let error = app()
            .check_format(false)
            .template_dir(&templates)
            .manifest_dir(&base)
            .out_dir(&out)
            .run()
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            error.to_string(),
            format!(
                "{}: invalid data-hemx-timeout value `{}`; expected positive whole milliseconds like `250`/`250ms` or seconds like `120s`, within 1..2147483647ms",
                path.display(),
                value.unwrap_or("")
            )
        );
    }
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn timeout_static_boundaries_and_dynamic_values_are_allowed() {
    for attr in [
        "",
        r#"data-hemx-timeout="1""#,
        r#"data-hemx-timeout="1ms""#,
        r#"data-hemx-timeout="1s""#,
        r#"data-hemx-timeout="120s""#,
        r#"data-hemx-timeout=" 120s ""#,
        "data-hemx-timeout=\"\u{feff}1000ms\u{feff}\"",
        r#"data-hemx-timeout="0001ms""#,
        r#"data-hemx-timeout="2147483647""#,
        r#"data-hemx-timeout="2147483647ms""#,
        r#"data-hemx-timeout="2147483s""#,
        r#"+data-hemx-timeout="self.timeout""#,
    ] {
        for tag in ["form", "button"] {
            let source = format!(r#"<{tag} data-hemx-handle="save" {attr}></{tag}>"#);
            assert!(inspect(&source).is_ok(), "rejected {source}");
        }
    }
    // The stricter timeout bounds must not change existing duration attributes.
    for attr in ["debounce", "throttle", "every", "interval"] {
        for value in ["0", "2147483648ms", "18446744073709551616s"] {
            let source =
                format!(r#"<button data-hemx-handle="save" data-hemx-{attr}="{value}"></button>"#);
            assert!(
                inspect(&source).is_ok(),
                "rejected legacy duration {source}"
            );
        }
    }
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

    // Names resolved at compile time cannot come from runtime values.
    for name in ["slot", "collection", "handle", "form"] {
        let error = inspect(&format!(r#"<div +data-hemx-{name}="self.name"></div>"#))
            .err()
            .expect("dynamic resource name must fail");
        assert!(
            error.to_string().starts_with(&format!(
                "templates/roles.heml: `+data-hemx-{name}` must be a static name"
            )),
            "{error}"
        );
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
            r#"<main data-hemx-root><section data-hemx-slot="outlet">{+ Child +}</section><ul data-hemx-collection="rows"></ul><template h-for="row in &self.rows" h-key="row"><li data-hemx-slot="repeated"></li></template></main>"#,
        ),
        (
            "child",
            r#"<article><span data-hemx-slot="child_text">child</span>{+ Grandchild +}</article>"#,
        ),
        (
            "grandchild",
            r#"<section data-hemx-slot='grandchild_text'><form data-hemx-form='editor' data-hemx-handle='save' data-hemx-timeout='120s'><button>Save</button></form></section>"#,
        ),
        ("trusted", r#"<p>{+ self.text +}{+= self.raw =+}</p>"#),
    ] {
        std::fs::write(templates.join(format!("{name}.heml")), source).unwrap();
    }
    app()
        .check_format(false)
        .template_dir(&templates)
        .manifest_dir(&base)
        .out_dir(&out)
        .run()
        .unwrap();
    let generated = std::fs::read_to_string(out.join("hemx.generated.rs")).unwrap();
    assert!(!generated.contains("pub struct Component;"));
    assert!(!generated.contains("__HEMX_LOWERING_TABLE"));

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
                "--features",
                "hemplate/derive",
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
            .env("CARGO_MANIFEST_DIR", &base)
            .env(
                hemplate_core::transform::DIR_ENV,
                out.join("hemplate-transform"),
            )
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
mod ui {{ mod __hemx_surface {{ pub const INCLUDE_THROUGH_HEMX_SURFACE: () = (); }} {generated} }}
use hemplate::Hemplate;
use hemx::GeneratedTarget;
use hemx::advanced::{{Effect, PatchMode, ResourceRef, ScopeKey}};
// Real derived templates: only the transform written by the build step lowers them.
#[derive(Hemplate)] struct Parent {{ rows: Vec<String> }}
#[derive(Hemplate)] struct Child;
#[derive(Hemplate)] struct Grandchild;
#[derive(Hemplate)] struct Trusted {{ text: String, raw: ::hemx::advanced::SafeHtml }}
fn parent() -> Parent {{ Parent {{ rows: Vec::new() }} }}
impl hemx::KeyedPartial for Child {{ fn hemx_key(&self) -> String {{ "new-row".into() }} }}
fn assert_lowered(html: &str) {{
    assert!(html.contains(&format!("data-hemx-resource=\"{{}}\"", ui::child::child_text.__hemx_resource_id().id)));
    assert!(html.contains(&format!("data-hemx-resource='{{}}'", ui::grandchild::grandchild_text.__hemx_resource_id().id)));
    assert!(html.contains(&format!("data-hemx-action='{{}}'", ui::grandchild::save.id().id)));
    assert!(html.contains("data-hemx-timeout='120s'"));
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
    let page = ui::render(&parent());
    assert_lowered(page.as_str());
    assert!(page.as_str().contains(&format!("data-hemx-resource=\"{{}}\" data-hemx-collection=\"\"", ui::parent::rows.__hemx_resource_id().id)));
    // Plain Hemplate rendering already yields the final markup and action fields.
    let plain = parent().render().unwrap();
    assert_eq!(plain, page.as_str());
    assert!(plain.contains(&format!("<input type=\"hidden\" name=\"__h\" value=\"{{}}\"><input type=\"hidden\" name=\"__b\" value=\"{{}}\">", ui::grandchild::save.id().id, ui::BUILD_FINGERPRINT.0)));
    // Interpolated text and trusted raw HTML are never rewritten.
    let marker = "<i data-hemx-slot=\"outlet\" data-hemx-handle=\"save\" h-key=\"k\"></i>";
    let trusted = Trusted {{ text: marker.into(), raw: ::hemx::advanced::SafeHtml::trusted(marker) }}.render().unwrap();
    assert!(trusted.ends_with(&format!("{{marker}}</p>")));
    assert!(trusted.contains("&lt;i data-hemx-slot=&quot;outlet&quot;"));
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
    assert_lowered(ui::render(&parent()).as_str());
    assert_lowered(ui::render(&Child).as_str());
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
