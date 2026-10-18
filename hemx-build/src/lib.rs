use hemplate_core::ast::build_ast;
use hemplate_core::surface::{
    extract_surface, AttributeOrigin, ControlKind, ScopeId, ScopeKind, SurfaceAttribute,
    SurfaceDocument, SurfaceNodeKind, SurfaceScope,
};
use hemx_core::{EFFECT_BATCH_ABI_VERSION, RUNTIME_ABI_VERSION, SURFACE_SCHEMA_VERSION};
use quote::ToTokens;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

mod artifact;
mod authoring;
#[cfg(test)]
mod contract_tests;
mod rust_source;

use artifact::{
    collect_input_files, generated_targets, parse_error, unkeyed_generated_target_diagnostics,
    write_if_changed, Resources,
};
use authoring::{
    reject_invalid_hemx_attr_placement, reject_invalid_hemx_attr_values,
    reject_selector_target_attrs, reject_unkeyed_loop, reject_unknown_hemx_attrs,
};
#[cfg(test)]
use rust_source::{
    collect_rust_struct_facts_from_file, compact_tokens, h_for_local_and_self_field,
    vec_element_type,
};
use rust_source::{
    context_type_for_heml_path, loop_locals_for_surface, nearest_dir_with, rust_struct_facts_in,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticSeverity {
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticCode {
    UnkeyedGeneratedTarget,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    pub code: DiagnosticCode,
    pub severity: DiagnosticSeverity,
    pub file: PathBuf,
    pub directive: String,
    pub target: String,
    pub message: String,
    pub expected: String,
    pub repair: String,
}

impl Diagnostic {
    pub fn to_io_error(&self) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, self.to_string())
    }
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: {}; expected {}; repair: {}",
            self.file.display(),
            self.message,
            self.expected,
            self.repair
        )
    }
}

#[derive(Clone, Debug)]
pub struct AppBuilder {
    out_dir: Option<PathBuf>,
    template_dir: PathBuf,
    global_exports: bool,
    surfaces: Vec<(PathBuf, SurfaceDocument)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedTarget {
    pub kind: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateContextFacts {
    pub context_type: String,
    pub self_fields: Vec<TemplateFieldFact>,
    pub locals: Vec<TemplateLocalFact>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateFieldFact {
    pub name: String,
    pub type_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateLocalFact {
    pub name: String,
    pub type_name: String,
    pub fields: Vec<TemplateFieldFact>,
}

pub fn diagnostics_for_heml_file(path: impl AsRef<Path>) -> io::Result<Vec<Diagnostic>> {
    let path = path.as_ref();
    let source = std::fs::read_to_string(path)?;
    diagnostics_for_heml_source(path, source)
}

pub fn diagnostics_for_heml_source(
    path: impl AsRef<Path>,
    source: impl Into<String>,
) -> io::Result<Vec<Diagnostic>> {
    let path = path.as_ref();
    let surface = surface_for_heml_source(path, source.into())?;
    Ok(unkeyed_generated_target_diagnostics(path, &surface))
}

pub fn generated_targets_for_heml_source(
    path: impl AsRef<Path>,
    source: impl Into<String>,
) -> io::Result<Vec<GeneratedTarget>> {
    let path = path.as_ref();
    let surface = surface_for_heml_source(path, source.into())?;
    Ok(generated_targets(&surface))
}

pub fn template_context_facts_for_heml_file(
    path: impl AsRef<Path>,
) -> io::Result<Option<TemplateContextFacts>> {
    let path = path.as_ref();
    let source = std::fs::read_to_string(path)?;
    template_context_facts_for_heml_source(path, source)
}

pub fn template_context_facts_for_heml_source(
    path: impl AsRef<Path>,
    source: impl Into<String>,
) -> io::Result<Option<TemplateContextFacts>> {
    let path = path.as_ref();
    let source = source.into();
    let Some(context_type) = context_type_for_heml_path(path) else {
        return Ok(None);
    };
    let Some(root) = nearest_dir_with(path, "Cargo.toml") else {
        return Ok(None);
    };
    let structs = rust_struct_facts_in(&root)?;
    let Some(context) = structs.get(&context_type) else {
        return Ok(None);
    };
    if !context.derives_hemplate {
        return Ok(None);
    }

    let surface = surface_for_heml_source(path, source)?;
    let locals = loop_locals_for_surface(&surface, &context.fields, &structs);
    Ok(Some(TemplateContextFacts {
        context_type,
        self_fields: context.fields.clone(),
        locals,
    }))
}

fn surface_for_heml_source(path: &Path, source: String) -> io::Result<SurfaceDocument> {
    let doc = build_ast(Arc::new(source)).map_err(|err| parse_error(path, err))?;
    let Some(doc) = doc else {
        return Ok(SurfaceDocument {
            nodes: Vec::new(),
            scopes: vec![SurfaceScope {
                parent: None,
                kind: ScopeKind::Root,
            }],
            forms: Vec::new(),
        });
    };
    Ok(extract_surface(&doc))
}

impl AppBuilder {
    pub fn out_dir(mut self, out_dir: impl Into<PathBuf>) -> Self {
        self.out_dir = Some(out_dir.into());
        self
    }

    pub fn template_dir(mut self, template_dir: impl Into<PathBuf>) -> Self {
        self.template_dir = template_dir.into();
        self
    }

    pub fn global_exports(mut self, enabled: bool) -> Self {
        self.global_exports = enabled;
        self
    }

    pub fn surface(mut self, path: impl Into<PathBuf>, surface: SurfaceDocument) -> Self {
        self.surfaces.push((path.into(), surface));
        self
    }

    pub fn run(self) -> io::Result<()> {
        println!("cargo:rerun-if-changed={}", self.template_dir.display());

        let out_dir = self
            .out_dir
            .or_else(|| std::env::var_os("OUT_DIR").map(PathBuf::from));

        let Some(out_dir) = out_dir else {
            return Ok(());
        };

        std::fs::create_dir_all(&out_dir)?;

        let mut resources = Resources::default();
        let input_paths = collect_input_files(&self.template_dir)?;
        if self.surfaces.is_empty() {
            for path in input_paths.iter().filter(|path| {
                path.extension().and_then(|extension| extension.to_str()) == Some("heml")
            }) {
                let source = std::fs::read_to_string(path)?;
                let surface = surface_for_heml_source(path, source)?;
                resources.add_surface(&self.template_dir, path, &surface)?;
            }
        } else {
            for (path, surface) in &self.surfaces {
                resources.add_surface(&self.template_dir, path, surface)?;
            }
        }
        for path in input_paths.iter().filter(|path| {
            matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("css" | "scss")
            )
        }) {
            let source = std::fs::read_to_string(path)?;
            resources.add_stylesheet(&self.template_dir, path, &source)?;
        }

        write_if_changed(
            &out_dir.join("hemx.generated.rs"),
            resources.generated_rs(self.global_exports).as_bytes(),
        )?;
        write_if_changed(&out_dir.join("hemx.syms"), resources.syms().as_bytes())?;
        Ok(())
    }
}

pub fn app() -> AppBuilder {
    AppBuilder {
        out_dir: None,
        template_dir: PathBuf::from("templates"),
        global_exports: false,
        surfaces: Vec::new(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::*;

    static OUT_DIR_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn public_heml_inspection_entry_points_preserve_paths_and_io_diagnostics() {
        let root = std::env::temp_dir().join(format!(
            "hemx-build-inspection-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        std::fs::create_dir_all(&root).unwrap();
        let template = root.join("card.heml");
        let source = r#"<template h-for="todo in &self.todos"><button data-hemx-handle="save">Save</button></template>"#;
        std::fs::write(&template, source).unwrap();

        let from_source = diagnostics_for_heml_source(&template, source.to_owned()).unwrap();
        let from_file = diagnostics_for_heml_file(&template).unwrap();
        assert_eq!(from_file, from_source);
        assert_eq!(from_source.len(), 1);
        assert_eq!(from_source[0].file, template);
        assert_eq!(from_source[0].directive, "data-hemx-handle");

        let targets = generated_targets_for_heml_source(
            &template,
            r#"<main data-hemx-root="app"><section h-slot="notice"></section><button data-hemx-handle="save">Save</button></main>"#,
        )
        .unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].name, "save");
        assert_eq!(
            template_context_facts_for_heml_source(
                &template,
                "<main>{{ self.title }}</main>".to_owned(),
            )
            .unwrap(),
            None
        );
        let crate_root = root.join("crate");
        let templates = crate_root.join("templates");
        std::fs::create_dir_all(crate_root.join("src")).unwrap();
        std::fs::create_dir_all(&templates).unwrap();
        std::fs::write(
            crate_root.join("Cargo.toml"),
            "[package]\nname='facts'\nversion='0.0.0'\n",
        )
        .unwrap();
        std::fs::write(
            crate_root.join("src/lib.rs"),
            "#[derive(Hemplate)] struct Profile { title: String }\n#[derive(Clone, hemplate::Hemplate)] struct ProfileCard { card_title: String }\n#[derive(NotHemplate)] struct ProfileDetails { hidden: String }\n#[derive(HemplateExtra)] struct Extra { hidden: String }\nmod inline { #[derive(Hemplate)] pub struct InlineProfile { pub inline_title: String } }\nmod external;\nstruct Plain { title: String }",
        )
        .unwrap();
        std::fs::create_dir_all(crate_root.join("src/nested")).unwrap();
        std::fs::write(
            crate_root.join("src/nested/profile.rs"),
            "#[derive(Hemplate)] struct NestedProfile { nested_title: String }",
        )
        .unwrap();
        std::fs::write(crate_root.join("src/external.rs"), "struct External;").unwrap();
        std::fs::write(crate_root.join("src/ignored.txt"), [0xff]).unwrap();
        std::fs::write(
            crate_root.join("outside.rs"),
            "#[derive(Hemplate)] struct OutsideProfile { outside: String }",
        )
        .unwrap();
        let profile = templates.join("profile.heml");
        let facts = template_context_facts_for_heml_source(
            &profile,
            "<main>{{ self.title }}</main>".to_owned(),
        )
        .unwrap()
        .expect("Hemlate-derived context facts");
        assert_eq!(facts.context_type, "Profile");
        assert_eq!(facts.self_fields[0].name, "title");

        for (template, context_type, field) in [
            ("inline_profile.heml", "InlineProfile", "inline_title"),
            ("nested_profile.heml", "NestedProfile", "nested_title"),
        ] {
            let nested_facts = template_context_facts_for_heml_source(
                templates.join(template),
                format!("<main>{{{{ self.{field} }}}}</main>"),
            )
            .unwrap()
            .expect("recursive Rust struct fact");
            assert_eq!(nested_facts.context_type, context_type);
            assert_eq!(nested_facts.self_fields[0].name, field);
        }
        assert_eq!(
            template_context_facts_for_heml_source(
                templates.join("outside_profile.heml"),
                "<main>{{ self.outside }}</main>".to_owned(),
            )
            .unwrap(),
            None,
            "Rust facts outside src must not become template authority"
        );

        let card_facts = template_context_facts_for_heml_source(
            templates.join("profile-card.heml"),
            "<main>{{ self.card_title }}</main>".to_owned(),
        )
        .unwrap()
        .expect("qualified Hemlate derive");
        assert_eq!(card_facts.context_type, "ProfileCard");
        assert_eq!(card_facts.self_fields[0].name, "card_title");
        assert_eq!(
            template_context_facts_for_heml_source(
                templates.join("profile_details.heml"),
                "<main>{{ self.hidden }}</main>".to_owned(),
            )
            .unwrap(),
            None,
            "derive names containing Hemlate must not grant context authority"
        );
        assert_eq!(
            context_type_for_heml_path(Path::new("--profile__card--.heml")),
            Some("ProfileCard".into())
        );
        assert_eq!(context_type_for_heml_path(Path::new("---.heml")), None);
        assert_eq!(context_type_for_heml_path(Path::new("")), None);
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            assert_eq!(
                context_type_for_heml_path(Path::new(std::ffi::OsStr::from_bytes(b"\xff.heml"))),
                None
            );
        }

        std::fs::write(crate_root.join("src/broken.rs"), [0xff]).unwrap();
        assert_eq!(
            template_context_facts_for_heml_source(
                &profile,
                "<main>{{ self.title }}</main>".to_owned(),
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::InvalidData
        );
        std::fs::remove_file(crate_root.join("src/broken.rs")).unwrap();
        assert_eq!(
            template_context_facts_for_heml_source(
                templates.join("plain.heml"),
                "<main>{{ self.title }}</main>".to_owned(),
            )
            .unwrap(),
            None
        );
        let missing = root.join("missing.heml");
        for error in [
            diagnostics_for_heml_file(&missing).unwrap_err(),
            template_context_facts_for_heml_file(&missing).unwrap_err(),
        ] {
            assert_eq!(error.kind(), io::ErrorKind::NotFound);
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rust_type_facts_are_compact_and_vector_specific() {
        let qualified: syn::Type =
            syn::parse_str("std::collections::HashMap<String, &crate::Thing<Left, Right>>")
                .unwrap();
        assert_eq!(
            compact_tokens(&qualified),
            "std::collections::HashMap<String,&crate::Thing<Left,Right>>"
        );
        let mutable: syn::Type = syn::parse_str("&mut crate::Thing").unwrap();
        assert_eq!(compact_tokens(&mutable), "&mut crate::Thing");
        let tuple: syn::Type = syn::parse_str("(&crate::Thing, String)").unwrap();
        assert_eq!(compact_tokens(&tuple), "(&crate::Thing,String)");
        let function: syn::Type = syn::parse_str("fn(&crate::Thing) -> &crate::Thing").unwrap();
        assert_eq!(
            compact_tokens(&function),
            "fn (&crate::Thing) ->&crate::Thing"
        );

        assert_eq!(vec_element_type("Vec<String>"), Some("String".into()));
        assert_eq!(
            vec_element_type("std::vec::Vec<&crate::Thing<Left,Right>>"),
            Some("&crate::Thing<Left,Right>".into())
        );
        assert_eq!(vec_element_type("Vec< String >"), Some("String".into()));
        for invalid in [
            "",
            "String",
            "Vec<String",
            "VecString>",
            "alloc::vec::Vec<String>",
        ] {
            assert_eq!(
                vec_element_type(invalid),
                None,
                "accepted non-vector type {invalid:?}"
            );
        }
    }

    #[test]
    fn rust_fact_collection_propagates_parse_and_recursive_directory_errors() {
        let base = test_dir("hemx-build-rust-fact-errors");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("src")).unwrap();
        let mut facts = HashMap::new();
        std::fs::write(base.join("src/broken.rs"), "struct {").unwrap();
        assert_eq!(
            collect_rust_struct_facts_from_file(&base.join("src/broken.rs"), &mut facts)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert!(facts.is_empty());
        std::fs::remove_file(base.join("src/broken.rs")).unwrap();

        assert!(rust_struct_facts_in(&base.join("missing"))
            .unwrap()
            .is_empty());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn identifier_and_literal_helpers_fail_closed_at_boundaries() {
        assert_eq!(rust_ident("alpha_9"), Some("alpha_9".into()));
        for invalid in ["", "9alpha", "alpha-beta", "alpha beta", "alphaé"] {
            assert_eq!(
                rust_ident(invalid),
                None,
                "accepted Rust identifier {invalid:?}"
            );
        }
        assert_eq!(class_ident("alpha-beta_9"), Some("alpha_beta_9".into()));
        for invalid in ["", "9alpha", "alphaé", "alpha.beta", "alpha beta"] {
            assert_eq!(class_ident(invalid), None, "accepted CSS class {invalid:?}");
        }

        assert_eq!(rust_str("a\n\"b\\c"), "\"a\\n\\\"b\\\\c\"");
        assert_eq!(rust_str_opt(None), "None");
        assert_eq!(rust_str_opt(Some("a\n\"b\\c")), "Some(\"a\\n\\\"b\\\\c\")");
        assert_eq!(
            canonical_symbol(
                Path::new("templates"),
                Path::new("templates/nested/panel.heml"),
                "save",
            ),
            "nested/panel.heml::save"
        );

        let root = Path::new("templates");
        assert_eq!(
            component_ident(root, Path::new("templates/nested/profile_card.heml")).unwrap(),
            "profile_card"
        );
        assert_eq!(
            component_ident(root, Path::new("templates/9bad.heml"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            component_ident(root, Path::new("outside.heml")).unwrap(),
            "outside"
        );
        assert_eq!(
            data_param_ident("data-hemx-param-item-id"),
            Some("item_id".into())
        );
        assert_eq!(data_param_ident("data-item-id"), None);
        assert_eq!(data_param_ident("aria-label"), None);
        assert_eq!(app().template_dir, PathBuf::from("templates"));
        assert_eq!(nearest_dir_with(Path::new("/"), "hemx-absent-marker"), None);

        let mut exports = String::new();
        let mut used = BTreeSet::from(["shared".to_owned(), "shared_target".to_owned()]);
        push_root_export(&mut exports, "", "targets", &mut used, "shared", "target");
        assert!(
            exports.is_empty(),
            "colliding alias must not be emitted twice"
        );

        let mut resources = BTreeMap::new();
        let first = insert_resource(
            &mut resources,
            "slot",
            "panel.heml::summary".into(),
            "summary".into(),
            "panel".into(),
        )
        .unwrap();
        first.role = SlotRole::Row;
        assert_eq!(
            insert_resource(
                &mut resources,
                "slot",
                "panel.heml::summary".into(),
                "summary".into(),
                "panel".into(),
            )
            .unwrap()
            .role,
            SlotRole::Row
        );
        assert_eq!(
            insert_resource(
                &mut resources,
                "slot",
                "other.heml::summary".into(),
                "summary".into(),
                "other".into(),
            )
            .unwrap_err()
            .to_string(),
            "duplicate generated identifier `summary` for `panel.heml::summary` and `other.heml::summary`"
        );

        let invalid_surface = surface_for_heml_source(
            Path::new("123.heml"),
            r#"<section data-hemx-slot="summary"></section>"#.to_owned(),
        )
        .unwrap();
        let mut extracted = Resources::default();
        assert_eq!(
            extracted
                .add_surface(
                    Path::new("templates"),
                    Path::new("templates/123.heml"),
                    &invalid_surface,
                )
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );

        let kind = "handle";
        let source = r#"<button data-hemx-handle="shared"></button>"#;
        {
            let first =
                surface_for_heml_source(Path::new("first.heml"), source.to_owned()).unwrap();
            let second =
                surface_for_heml_source(Path::new("second.heml"), source.to_owned()).unwrap();
            let mut duplicate = Resources::default();
            duplicate
                .add_surface(
                    Path::new("templates"),
                    Path::new("templates/first.heml"),
                    &first,
                )
                .unwrap();
            assert_eq!(
                duplicate
                    .add_surface(
                        Path::new("templates"),
                        Path::new("templates/second.heml"),
                        &second,
                    )
                    .unwrap_err()
                    .to_string(),
                "duplicate generated identifier `shared` for `first.heml::shared` and `second.heml::shared`",
                "{kind} collision must propagate"
            );
        }

        let mut form_without_controls = surface_for_heml_source(
            Path::new("form.heml"),
            r#"<form data-hemx-handle="save" data-hemx-form="empty"></form>"#.to_owned(),
        )
        .unwrap();
        form_without_controls.forms.clear();
        let mut form_resources = Resources::default();
        form_resources
            .add_surface(
                Path::new("templates"),
                Path::new("templates/form.heml"),
                &form_without_controls,
            )
            .unwrap();
        assert!(form_resources.forms["empty"].controls.is_empty());

        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let invalid =
                PathBuf::from("templates").join(std::ffi::OsStr::from_bytes(b"\xff.heml"));
            assert_eq!(
                component_ident(root, &invalid).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
    }

    #[test]
    fn generated_contract_fingerprint_is_deterministic() {
        assert_eq!(
            (
                SURFACE_SCHEMA_VERSION,
                EFFECT_BATCH_ABI_VERSION,
                RUNTIME_ABI_VERSION
            ),
            (2, 3, 3),
            "snapshots describe the elected schema 2 and wire/runtime 3 contract"
        );
        let resource = |symbol: &str, component: &str, id| Resource {
            symbol: symbol.into(),
            ident: symbol.into(),
            component: component.into(),
            role: SlotRole::Slot,
            id,
        };
        let mut resources = Resources::default();
        resources
            .slots
            .insert("slot".into(), resource("slot", "page", 11));
        resources
            .handles
            .insert("save".into(), resource("save", "page", 12));
        resources.forms.insert(
            "profile".into(),
            FormResource {
                resource: resource("profile", "page", 14),
                controls: vec![
                    GeneratedControl {
                        name: "email".into(),
                        kind: ControlKind::Text,
                        required: true,
                    },
                    GeneratedControl {
                        name: "nickname".into(),
                        kind: ControlKind::Text,
                        required: false,
                    },
                ],
            },
        );

        let expected_parts = vec![
            SURFACE_SCHEMA_VERSION,
            EFFECT_BATCH_ABI_VERSION,
            RUNTIME_ABI_VERSION,
            0,
            11,
            0,
            1,
            12,
            2,
            14,
            2,
            stable_id("form-field", "email"),
            1,
            stable_id("form-field", "nickname"),
            0,
        ];
        assert_eq!(resources.fingerprint_parts(), expected_parts);

        let generated = resources.generated_rs(false);
        assert!(generated.starts_with("// @generated by hemx-build. Do not edit.\n"));
        assert!(generated.contains("pub const BUILD_FINGERPRINT"));
        assert!(generated.contains("\n#[allow(non_upper_case_globals)]\npub mod page {\n"));
        assert!(generated.contains("pub const slot: SlotTarget"));
        assert!(generated.contains("pub const save: ::hemx::Handle"));
        assert!(generated.contains("pub const profile: ::hemx::Form"));
        assert!(generated.contains("\n    pub mod targets {\n"));

        let generated_with_globals = resources.generated_rs(true);
        assert!(generated_with_globals.contains("pub mod advanced {"));
        assert!(generated_with_globals.contains("pub mod slots {"));

        let mut row_slot = resource("row", "row", 21);
        row_slot.role = SlotRole::Row;
        resources.slots.insert("row".into(), row_slot);
        resources
            .handles
            .insert("edit".into(), resource("edit", "row", 22));
        resources
            .slots
            .insert("child".into(), resource("child", "page", 24));
        resources
            .handles
            .insert("child_handle".into(), resource("child_handle", "child", 25));
        resources
            .slots
            .insert("shared".into(), resource("shared", "page", 26));
        resources
            .handles
            .insert("shared".into(), resource("shared", "page", 27));
        resources.forms.insert(
            "shared".into(),
            FormResource {
                resource: resource("shared", "page", 28),
                controls: Vec::new(),
            },
        );
        for (symbol, ident, component, token) in [
            ("a::card", "card", "page", "card"),
            ("b::card", "card", "row", "card"),
            ("z::last", "last", "page", "last"),
        ] {
            resources.classes.insert(
                symbol.into(),
                ClassToken {
                    symbol: symbol.into(),
                    ident: ident.into(),
                    component: component.into(),
                    token: token.into(),
                },
            );
        }
        for (component, name) in [("page", "click"), ("row", "click"), ("z", "submit")] {
            resources.events.insert(
                format!("{component}::{name}"),
                EventToken {
                    symbol: format!("{component}::{name}"),
                    ident: name.into(),
                    component: component.into(),
                    name: name.into(),
                },
            );
        }
        resources
            .handle_forms
            .insert("save".into(), "profile".into());
        resources
            .handle_params
            .entry("save".into())
            .or_default()
            .insert("item_id".into());
        let canonical_generated = resources.generated_rs(false);
        let canonical_globals = resources.generated_rs(true);
        let canonical_syms = resources.syms();
        assert_eq!(
            stable_id("generated-rs", &canonical_generated),
            1_958_256_377,
            "canonical generated Rust changed"
        );
        assert_eq!(
            stable_id("generated-rs-global", &canonical_globals),
            2_790_822_348,
            "canonical global-export Rust changed"
        );
        assert_eq!(
            stable_id("generated-syms", &canonical_syms),
            2_299_561_723,
            "canonical symbol manifest changed"
        );
    }

    #[test]
    fn app_builder_propagates_output_and_contract_errors_without_panicking() {
        let root = std::env::temp_dir().join(format!(
            "hemx-build-error-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let out = root.join("out");
        std::fs::create_dir_all(out.join("hemx.generated.rs")).unwrap();

        let result = std::panic::catch_unwind(|| app().out_dir(&out).run());
        assert!(result.is_ok(), "ordinary output errors must not panic");
        assert!(result.unwrap().is_err());

        std::fs::remove_dir_all(out.join("hemx.generated.rs")).unwrap();
        std::fs::create_dir_all(out.join("hemx.syms")).unwrap();
        let result = std::panic::catch_unwind(|| app().out_dir(&out).run());
        assert!(result.is_ok(), "symbol output errors must not panic");
        assert!(result.unwrap().is_err());

        std::fs::remove_dir_all(out.join("hemx.syms")).unwrap();

        let templates = root.join("templates");
        let invalid_out = root.join("invalid-out");
        std::fs::create_dir_all(&templates).unwrap();
        std::fs::write(
            templates.join("invalid_param.heml"),
            r#"<button data-hemx-handle="save" data-hemx-param-123="value">Save</button>"#,
        )
        .unwrap();
        let error = app()
            .template_dir(&templates)
            .out_dir(&invalid_out)
            .run()
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            error.to_string(),
            "invalid handler param attribute `data-hemx-param-123`; expected data-hemx-param-* name usable from Rust"
        );

        std::fs::remove_file(templates.join("invalid_param.heml")).unwrap();
        std::fs::write(
            templates.join("invalid_slot.heml"),
            r#"<section data-hemx-slot="123">Invalid</section>"#,
        )
        .unwrap();
        let error = app()
            .template_dir(&templates)
            .out_dir(&invalid_out)
            .run()
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            error.to_string(),
            "invalid hemx slot name `123`; expected a Rust identifier"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn app_builder_discovers_nested_inputs_and_propagates_collection_errors() {
        let base = test_dir("hemx-build-collection");
        let templates = base.join("templates");
        let nested = templates.join("nested");
        let out = base.join("out");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(templates.join("a_empty.heml"), "").unwrap();
        std::fs::write(
            nested.join("panel.heml"),
            r#"<section data-hemx-slot="nested_panel"></section>"#,
        )
        .unwrap();
        std::fs::write(nested.join("panel.css"), ".nested-css {}").unwrap();
        std::fs::write(nested.join("theme.scss"), ".nested-scss {}").unwrap();
        std::fs::write(nested.join("ignored.txt"), [0xff]).unwrap();

        app().template_dir(&templates).out_dir(&out).run().unwrap();
        let syms = std::fs::read_to_string(out.join("hemx.syms")).unwrap();
        assert!(syms.contains("slot\tnested/panel.heml::nested_panel\tnested_panel\t"));
        assert!(syms.contains("class\tnested/panel.css::nested-css\tnested_css\tnested-css\n"));
        assert!(syms.contains("class\tnested/theme.scss::nested-scss\tnested_scss\tnested-scss\n"));

        let missing = base.join("missing");
        let missing_out = base.join("missing-out");
        app()
            .template_dir(&missing)
            .out_dir(&missing_out)
            .run()
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(missing_out.join("hemx.syms")).unwrap(),
            "hemx-syms-v1\n"
        );

        let _out_dir_lock = OUT_DIR_LOCK.lock().unwrap();
        let previous_out_dir = std::env::var_os("OUT_DIR");
        let env_out = base.join("env-out");
        std::env::set_var("OUT_DIR", &env_out);
        let env_result = app().template_dir(&missing).run();
        match previous_out_dir {
            Some(value) => std::env::set_var("OUT_DIR", value),
            None => std::env::remove_var("OUT_DIR"),
        }
        env_result.unwrap();
        assert!(env_out.join("hemx.generated.rs").is_file());

        let template_file = base.join("not-a-directory");
        std::fs::write(&template_file, "not a directory").unwrap();
        let error = app()
            .template_dir(&template_file)
            .out_dir(base.join("file-root-out"))
            .run()
            .unwrap_err();
        assert!(matches!(
            error.kind(),
            io::ErrorKind::NotADirectory | io::ErrorKind::Other
        ));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let restricted_root = base.join("restricted-root");
            let restricted_nested = restricted_root.join("nested");
            std::fs::create_dir_all(&restricted_nested).unwrap();
            std::fs::set_permissions(&restricted_nested, std::fs::Permissions::from_mode(0o000))
                .unwrap();
            let error = app()
                .template_dir(&restricted_root)
                .out_dir(base.join("restricted-out"))
                .run()
                .unwrap_err();
            std::fs::set_permissions(&restricted_nested, std::fs::Permissions::from_mode(0o700))
                .unwrap();
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        }

        let invalid_heml = base.join("invalid-heml");
        std::fs::create_dir_all(&invalid_heml).unwrap();
        std::fs::write(invalid_heml.join("bad.heml"), [0xff]).unwrap();
        assert_eq!(
            app()
                .template_dir(&invalid_heml)
                .out_dir(base.join("invalid-heml-out"))
                .run()
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );

        let invalid_css = base.join("invalid-css");
        std::fs::create_dir_all(&invalid_css).unwrap();
        std::fs::write(invalid_css.join("bad.css"), [0xff]).unwrap();
        assert_eq!(
            app()
                .template_dir(&invalid_css)
                .out_dir(base.join("invalid-css-out"))
                .run()
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );

        let blocked_out = base.join("blocked-out");
        std::fs::write(&blocked_out, "not a directory").unwrap();
        assert!(app()
            .template_dir(&missing)
            .out_dir(&blocked_out)
            .run()
            .is_err());

        let explicit_surface = surface_for_heml_source(
            Path::new("explicit.heml"),
            r#"<section data-hemx-slot="123"></section>"#.to_owned(),
        )
        .unwrap();
        assert_eq!(
            app()
                .surface("explicit.heml", explicit_surface)
                .out_dir(base.join("explicit-out"))
                .run()
                .unwrap_err()
                .to_string(),
            "invalid hemx slot name `123`; expected a Rust identifier"
        );

        let ordered = base.join("ordered");
        std::fs::create_dir_all(&ordered).unwrap();
        std::fs::write(ordered.join("a.css"), ".foo-bar {}").unwrap();
        std::fs::write(ordered.join("b.css"), ".foo_bar {}").unwrap();
        assert_eq!(
            app()
                .template_dir(&ordered)
                .out_dir(base.join("ordered-out"))
                .run()
                .unwrap_err()
                .to_string(),
            "duplicate generated class identifier `foo_bar` for CSS classes `foo-bar` and `foo_bar`"
        );

        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn no_op_build_preserves_generated_artifact_timestamps() {
        let base = test_dir("hemx-build-no-op");
        let templates = base.join("templates");
        let out = base.join("out");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&templates).unwrap();
        std::fs::write(
            templates.join("panel.heml"),
            r#"<button data-hemx-handle="save">Save</button>"#,
        )
        .unwrap();

        app().template_dir(&templates).out_dir(&out).run().unwrap();
        let generated = out.join("hemx.generated.rs");
        let symbols = out.join("hemx.syms");
        let generated_modified = std::fs::metadata(&generated).unwrap().modified().unwrap();
        let symbols_modified = std::fs::metadata(&symbols).unwrap().modified().unwrap();

        std::thread::sleep(std::time::Duration::from_millis(20));
        app().template_dir(&templates).out_dir(&out).run().unwrap();

        assert_eq!(
            std::fs::metadata(generated).unwrap().modified().unwrap(),
            generated_modified,
            "no-op codegen must not invalidate downstream Rust compilation"
        );
        assert_eq!(
            std::fs::metadata(symbols).unwrap().modified().unwrap(),
            symbols_modified,
            "no-op symbol generation must preserve its artifact timestamp"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn changed_template_refreshes_generated_artifacts() {
        let base = test_dir("hemx-build-refresh");
        let templates = base.join("templates");
        let out = base.join("out");
        let template = templates.join("panel.heml");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&templates).unwrap();
        std::fs::write(
            &template,
            r#"<button data-hemx-handle="save">Save</button>"#,
        )
        .unwrap();

        app().template_dir(&templates).out_dir(&out).run().unwrap();
        let initial_generated = std::fs::read_to_string(out.join("hemx.generated.rs")).unwrap();
        let initial_symbols = std::fs::read_to_string(out.join("hemx.syms")).unwrap();
        assert!(initial_generated.contains("pub const save"));
        assert!(initial_symbols.contains("save"));

        std::fs::write(
            &template,
            r#"<button data-hemx-handle="cancel">Cancel</button>"#,
        )
        .unwrap();
        app().template_dir(&templates).out_dir(&out).run().unwrap();

        let refreshed_generated = std::fs::read_to_string(out.join("hemx.generated.rs")).unwrap();
        let refreshed_symbols = std::fs::read_to_string(out.join("hemx.syms")).unwrap();
        assert_ne!(refreshed_generated, initial_generated);
        assert_ne!(refreshed_symbols, initial_symbols);
        assert!(refreshed_generated.contains("pub const cancel"));
        assert!(!refreshed_generated.contains("pub const save"));
        assert!(refreshed_symbols.contains("cancel"));
        assert!(!refreshed_symbols.contains("save"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn emits_generated_resources_from_heml() {
        let base = test_dir("hemx-build-test");
        let templates = base.join("templates");
        let out = base.join("out");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&templates).unwrap();
        std::fs::write(
            templates.join("todo.heml"),
            r#"<form class="todo-card" data-hemx-handle="create" data-hemx-form="new_todo"><input name="title" required><select name="labels" multiple><option value="urgent">Urgent</option></select><input name="__h" value="reserved"></form><button data-hemx-handle="delete" data-hemx-param-todo-id="7" data-tooltip="Remove" data-hemx-on="click change">Delete</button><ul data-hemx-slot="todos"></ul>"#,
        )
        .unwrap();
        std::fs::write(templates.join("todo.css"), ".todo-card { display: block; }").unwrap();

        app()
            .template_dir(&templates)
            .out_dir(&out)
            .global_exports(true)
            .run()
            .unwrap();

        let generated = std::fs::read_to_string(out.join("hemx.generated.rs")).unwrap();
        assert!(generated.contains("pub mod components"));
        assert!(generated.contains(
            "pub const todo: ::hemx::ComponentRef = ::hemx::ComponentRef::new(\"todo\")"
        ));
        assert!(generated.contains("pub mod advanced"));
        assert!(generated.contains("pub mod slots"));
        assert!(generated.contains("pub const todos"));
        assert!(generated.contains("pub mod targets"));
        assert!(generated.contains("pub struct SlotTarget<T>"));
        assert!(generated.contains("impl<T> ::hemx::GeneratedTarget for SlotTarget<T>"));
        assert!(generated.contains(
            "impl<K: ::std::string::ToString, T> ::hemx::GeneratedTarget for KeyedSlotTarget<K, T>"
        ));
        assert!(generated.contains(
            "pub fn set<'a, V>(self, __hemx_rows: impl ::std::iter::IntoIterator<Item = &'a V>) -> ::hemx::advanced::Effect"
        ));
        assert!(generated.contains(
            "pub fn transition<'a, B, A, V>(self, __hemx_before: impl ::std::iter::IntoIterator<Item = B>, __hemx_after: impl ::std::iter::IntoIterator<Item = A>, __hemx_changed: impl ::std::iter::IntoIterator<Item = &'a V>) -> ::hemx::advanced::Effect"
        ));
        assert!(!generated.contains("pub fn append(") && !generated.contains("move_before"));
        assert!(!generated.contains("pub fn sync<") && !generated.contains("pub fn order<"));
        assert!(generated.contains("pub const todos: SlotTarget<::std::string::String> = SlotTarget::new(super::advanced::slots::todos);"));
        assert!(generated.contains("pub use self::targets::todos;"));
        assert!(generated.contains(
            "pub fn put(self, __hemx_view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect"
        ));
        assert!(generated.contains(
            "pub fn replace(self, __hemx_view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect"
        ));
        assert!(generated.contains(
            "pub fn set(self, __hemx_value: impl ::std::string::ToString) -> ::hemx::advanced::Effect"
        ));
        assert!(generated.contains("pub mod handles"));
        assert!(generated
            .contains("pub const create: ::hemx::Handle<::hemx::Form<::std::string::String>>"));
        assert!(generated.contains("pub mod params"));
        assert!(generated.contains(
            "pub const todo_id: ::hemx::ParamName = ::hemx::ParamName::new(\"todo_id\")"
        ));
        assert!(generated.contains("pub mod forms"));
        assert!(generated.contains("pub mod events"));
        assert!(generated
            .contains("pub const click: ::hemx::EventName = ::hemx::EventName::new(\"click\")"));
        assert!(generated
            .contains("pub const change: ::hemx::EventName = ::hemx::EventName::new(\"change\")"));
        assert!(generated.contains("pub const new_todo"));
        assert!(generated.contains("pub mod todo"));
        assert!(generated.contains("pub const ALL_IDS"));
        assert!(generated.contains(
            "pub fn lower(html: impl ::std::convert::AsRef<str>) -> ::std::string::String"
        ));
        assert!(generated.contains("#[doc(hidden)]\npub fn lower_html"));
        assert!(generated.contains("pub fn static_fragment(html: &'static str) -> ::hemx::Html"));
        assert!(generated.contains(
            "#[doc(hidden)]\npub fn render(view: &impl ::hemplate::Hemplate) -> ::hemx::Html"
        ));
        assert!(generated.contains("pub fn page(view: &impl ::hemplate::Hemplate) -> ::hemx::Html"));
        assert!(generated.contains(
            "#[doc(hidden)]\npub fn render_html(view: &impl ::hemplate::Hemplate) -> ::hemx::Html"
        ));
        assert!(generated.contains("pub fn put<T>(slot: ::hemx::advanced::Slot<T>, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect"));
        assert!(
            !generated.contains("pub fn append<K, T>")
                && !generated.contains("pub fn prepend<K, T>")
        );
        assert!(generated.contains("pub fn replace<K, T>(slot: ::hemx::advanced::KeyedSlot<K, T>, key: K, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect"));
        assert!(generated.contains("data-hemx-slot"));
        assert!(generated.contains("data-hemx-form"));
        assert!(generated.contains("data-hemx-resource"));
        assert!(!generated.contains("data-sid"));
        assert!(!generated.contains("data-fid"));
        assert!(generated.contains("__hemx_inject_handle_inputs"));

        let syms = std::fs::read_to_string(out.join("hemx.syms")).unwrap();
        for line in [
            format!(
                "slot\ttodo.heml::todos\ttodos\t{}\n",
                stable_id("slot", "todo.heml::todos")
            ),
            format!(
                "handle\ttodo.heml::create\tcreate\t{}\n",
                stable_id("handle", "todo.heml::create")
            ),
            format!(
                "handle\ttodo.heml::delete\tdelete\t{}\n",
                stable_id("handle", "todo.heml::delete")
            ),
            format!(
                "form\ttodo.heml::new_todo\tnew_todo\t{}\n",
                stable_id("form", "todo.heml::new_todo")
            ),
            "handle_form\tcreate\tnew_todo\n".to_owned(),
            "form_field\tnew_todo\tlabels\tfalse\ttrue\n".to_owned(),
            "form_field\tnew_todo\ttitle\ttrue\tfalse\n".to_owned(),
            "handle_param\tdelete\ttodo_id\n".to_owned(),
            "class\ttodo.css::todo-card\ttodo_card\ttodo-card\n".to_owned(),
            "class\ttodo.heml::todo-card\ttodo_card\ttodo-card\n".to_owned(),
            "event\ttodo.heml::change\tchange\tchange\n".to_owned(),
            "event\ttodo.heml::click\tclick\tclick\n".to_owned(),
        ] {
            assert!(
                syms.contains(&line),
                "missing symbol line {line:?} in {syms}"
            );
        }
        assert!(!syms.contains("form_field\tnew_todo\t__h\t"));
        // Ordinary data-* metadata is not a handler parameter.
        assert!(!syms.contains("handle_param\tdelete\ttooltip"), "{syms}");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn stylesheet_class_scanner_is_sorted_deduplicated_and_boundary_aware() {
        assert_eq!(stylesheet_class_tokens(""), Vec::<&str>::new());
        assert_eq!(stylesheet_class_tokens(".a"), vec!["a"]);
        assert_eq!(stylesheet_class_tokens(".ab"), vec!["ab"]);
        assert_eq!(stylesheet_class_tokens("x.skip .kept"), vec!["kept"]);
        assert_eq!(stylesheet_class_tokens("x..kept"), vec!["kept"]);
        assert_eq!(stylesheet_class_tokens("..kept"), vec!["kept"]);
        assert_eq!(stylesheet_class_tokens(". .kept"), vec!["kept"]);
        assert_eq!(stylesheet_class_tokens(".1 .kept"), vec!["kept"]);
        assert_eq!(stylesheet_class_tokens(".é .kept"), vec!["kept"]);
        assert!(!preceded_by_class_in_compound(b"tag.", 3));
        assert!(preceded_by_class_in_compound(b".first.", 6));
        assert_eq!(
            stylesheet_class_tokens(
                ".z9, .alpha.alpha, ._private, .-prefixed, .compound.second, tag.skipped, name-.skipped, name_.skipped"
            ),
            vec!["-prefixed", "_private", "alpha", "compound", "second", "z9"]
        );
        assert_eq!(
            stylesheet_class_tokens(
                " .space\n.newline\r.return\t.tab,.comma{.open}.close>.child+.adjacent~.sibling(.paren)"
            ),
            vec![
                "adjacent", "child", "close", "comma", "newline", "open", "paren", "return",
                "sibling", "space", "tab"
            ]
        );
    }

    #[test]
    fn class_and_form_resource_contracts_fail_closed_and_deduplicate() {
        let base = test_dir("hemx-build-resource-collisions");
        let templates = base.join("templates");
        let out = base.join("out");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&templates).unwrap();
        let invalid_stylesheet = templates.join("123.css");
        std::fs::write(&invalid_stylesheet, ".valid {}").unwrap();
        let invalid_stylesheet_name = app()
            .template_dir(&templates)
            .out_dir(&out)
            .run()
            .unwrap_err();
        assert_eq!(invalid_stylesheet_name.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            invalid_stylesheet_name.to_string(),
            "invalid template name `123`; expected a Rust module identifier"
        );
        std::fs::remove_file(invalid_stylesheet).unwrap();

        let invalid_class_template = templates.join("invalid_class.heml");
        std::fs::write(&invalid_class_template, r#"<div class="123"></div>"#).unwrap();
        let invalid_class = app()
            .template_dir(&templates)
            .out_dir(&out)
            .run()
            .unwrap_err();
        assert_eq!(invalid_class.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            invalid_class.to_string(),
            "invalid CSS class `123`; expected an ASCII class token usable from Rust"
        );
        std::fs::remove_file(invalid_class_template).unwrap();

        let stylesheet = templates.join("app.css");
        std::fs::write(&stylesheet, ".foo-bar {} .foo_bar {}").unwrap();
        let class_error = app()
            .template_dir(&templates)
            .out_dir(&out)
            .run()
            .unwrap_err();
        assert_eq!(class_error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            class_error.to_string(),
            "duplicate generated class identifier `foo_bar` for CSS classes `foo-bar` and `foo_bar`"
        );

        std::fs::remove_file(stylesheet).unwrap();
        let first_template = templates.join("a.heml");
        std::fs::write(
            &first_template,
            r#"<form data-hemx-handle="save_a" data-hemx-form="profile-card"><input name="name"></form>"#,
        )
        .unwrap();
        let invalid_form = app()
            .template_dir(&templates)
            .out_dir(&out)
            .run()
            .unwrap_err();
        assert_eq!(invalid_form.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            invalid_form.to_string(),
            "invalid hemx form name `profile-card`; expected a Rust identifier"
        );

        std::fs::write(
            &first_template,
            r#"<form data-hemx-handle="save_a" data-hemx-form="profile"><input name="name"></form>"#,
        )
        .unwrap();
        std::fs::write(
            templates.join("b.heml"),
            r#"<form data-hemx-handle="save_b" data-hemx-form="profile"><input name="name"></form>"#,
        )
        .unwrap();
        let form_error = app()
            .template_dir(&templates)
            .out_dir(&out)
            .run()
            .unwrap_err();
        assert_eq!(form_error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            form_error.to_string(),
            "duplicate generated identifier `profile` for `a.heml::profile` and `b.heml::profile`"
        );

        std::fs::remove_file(templates.join("b.heml")).unwrap();
        std::fs::write(
            templates.join("a.heml"),
            r#"<form data-hemx-handle="save_name" data-hemx-form="profile"><input name="name" required></form><form data-hemx-handle="save_again" data-hemx-form="profile"><input name="name" required></form>"#,
        )
        .unwrap();
        app().template_dir(&templates).out_dir(&out).run().unwrap();
        let syms = std::fs::read_to_string(out.join("hemx.syms")).unwrap();
        assert_eq!(
            syms.matches("form_field\tprofile\tname\ttrue\tfalse\n")
                .count(),
            1
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn explicit_collection_generates_insertion_target() {
        let base = test_dir("hemx-build-keyed-collection-test");
        let templates = base.join("templates");
        let out = base.join("out");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&templates).unwrap();
        std::fs::write(
            templates.join("todos.heml"),
            r#"<ul data-hemx-collection="row"><template h-for="todo in &self.todos" h-key="todo.id"><li +data-hemx-key="todo.id">{+ todo +}</li></template></ul>"#,
        )
        .unwrap();

        app().template_dir(&templates).out_dir(&out).run().unwrap();

        let generated = std::fs::read_to_string(out.join("hemx.generated.rs")).unwrap();
        assert!(generated.contains("pub const row: ::hemx::advanced::Slot"));
        assert!(generated.contains("pub const row: CollectionTarget"));
        assert!(generated.contains("pub use self::targets::row;"));
        assert!(!generated.contains("pub const row: SlotTarget"));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn app_can_consume_precomputed_surface_facts() {
        let base = test_dir("hemx-build-precomputed-surface-test");
        let templates = base.join("templates");
        let out = base.join("out");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&templates).unwrap();
        let path = templates.join("todo.heml");
        let source = Arc::new(
            r#"<button data-hemx-handle="create" data-hemx-on="click">Create</button><div data-hemx-slot="todos"></div>"#
                .to_string(),
        );
        let ast = build_ast(source).unwrap().unwrap();
        let surface = extract_surface(&ast);

        app()
            .template_dir(&templates)
            .out_dir(&out)
            .surface(&path, surface)
            .run()
            .unwrap();

        let generated = std::fs::read_to_string(out.join("hemx.generated.rs")).unwrap();
        assert!(generated.contains("pub mod todo"));
        assert!(generated.contains("pub const create: ::hemx::Handle<()> = ::hemx::Handle::new("));
        assert!(generated.contains("pub const todos: ::hemx::advanced::Slot<::std::string::String> = ::hemx::advanced::Slot::new("));
        assert!(generated.contains("pub const todos: SlotTarget<::std::string::String> = SlotTarget::new(super::advanced::slots::todos);"));
        assert!(generated.contains("pub use self::targets::todos;"));
        assert!(generated
            .contains("pub const click: ::hemx::EventName = ::hemx::EventName::new(\"click\")"));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn global_resource_exports_are_explicit_opt_in() {
        let base = test_dir("hemx-build-no-global-exports-test");
        let templates = base.join("templates");
        let out = base.join("out");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&templates).unwrap();
        std::fs::write(
            templates.join("todo.heml"),
            r#"<button data-hemx-handle="create">Create</button><div data-hemx-slot="todos"></div>"#,
        )
        .unwrap();

        app().template_dir(&templates).out_dir(&out).run().unwrap();

        let generated = std::fs::read_to_string(out.join("hemx.generated.rs")).unwrap();
        assert!(!generated.contains("\n#[allow(non_upper_case_globals)]\npub mod components"));
        assert!(!generated.contains("\n#[allow(non_upper_case_globals)]\npub mod slots"));
        assert!(generated.contains(
            "\npub fn lower(html: impl ::std::convert::AsRef<str>) -> ::std::string::String"
        ));
        assert!(generated.contains("\n#[doc(hidden)]\npub fn lower_html"));
        assert!(generated.contains("\npub fn static_fragment(html: &'static str) -> ::hemx::Html"));
        assert!(generated.contains(
            "\n#[doc(hidden)]\npub fn render(view: &impl ::hemplate::Hemplate) -> ::hemx::Html"
        ));
        assert!(
            generated.contains("\npub fn page(view: &impl ::hemplate::Hemplate) -> ::hemx::Html")
        );
        assert!(generated.contains("\n#[doc(hidden)]\npub fn render_html(view: &impl ::hemplate::Hemplate) -> ::hemx::Html"));
        assert!(generated.contains("\npub fn put<T>(slot: ::hemx::advanced::Slot<T>, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect"));
        assert!(generated.contains("\npub fn replace<K, T>(slot: ::hemx::advanced::KeyedSlot<K, T>, key: K, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect"));
        assert!(generated.contains("pub mod todo"));
        assert!(generated.contains(
            "    pub const COMPONENT: ::hemx::ComponentRef = ::hemx::ComponentRef::new(\"todo\")"
        ));
        assert!(generated.contains("    pub mod advanced"));
        assert!(generated.contains("    pub mod slots"));
        assert!(generated.contains("    pub mod targets"));
        assert!(generated.contains("    pub const todos: SlotTarget<::std::string::String> = SlotTarget::new(super::advanced::slots::todos);"));
        assert!(generated.contains("    pub use self::targets::todos;"));
        assert!(generated.contains("    #[doc(hidden)]\n    pub fn lower_html"));
        assert!(generated.contains(
            "    #[doc(hidden)]\n    pub fn render(view: &impl ::hemplate::Hemplate) -> ::hemx::Html"
        ));
        assert!(
            generated.contains("    pub fn page(view: &impl ::hemplate::Hemplate) -> ::hemx::Html")
        );
        assert!(generated.contains("    #[doc(hidden)]\n    pub fn render_html(view: &impl ::hemplate::Hemplate) -> ::hemx::Html"));
        assert!(
            generated.contains("    pub fn static_fragment(html: &'static str) -> ::hemx::Html")
        );
        assert!(generated.contains("    pub fn put<T>(slot: ::hemx::advanced::Slot<T>, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect"));
        assert!(generated.contains("    pub fn replace<K, T>(slot: ::hemx::advanced::KeyedSlot<K, T>, key: K, view: &impl ::hemplate::Hemplate) -> ::hemx::advanced::Effect"));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn emits_form_contract_metadata_from_surface_controls() {
        let base = test_dir("hemx-build-form-contract-test");
        let templates = base.join("templates");
        let out = base.join("out");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&templates).unwrap();
        std::fs::write(
            templates.join("profile.heml"),
            r#"<form data-hemx-handle="save" data-hemx-form="profile">
                <input type="hidden" name="__h" value="123">
                <input name="title" required>
                <template h-if="alternate"><input name="title"></template>
                <input name="count" type="number" min="1" max="10" step="1">
                <select name="labels" multiple></select>
                <input name="avatar" type="file">
                <input name="token" type="hidden">
                <input name="enabled" type="checkbox">
                <input name="mode" type="radio">
                <textarea name="notes"></textarea>
                <input name="commit" type="submit">
                <input name="birthday" type="date">
                <button name="action" type="button">Preview</button>
            </form>"#,
        )
        .unwrap();

        app().template_dir(&templates).out_dir(&out).run().unwrap();

        let generated = std::fs::read_to_string(out.join("hemx.generated.rs")).unwrap();
        assert!(generated.contains("pub const PROFILE_CONTRACT: ::hemx::FormContract"));
        assert!(generated.contains("pub const PROFILE_FIELDS: &[::hemx::FormField]"));
        assert!(generated.contains(
            "::hemx::FormField { name: \"title\", kind: ::hemx::FormControlKind::Text, required: true }"
        ));
        assert!(generated.contains(
            "::hemx::FormField { name: \"count\", kind: ::hemx::FormControlKind::Number { min: Some(\"1\"), max: Some(\"10\"), step: Some(\"1\") }, required: false }"
        ));
        assert!(generated.contains(
            "::hemx::FormField { name: \"labels\", kind: ::hemx::FormControlKind::Select { multiple: true }, required: false }"
        ));
        assert!(generated.contains(
            "::hemx::FormField { name: \"avatar\", kind: ::hemx::FormControlKind::File, required: false }"
        ));
        for field in [
            "::hemx::FormField { name: \"token\", kind: ::hemx::FormControlKind::Hidden, required: false }",
            "::hemx::FormField { name: \"enabled\", kind: ::hemx::FormControlKind::Checkbox, required: false }",
            "::hemx::FormField { name: \"mode\", kind: ::hemx::FormControlKind::Radio, required: false }",
            "::hemx::FormField { name: \"notes\", kind: ::hemx::FormControlKind::TextArea, required: false }",
            "::hemx::FormField { name: \"commit\", kind: ::hemx::FormControlKind::Submit, required: false }",
            "::hemx::FormField { name: \"birthday\", kind: ::hemx::FormControlKind::Other { tag: \"input\", input_type: Some(\"date\") }, required: false }",
            "::hemx::FormField { name: \"action\", kind: ::hemx::FormControlKind::Other { tag: \"button\", input_type: None }, required: false }",
        ] {
            assert!(generated.contains(field), "missing form field metadata {field}");
        }

        let syms = std::fs::read_to_string(out.join("hemx.syms")).unwrap();
        assert!(syms.contains("handle_form\tsave\tprofile\n"));
        assert!(!syms.contains("form_field\tprofile\t__h\t"));
        assert_eq!(
            syms.matches("form_field\tprofile\ttitle\ttrue\tfalse\n")
                .count(),
            1,
            "conditional controls with one submitted name are one Rust form field"
        );
        assert!(syms.contains("form_field\tprofile\tlabels\tfalse\ttrue\n"));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn emits_checked_css_class_tokens_from_templates_and_stylesheets() {
        let base = test_dir("hemx-build-classes-test");
        let templates = base.join("templates");
        let out = base.join("out");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&templates).unwrap();
        std::fs::write(
            templates.join("card.heml"),
            r#"<article class="card is-active" data-hemx-slot="card_body"></article>"#,
        )
        .unwrap();
        std::fs::write(
            templates.join("card.css"),
            r#".card { padding: 1rem; }
.is-active:hover, .drag-handle { cursor: grab; }
.work-card.is-selected { outline: 1px solid currentColor; }
"#,
        )
        .unwrap();
        std::fs::write(
            templates.join("panel.scss"),
            r#".panel-shell { &.is-open { display: block; } }"#,
        )
        .unwrap();

        app()
            .template_dir(&templates)
            .out_dir(&out)
            .global_exports(true)
            .run()
            .unwrap();

        let generated = std::fs::read_to_string(out.join("hemx.generated.rs")).unwrap();
        assert!(generated.contains("pub mod classes"));
        assert!(generated
            .contains("pub const card: ::hemx::CssClass = ::hemx::CssClass::new(\"card\")"));
        assert!(generated.contains(
            "pub const is_active: ::hemx::CssClass = ::hemx::CssClass::new(\"is-active\")"
        ));
        assert!(generated.contains(
            "pub const drag_handle: ::hemx::CssClass = ::hemx::CssClass::new(\"drag-handle\")"
        ));
        assert!(generated.contains(
            "pub const work_card: ::hemx::CssClass = ::hemx::CssClass::new(\"work-card\")"
        ));
        assert!(generated.contains(
            "pub const is_selected: ::hemx::CssClass = ::hemx::CssClass::new(\"is-selected\")"
        ));
        assert!(generated.contains("pub mod card"));
        assert!(generated.contains("pub mod panel"));

        let syms = std::fs::read_to_string(out.join("hemx.syms")).unwrap();
        assert!(syms.contains("class\t"));
        assert!(syms.contains("\tdrag_handle\tdrag-handle"));

        let source = format!(
            r###"
#![allow(dead_code)]
mod hemx {{
    #[derive(Clone, Copy)] pub struct BuildFingerprint(pub u64);
    impl BuildFingerprint {{ pub const fn from_parts(_: &[u32]) -> Self {{ Self(4242) }} }}
    pub struct Effect;
    #[derive(Clone, Copy)] pub struct ResourceId;
    #[derive(Clone, Copy)] pub struct EventName(&'static str);
    impl EventName {{ pub const fn new(name: &'static str) -> Self {{ Self(name) }} }}
    pub trait IntoEffect {{}}
    impl IntoEffect for Effect {{}}
    pub trait GeneratedTarget {{ fn __hemx_resource_id(self) -> ResourceId; }}
    pub trait KeyedPartial {{ fn hemx_key(&self) -> String; }}
    #[derive(Clone, Copy)] pub struct Slot<T>(::std::marker::PhantomData<T>);
    impl<T> Slot<T> {{ pub const fn new(_: u32) -> Self {{ Self(::std::marker::PhantomData) }} pub const fn id(self) -> ResourceId {{ ResourceId }} pub fn html(self, _: impl ::std::convert::Into<SafeHtml>) -> Effect {{ Effect }} pub fn text(self, _: impl ::std::string::ToString) -> Effect {{ Effect }} pub fn rows_html<O, K, H>(self, _: impl ::std::iter::IntoIterator<Item = O>, _: impl ::std::iter::IntoIterator<Item = (K, H)>) -> Effect {{ Effect }} pub fn rows_delta_html<B, A, K, H>(self, _: impl ::std::iter::IntoIterator<Item = B>, _: impl ::std::iter::IntoIterator<Item = A>, _: impl ::std::iter::IntoIterator<Item = (K, H)>) -> Effect {{ Effect }} }}
    pub struct KeyedSlot<K, T>(::std::marker::PhantomData<(K, T)>); impl<K, T> Copy for KeyedSlot<K, T> {{}} impl<K, T> Clone for KeyedSlot<K, T> {{ fn clone(&self) -> Self {{ *self }} }}
    impl<K, T> KeyedSlot<K, T> {{ pub const fn new(_: u32) -> Self {{ Self(::std::marker::PhantomData) }} pub const fn id(self) -> ResourceId {{ ResourceId }} pub fn replace_html(self, _: K, _: impl ::std::convert::Into<SafeHtml>) -> Effect {{ Effect }} pub fn remove(self, _: K) -> Effect {{ Effect }} pub fn move_before(self, _: K, _: K) -> Effect {{ Effect }} pub fn move_to_end(self, _: K) -> Effect {{ Effect }} }}
    #[derive(Clone, Copy)] pub struct Handle<T>(::std::marker::PhantomData<T>);
    impl<T> Handle<T> {{ pub const fn new(_: u32) -> Self {{ Self(::std::marker::PhantomData) }} }}
    #[derive(Clone, Copy)] pub struct Form<T>(::std::marker::PhantomData<T>);
    impl<T> Form<T> {{ pub const fn new(_: u32) -> Self {{ Self(::std::marker::PhantomData) }} }}
    #[derive(Clone, Copy)] pub struct CssClass(&'static str);
    impl CssClass {{ pub const fn new(name: &'static str) -> Self {{ Self(name) }} pub const fn as_str(self) -> &'static str {{ self.0 }} }}
    #[derive(Clone, Copy)] pub struct ComponentRef(&'static str);
    impl ComponentRef {{ pub const fn new(name: &'static str) -> Self {{ Self(name) }} pub const fn as_str(self) -> &'static str {{ self.0 }} }}
    pub struct SafeHtml(String);
    impl SafeHtml {{ pub fn trusted(html: impl Into<String>) -> Self {{ Self(html.into()) }} }}
    pub struct Html(SafeHtml);
    impl ::std::convert::From<Html> for SafeHtml {{ fn from(value: Html) -> Self {{ value.0 }} }}
    impl ::std::convert::AsRef<str> for Html {{ fn as_ref(&self) -> &str {{ "" }} }}
    pub mod __private {{ pub fn html_trusted(html: impl Into<String>) -> super::Html {{ super::Html(super::SafeHtml::trusted(html)) }} }}
    pub mod advanced {{ pub use super::*; }}
    pub struct FormContract {{ pub fields: &'static [FormField] }}
    pub struct FormField {{ pub name: &'static str, pub kind: FormControlKind, pub required: bool }}
    pub enum FormControlKind {{ Text, Number {{ min: Option<&'static str>, max: Option<&'static str>, step: Option<&'static str> }}, Checkbox, Radio, Select {{ multiple: bool }}, TextArea, File, Hidden, Submit, Other {{ tag: &'static str, input_type: Option<&'static str> }} }}
}}
mod hemplate {{
    pub trait Hemplate {{ fn render_into(&self, out: &mut String) -> Result<(), ()>; }}
}}
{generated}
fn main() {{
    assert_eq!(classes::drag_handle.as_str(), "drag-handle");
    assert_eq!(card::classes::is_active.as_str(), "is-active");
    assert_eq!(panel::classes::panel_shell.as_str(), "panel-shell");
}}
"###,
        );
        let source_path = base.join("classes.rs");
        let bin_path = base.join("classes-bin");
        std::fs::write(&source_path, source).unwrap();
        let status = std::process::Command::new("rustc")
            .arg(&source_path)
            .arg("-o")
            .arg(&bin_path)
            .status()
            .unwrap();
        assert!(status.success());
        let status = std::process::Command::new(&bin_path).status().unwrap();
        assert!(status.success());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn diagnostics_for_heml_file_reports_unkeyed_generated_target() {
        let dir = test_dir("diagnostics_for_heml_file_reports_unkeyed_generated_target");
        std::fs::create_dir_all(&dir).expect("create test dir");
        let template = dir.join("todo.heml");
        std::fs::write(
            &template,
            r#"<main data-hemx-root="todos"><template h-for="todo in &self.todos"><li data-hemx-slot="todo_row">{+ todo.title +}</li></template></main>"#,
        )
        .expect("write template");

        let diagnostics = diagnostics_for_heml_file(&template).expect("diagnostics");
        assert_eq!(diagnostics.len(), 1);
        let diagnostic = &diagnostics[0];
        assert_eq!(diagnostic.code, DiagnosticCode::UnkeyedGeneratedTarget);
        assert_eq!(diagnostic.directive, "data-hemx-slot");
        assert_eq!(diagnostic.target, "todo_row");
        assert!(diagnostic.repair.contains("h-key=\"todo.id\""));
    }

    #[test]
    fn generated_targets_for_heml_source_uses_surface_facts() {
        let targets = generated_targets_for_heml_source(
            "inline.heml",
            r#"
                <main data-hemx-root="todos">
                    <section class="card" data-hemx-slot="summary"></section>
                    <section data-hemx-root="nested" data-hemx-slot="after-root"></section>
                    <section data-hemx-slot="summary"></section>
                    <form data-hemx-form="summary"></form>
                    <span data-hemx-handle data-hemx-slot="after-missing"></span>
                    <button data-hemx-handle="submit">Save</button>
                    <aside data-hemx-slot="later"></aside>
                </main>
            "#,
        )
        .expect("generated targets");

        assert_eq!(
            targets,
            vec![
                GeneratedTarget {
                    kind: "slot".into(),
                    name: "summary".into(),
                },
                GeneratedTarget {
                    kind: "slot".into(),
                    name: "after-root".into(),
                },
                GeneratedTarget {
                    kind: "form".into(),
                    name: "summary".into(),
                },
                GeneratedTarget {
                    kind: "slot".into(),
                    name: "after-missing".into(),
                },
                GeneratedTarget {
                    kind: "handle".into(),
                    name: "submit".into(),
                },
                GeneratedTarget {
                    kind: "slot".into(),
                    name: "later".into(),
                },
            ]
        );
    }

    #[test]
    fn template_context_facts_include_self_fields_and_h_for_local_fields() {
        let dir = test_dir("template_context_facts_include_self_fields_and_h_for_local_fields");
        std::fs::create_dir_all(dir.join("src")).expect("create src dir");
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"facts\"\nversion = \"0.1.0\"\n",
        )
        .expect("write manifest");
        std::fs::write(
            dir.join("src/lib.rs"),
            r#"
                #[derive(Clone)]
                pub struct ExercisePlan { pub name: String, pub kg: f32 }

                #[derive(Hemplate)]
                pub struct Workout {
                    pub plan: Vec<ExercisePlan>,
                    pub alternate: std::vec::Vec<ExercisePlan>,
                    pub mysteries: Vec<MissingFact>,
                    pub progress: String,
                }
            "#,
        )
        .expect("write lib");
        let template = dir.join("workout.heml");
        let source = r#"
            <main title="ghost in self.plan">
                <p h-for="malformed">{+ self.progress +}</p>
                <p h-for="(tuple, item) in self.plan">ignored</p>
                <p h-for="unknown in self.unknown">ignored</p>
                <p h-for="scalar in self.progress">ignored</p>
                <li h-for="exercise in &self.plan">{+ exercise.name +}</li>
                <li h-for="exercise in self.alternate">{+ exercise.kg +}</li>
                <li h-for="mystery in self.mysteries">{+ mystery.anything +}</li>
                <li h-for="later in self.plan[0]">{+ later.name +}</li>
            </main>
        "#;
        std::fs::write(&template, source).expect("write heml");

        let facts = template_context_facts_for_heml_source(&template, source)
            .expect("facts")
            .expect("derive facts");

        assert_eq!(facts.context_type, "Workout");
        assert!(facts
            .self_fields
            .iter()
            .any(|field| field.name == "progress" && field.type_name == "String"));
        let local = facts
            .locals
            .iter()
            .find(|local| local.name == "exercise")
            .expect("exercise local");
        assert_eq!(local.type_name, "ExercisePlan");
        assert!(local
            .fields
            .iter()
            .any(|field| field.name == "name" && field.type_name == "String"));
        assert_eq!(
            facts
                .locals
                .iter()
                .filter(|local| local.name == "exercise")
                .count(),
            1
        );
        let mystery = facts
            .locals
            .iter()
            .find(|local| local.name == "mystery")
            .expect("unknown element type still exposes the local");
        assert_eq!(mystery.type_name, "MissingFact");
        assert!(mystery.fields.is_empty());
        assert!(facts.locals.iter().any(|local| {
            local.name == "later"
                && local.type_name == "ExercisePlan"
                && local.fields.iter().any(|field| field.name == "kg")
        }));
        for rejected in ["ghost", "tuple", "item", "unknown", "scalar"] {
            assert!(
                facts.locals.iter().all(|local| local.name != rejected),
                "unexpected loop local {rejected}"
            );
        }

        assert_eq!(
            h_for_local_and_self_field(" item in &self.plan "),
            Some(("item".into(), "plan".into()))
        );
        assert_eq!(
            h_for_local_and_self_field("item in self.plan.iter()"),
            Some(("item".into(), "plan".into()))
        );
        for invalid in [
            "item:self.plan",
            " in self.plan",
            "1item in self.plan",
            "item-name in self.plan",
            "(item, index) in self.plan",
            "item in plan",
            "item in &plan",
            "item in self.",
        ] {
            assert_eq!(
                h_for_local_and_self_field(invalid),
                None,
                "accepted invalid h-for {invalid:?}"
            );
        }
    }

    #[test]
    fn unkeyed_generated_target_diagnostic_is_structured() {
        let diagnostic = unkeyed_generated_target_diagnostic(
            Path::new("templates/todo.heml"),
            "slot",
            "todo_row",
            "todo",
            "&self.todos",
        );

        assert_eq!(diagnostic.code, DiagnosticCode::UnkeyedGeneratedTarget);
        assert_eq!(diagnostic.severity, DiagnosticSeverity::Error);
        assert_eq!(diagnostic.file, PathBuf::from("templates/todo.heml"));
        assert_eq!(diagnostic.directive, "data-hemx-slot");
        assert_eq!(diagnostic.target, "todo_row");
        assert!(diagnostic
            .message
            .contains("h-for=\"todo in &self.todos\" without h-key"));
        assert!(diagnostic.expected.contains("ui::todo_row.replace(row)"));
        assert!(diagnostic.repair.contains("h-key=\"todo.id\""));
        assert!(diagnostic
            .repair
            .contains("dynamic +data-hemx-key on the child"));
        assert!(diagnostic.to_string().contains("templates/todo.heml"));
    }

    #[test]
    fn rejects_hemx_resources_inside_unkeyed_for() {
        for (case, template, resource, helper) in [
            (
                "slot",
                r#"<template h-for="todo in &self.todos"><li data-hemx-slot="todo_row">{+ todo.title +}</li></template>"#,
                "data-hemx-slot=\"todo_row\"",
                "ui::todo_row",
            ),
            (
                "slot_data_key",
                r#"<template h-for="todo in &self.todos"><li data-hemx-slot="todo_row" +data-hemx-key="todo.id">{+ todo.title +}</li></template>"#,
                "data-hemx-slot=\"todo_row\"",
                "ui::todo_row",
            ),
            (
                "handle",
                r#"<template h-for="todo in &self.todos"><button data-hemx-handle="delete">Delete</button></template>"#,
                "data-hemx-handle=\"delete\"",
                "ui::delete",
            ),
            (
                "nested-handle",
                r#"<template h-for="todo in &self.todos"><template h-if="todo.visible"><button data-hemx-handle="delete">Delete</button></template></template>"#,
                "data-hemx-handle=\"delete\"",
                "ui::delete",
            ),
        ] {
            let base = test_dir(&format!("hemx-build-unkeyed-for-{case}-test"));
            let templates = base.join("templates");
            let out = base.join("out");
            let _ = std::fs::remove_dir_all(&base);
            std::fs::create_dir_all(&templates).unwrap();
            std::fs::write(templates.join("todo.heml"), template).unwrap();

            let err = app()
                .template_dir(&templates)
                .out_dir(&out)
                .run()
                .unwrap_err();
            let err = err.to_string();
            assert!(err.contains("inside h-for=\"todo in &self.todos\" without h-key"));
            assert!(err.contains(resource));
            assert!(err.contains("h-key=\"todo.id\""));
            assert!(err.contains("generated keyed helpers"));
            assert!(err.contains(helper));
            assert!(err.contains("dynamic +data-hemx-key on the child"));

            let _ = std::fs::remove_dir_all(&base);
        }

        let base = test_dir("hemx-build-keyed-nested-resources-test");
        let templates = base.join("templates");
        let out = base.join("out");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&templates).unwrap();
        let keyed_source = r#"
            <main>
                <ul data-hemx-slot="todos">
                    <template h-for="todo in &self.todos" h-key="todo.id">
                        <li data-hemx-slot="todo_row">
                            <template h-for="child in &todo.children" h-key="child.id">
                                <button data-hemx-handle="delete">Delete</button>
                            </template>
                        </li>
                    </template>
                    <template h-for="other in &self.others"><li>Unaddressed sibling</li></template>
                </ul>
                <aside data-hemx-slot="summary"></aside>
            </main>
        "#;
        std::fs::write(templates.join("todo.heml"), keyed_source).unwrap();
        app().template_dir(&templates).out_dir(&out).run().unwrap();
        let syms = std::fs::read_to_string(out.join("hemx.syms")).unwrap();
        assert!(syms.contains("slot\ttodo.heml::todos\ttodos\t"));
        assert!(syms.contains("slot\ttodo.heml::todo_row\ttodo_row\t"));
        assert!(syms.contains("handle\ttodo.heml::delete\tdelete\t"));
        assert!(syms.contains("slot\ttodo.heml::summary\tsummary\t"));
        let generated = std::fs::read_to_string(out.join("hemx.generated.rs")).unwrap();
        assert!(generated.contains("pub const todo_row: ::hemx::advanced::KeyedSlot"));
        assert!(generated.contains("pub const todos: ::hemx::advanced::Slot"));
        assert!(generated.contains("pub const summary: ::hemx::advanced::Slot"));
        assert!(!generated.contains("pub const summary: ::hemx::advanced::KeyedSlot"));

        let surface =
            surface_for_heml_source(Path::new("todo.heml"), keyed_source.to_owned()).unwrap();
        let keyed_scope = surface
            .scopes
            .iter()
            .position(|scope| {
                matches!(
                    scope.kind,
                    ScopeKind::For {
                        key_expr: Some(_),
                        ..
                    }
                )
            })
            .map(|index| ScopeId(index as u32))
            .unwrap();
        let nested_scope = surface
            .scopes
            .iter()
            .enumerate()
            .find(|(_, scope)| scope.parent == Some(keyed_scope))
            .map(|(index, _)| ScopeId(index as u32))
            .unwrap();
        let sibling_scope = surface
            .scopes
            .iter()
            .enumerate()
            .find(|(_, scope)| {
                matches!(scope.kind, ScopeKind::For { key_expr: None, .. })
                    && scope.parent != Some(keyed_scope)
            })
            .map(|(index, _)| ScopeId(index as u32))
            .unwrap();
        assert!(is_inside_keyed_for(&surface, nested_scope));
        assert!(!is_inside_keyed_for(&surface, sibling_scope));
        assert!(!is_inside_keyed_for(&surface, ScopeId(u32::MAX)));
        assert!(unkeyed_generated_target_diagnostic_for_scope(
            &surface,
            ScopeId(u32::MAX),
            Path::new("todo.heml"),
            "slot",
            "missing",
        )
        .is_none());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn rejects_selector_style_targeting_attrs() {
        for (case, template, attr) in [
            (
                "hemx-target",
                r##"<button data-hemx-handle="save" data-hemx-target="#row">Save</button>"##,
                "data-hemx-target",
            ),
            (
                "hemx-select",
                r##"<button data-hemx-handle="save" data-hemx-select="closest tr">Save</button>"##,
                "data-hemx-select",
            ),
        ] {
            let base = test_dir(&format!("hemx-build-selector-target-{case}-test"));
            let templates = base.join("templates");
            let out = base.join("out");
            let _ = std::fs::remove_dir_all(&base);
            std::fs::create_dir_all(&templates).unwrap();
            let template_path = templates.join("target.heml");
            std::fs::write(&template_path, template).unwrap();

            let err = app()
                .template_dir(&templates)
                .out_dir(&out)
                .run()
                .unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidData);
            assert_eq!(
                err.to_string(),
                format!(
                    "{}: `{attr}` is selector-style targeting; hemx uses generated resources instead. Add data-hemx-slot to the local element and return an effect for that generated slot.",
                    template_path.display()
                )
            );

            let _ = std::fs::remove_dir_all(&base);
        }
    }

    #[test]
    fn rejects_unknown_hemx_authoring_attrs() {
        let base = test_dir("hemx-build-unknown-hemx-attr-test");
        let templates = base.join("templates");
        let out = base.join("out");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&templates).unwrap();
        let template_path = templates.join("typo.heml");
        std::fs::write(
            &template_path,
            r#"<button data-hemx-handle="save" data-hemx-pendig-class="busy">Save</button>"#,
        )
        .unwrap();

        let err = app()
            .template_dir(&templates)
            .out_dir(&out)
            .run()
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            err.to_string(),
            format!(
                "{}: unknown hemx attribute `data-hemx-pendig-class`; check the spelling or use a non-hemx data-* attribute for app-specific metadata",
                template_path.display()
            )
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn rejects_invalid_static_hemx_convention_values() {
        for (case, template, attr, guidance) in [
            (
                "policy",
                r#"<button data-hemx-handle="save" data-hemx-policy="newest">Save</button>"#,
                "data-hemx-policy",
                "latest",
            ),
            (
                "policy-missing",
                r#"<button data-hemx-handle="save" data-hemx-policy>Save</button>"#,
                "data-hemx-policy",
                "latest",
            ),
            (
                "events-mixed",
                r#"<button data-hemx-handle="save" data-hemx-on="click blur">Save</button>"#,
                "data-hemx-on",
                "runtime-supported",
            ),
            (
                "confirm-empty",
                r#"<button data-hemx-handle="save" data-hemx-confirm=" ">Save</button>"#,
                "data-hemx-confirm",
                "non-empty",
            ),
            (
                "sse-empty",
                r#"<main data-hemx-root="app" data-hemx-sse=" "></main>"#,
                "data-hemx-sse",
                "non-empty",
            ),
            (
                "revealed-ahead-negative",
                r#"<form data-hemx-revealed data-hemx-revealed-ahead="-1"></form>"#,
                "data-hemx-revealed-ahead",
                "non-negative",
            ),
            (
                "throttle-invalid",
                r#"<button data-hemx-handle="save" data-hemx-throttle="ms">Save</button>"#,
                "data-hemx-throttle",
                "milliseconds",
            ),
            (
                "policy-duplicate-anchor",
                r#"<button data-hemx-handle="save" data-hemx-policy="newest">Save</button>"#,
                "data-hemx-policy",
                "latest",
            ),
            (
                "debounce",
                r#"<button data-hemx-handle="save" data-hemx-debounce="soon">Save</button>"#,
                "data-hemx-debounce",
                "250ms",
            ),
            (
                "every",
                r#"<button data-hemx-handle="save" data-hemx-every="1sec">Save</button>"#,
                "data-hemx-every",
                "1s",
            ),
            (
                "interval",
                r#"<button data-hemx-handle="save" data-hemx-interval="often">Save</button>"#,
                "data-hemx-interval",
                "1s",
            ),
            (
                "history",
                r#"<form method="get" action="/search" data-hemx-history="append"><input name="q"></form>"#,
                "data-hemx-history",
                "replace",
            ),
            (
                "event",
                r#"<button data-hemx-handle="save" data-hemx-on="blur">Save</button>"#,
                "data-hemx-on",
                "click",
            ),
            (
                "empty-event",
                r#"<button data-hemx-handle="save" data-hemx-on="">Save</button>"#,
                "data-hemx-on",
                "click",
            ),
            (
                "empty-confirm",
                r#"<button data-hemx-handle="delete" data-hemx-confirm="">Delete</button>"#,
                "data-hemx-confirm",
                "non-empty",
            ),
            (
                "empty-sse",
                r#"<section data-hemx-root="notifications" data-hemx-sse=""></section>"#,
                "data-hemx-sse",
                "same-origin SSE URL",
            ),
        ] {
            let base = test_dir(&format!("hemx-build-invalid-convention-{case}-test"));
            let templates = base.join("templates");
            let out = base.join("out");
            let _ = std::fs::remove_dir_all(&base);
            std::fs::create_dir_all(&templates).unwrap();
            let template_path = templates.join("invalid.heml");
            std::fs::write(&template_path, template).unwrap();

            let err = app()
                .template_dir(&templates)
                .out_dir(&out)
                .run()
                .unwrap_err();
            let surface = surface_for_heml_source(&template_path, template.to_owned()).unwrap();
            let value = surface
                .nodes
                .iter()
                .flat_map(|node| &node.attrs)
                .find(|candidate| candidate.name == attr)
                .and_then(|candidate| candidate.value.as_deref())
                .unwrap_or("");
            let expectation = match attr {
                "data-hemx-policy" => {
                    "expected one of `latest`, `queue`, `drop`, or `parallel`"
                }
                "data-hemx-history" => {
                    "expected `push`, `replace`, or empty for the default push behavior"
                }
                "data-hemx-revealed-ahead" => "expected a non-negative number of viewports",
                "data-hemx-on" => "expected runtime-supported events: `click`, `submit`, `input`, `change`, `keydown`, `dragstart`, `dragover`, or `drop`",
                "data-hemx-confirm" => "expected a non-empty confirmation message",
                "data-hemx-sse" => "expected a non-empty same-origin SSE URL",
                "data-hemx-debounce" | "data-hemx-throttle" | "data-hemx-every"
                | "data-hemx-interval" => {
                    "expected milliseconds like `250`/`250ms` or seconds like `1s`"
                }
                _ => panic!("missing expectation for {attr}"),
            };
            assert_eq!(err.kind(), io::ErrorKind::InvalidData);
            assert_eq!(
                err.to_string(),
                format!(
                    "{}: invalid {attr} value `{value}`; {expectation}",
                    template_path.display()
                ),
                "guidance anchor {guidance:?}"
            );

            let _ = std::fs::remove_dir_all(&base);
        }
    }

    #[test]
    fn static_attribute_values_integrate_with_template_inspection() {
        let valid_source = r#"
            <main data-hemx-root="app" data-hemx-sse="/events">
                <button data-hemx-handle="save" data-hemx-policy="latest" data-hemx-on="click change" data-hemx-confirm="Save?" data-hemx-debounce="250ms" data-hemx-throttle="1s">Save</button>
                <form method="get" action="/search" data-hemx-history="replace"><input name="q"></form>
            </main>
        "#;
        let base = test_dir("hemx-build-valid-static-values");
        let templates = base.join("templates");
        let out = base.join("out");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&templates).unwrap();
        std::fs::write(templates.join("valid.heml"), valid_source).unwrap();
        app().template_dir(&templates).out_dir(&out).run().unwrap();
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn placement_contracts_fail_closed_with_exact_diagnostics() {
        for (case, template, attr, expectation) in [
            (
                "nav-button",
                r#"<button data-hemx-nav="" data-hemx-handle="go">Go</button>"#,
                "data-hemx-nav",
                "expected a real `<a href=...>` link so navigation works without JavaScript",
            ),
            (
                "nav-missing-href",
                r#"<a data-hemx-nav="">Docs</a>"#,
                "data-hemx-nav",
                "expected a real `<a href=...>` link so navigation works without JavaScript",
            ),
            (
                "nav-empty-href",
                r#"<a href="  " data-hemx-nav="">Docs</a>"#,
                "data-hemx-nav",
                "expected a real `<a href=...>` link so navigation works without JavaScript",
            ),
            (
                "boost-anchor",
                r#"<a href="/docs" data-hemx-boost="">Docs</a>"#,
                "data-hemx-boost",
                "expected a container around descendant links/forms; use `data-hemx-nav` on anchors or `data-hemx-handle` on forms",
            ),
            (
                "boost-form",
                r#"<form data-hemx-boost=""><input name="q"></form>"#,
                "data-hemx-boost",
                "expected a container around descendant links/forms; use `data-hemx-nav` on anchors or `data-hemx-handle` on forms",
            ),
            (
                "sse-child",
                r#"<section data-hemx-root="feed"><div data-hemx-sse="/events"></div></section>"#,
                "data-hemx-sse",
                "expected placement on the same element as `data-hemx-root`",
            ),
        ] {
            let base = test_dir(&format!("hemx-build-invalid-placement-{case}-test"));
            let templates = base.join("templates");
            let out = base.join("out");
            let template_path = templates.join("placement.heml");
            let _ = std::fs::remove_dir_all(&base);
            std::fs::create_dir_all(&templates).unwrap();
            std::fs::write(&template_path, template).unwrap();

            let error = app()
                .template_dir(&templates)
                .out_dir(&out)
                .run()
                .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert_eq!(
                error.to_string(),
                format!(
                    "{}: invalid {attr} placement; {expectation}",
                    template_path.display()
                )
            );

            let _ = std::fs::remove_dir_all(&base);
        }

        for (case, template) in [
            ("nav-link", r#"<a href="/docs" data-hemx-nav="">Docs</a>"#),
            (
                "boost-container",
                r#"<nav data-hemx-boost=""><a href="/docs">Docs</a></nav>"#,
            ),
            (
                "sse-root",
                r#"<main data-hemx-root="feed" data-hemx-sse="/events"></main>"#,
            ),
        ] {
            let base = test_dir(&format!("hemx-build-valid-placement-{case}-test"));
            let templates = base.join("templates");
            let out = base.join("out");
            let _ = std::fs::remove_dir_all(&base);
            std::fs::create_dir_all(&templates).unwrap();
            std::fs::write(templates.join("placement.heml"), template).unwrap();
            app().template_dir(&templates).out_dir(&out).run().unwrap();
            let _ = std::fs::remove_dir_all(base);
        }
    }

    #[test]
    fn websocket_attribute_is_rejected_with_its_replacement() {
        let base = test_dir("hemx-build-removed-websocket");
        let templates = base.join("templates");
        let template_path = templates.join("room.heml");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&templates).unwrap();
        std::fs::write(
            &template_path,
            r#"<main data-hemx-root="room" data-hemx-ws="/room/socket"></main>"#,
        )
        .unwrap();
        let error = app()
            .template_dir(&templates)
            .out_dir(base.join("out"))
            .run()
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        let message = error.to_string();
        assert!(
            message.starts_with(&format!(
                "{}: `data-hemx-ws` was removed",
                template_path.display()
            )),
            "{message}"
        );
        assert!(message.contains("data-hemx-sse") && message.contains("window.hemx.applyBatch"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn generated_lowering_preserves_native_interaction_contract() {
        let base = test_dir("hemx-build-lowering-test");
        let templates = base.join("templates");
        let out = base.join("out");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&templates).unwrap();
        std::fs::write(
            templates.join("todo.heml"),
            r#"<main data-hemx-root><a href="/docs">Docs</a><form action="/todos" method="post" data-hemx-handle="create"><label for="title">Title</label><input id="title" name="title" aria-invalid="true" aria-describedby="title-error"><span id="title-error">Required</span><button data-hemx-on="click">Create</button></form><form action="/search" method="get"><input name="q"></form><li data-hemx-slot="row" data-hemx-key="7"></li></main>"#,
        )
        .unwrap();

        app().template_dir(&templates).out_dir(&out).run().unwrap();
        let generated = std::fs::read_to_string(out.join("hemx.generated.rs")).unwrap();
        assert!(generated.contains("fn __hemx_lower_html"));
        let source = format!(
            r###"
#![allow(dead_code)]
mod hemx {{
    #[derive(Clone, Copy)] pub struct BuildFingerprint(pub u64);
    impl BuildFingerprint {{ pub const fn from_parts(_: &[u32]) -> Self {{ Self(4242) }} }}
    pub struct Effect;
    #[derive(Clone, Copy)] pub struct ResourceId;
    #[derive(Clone, Copy)] pub struct EventName(&'static str);
    impl EventName {{ pub const fn new(name: &'static str) -> Self {{ Self(name) }} }}
    pub trait IntoEffect {{}}
    impl IntoEffect for Effect {{}}
    pub trait GeneratedTarget {{ fn __hemx_resource_id(self) -> ResourceId; }}
    pub trait KeyedPartial {{ fn hemx_key(&self) -> String; }}
    #[derive(Clone, Copy)] pub struct Slot<T>(::std::marker::PhantomData<T>);
    impl<T> Slot<T> {{ pub const fn new(_: u32) -> Self {{ Self(::std::marker::PhantomData) }} pub const fn id(self) -> ResourceId {{ ResourceId }} pub fn html(self, _: impl ::std::convert::Into<SafeHtml>) -> Effect {{ Effect }} pub fn text(self, _: impl ::std::string::ToString) -> Effect {{ Effect }} pub fn rows_html<O, K, H>(self, _: impl ::std::iter::IntoIterator<Item = O>, _: impl ::std::iter::IntoIterator<Item = (K, H)>) -> Effect {{ Effect }} pub fn rows_delta_html<B, A, K, H>(self, _: impl ::std::iter::IntoIterator<Item = B>, _: impl ::std::iter::IntoIterator<Item = A>, _: impl ::std::iter::IntoIterator<Item = (K, H)>) -> Effect {{ Effect }} }}
    pub struct KeyedSlot<K, T>(::std::marker::PhantomData<(K, T)>); impl<K, T> Copy for KeyedSlot<K, T> {{}} impl<K, T> Clone for KeyedSlot<K, T> {{ fn clone(&self) -> Self {{ *self }} }}
    impl<K, T> KeyedSlot<K, T> {{ pub const fn new(_: u32) -> Self {{ Self(::std::marker::PhantomData) }} pub const fn id(self) -> ResourceId {{ ResourceId }} pub fn replace_html(self, _: K, _: impl ::std::convert::Into<SafeHtml>) -> Effect {{ Effect }} pub fn remove(self, _: K) -> Effect {{ Effect }} pub fn move_before(self, _: K, _: K) -> Effect {{ Effect }} pub fn move_to_end(self, _: K) -> Effect {{ Effect }} }}
    #[derive(Clone, Copy)] pub struct Handle<T>(::std::marker::PhantomData<T>);
    impl<T> Handle<T> {{ pub const fn new(_: u32) -> Self {{ Self(::std::marker::PhantomData) }} }}
    #[derive(Clone, Copy)] pub struct Form<T>(::std::marker::PhantomData<T>);
    impl<T> Form<T> {{ pub const fn new(_: u32) -> Self {{ Self(::std::marker::PhantomData) }} }}
    #[derive(Clone, Copy)] pub struct ComponentRef(&'static str);
    impl ComponentRef {{ pub const fn new(name: &'static str) -> Self {{ Self(name) }} pub const fn as_str(self) -> &'static str {{ self.0 }} }}
    pub struct SafeHtml(String);
    impl SafeHtml {{ pub fn trusted(html: impl Into<String>) -> Self {{ Self(html.into()) }} }}
    pub struct Html(SafeHtml);
    impl ::std::convert::From<Html> for SafeHtml {{ fn from(value: Html) -> Self {{ value.0 }} }}
    impl ::std::convert::AsRef<str> for Html {{ fn as_ref(&self) -> &str {{ "" }} }}
    pub mod __private {{ pub fn html_trusted(html: impl Into<String>) -> super::Html {{ super::Html(super::SafeHtml::trusted(html)) }} }}
    pub mod advanced {{ pub use super::*; }}
    pub struct FormContract {{ pub fields: &'static [FormField] }}
    pub struct FormField {{ pub name: &'static str, pub kind: FormControlKind, pub required: bool }}
    pub enum FormControlKind {{ Text, Number {{ min: Option<&'static str>, max: Option<&'static str>, step: Option<&'static str> }}, Checkbox, Radio, Select {{ multiple: bool }}, TextArea, File, Hidden, Submit, Other {{ tag: &'static str, input_type: Option<&'static str> }} }}
}}
mod hemplate {{
    pub trait Hemplate {{ fn render_into(&self, out: &mut String) -> Result<(), ()>; }}
}}
{generated}
fn main() {{
    let html = todo::lower_html(r#"<main data-hemx-root><a href="/docs">Docs</a><form action="/todos" method="post" data-hemx-handle="create"><label for="title">Title</label><input id="title" name="title" aria-invalid="true" aria-describedby="title-error"><span id="title-error">Required</span><button data-hemx-on="click">Create</button></form><form action="/search" method="get"><input name="q"></form><li data-hemx-slot="row" h-key="7"></li></main>"#);
    assert!(html.contains(r#"data-hemx-root"#));
    assert!(html.contains(r#"data-hemx-action="#));
    assert!(html.contains(r#"data-hemx-resource="#));
    assert!(html.contains(r#"data-hemx-key="7""#));
    assert!(html.contains(r#"data-hemx-on="click""#));
    assert!(html.contains(r#"<a href="/docs">"#));
    assert!(html.contains(r#"action="/todos" method="post""#));
    assert!(html.contains(r#"action="/search" method="get""#));
    assert!(html.contains(r#"aria-invalid="true""#));
    assert!(html.contains(r#"aria-describedby="title-error""#));
    assert!(html.contains(r#"id="title-error">Required"#));
    assert!(html.contains(r#"name="__h""#));
    // Only the handled form carries the build; the GET search form does not.
    let build = format!(r#"<input type="hidden" name="__b" value="{{}}">"#, BUILD_FINGERPRINT.0);
    assert_eq!(BUILD_FINGERPRINT.0, 4242);
    assert_eq!(html.matches(&build).count(), 1, "{{html}}");
    assert_eq!(html.matches(r#"name="__b""#).count(), 1);
    // A hand-written field is kept, not duplicated.
    let own = todo::lower_html(r#"<form data-hemx-handle="create"><input type="hidden" name="__b" value="1"></form>"#);
    assert_eq!(own.matches(r#"name="__b""#).count(), 1, "{{own}}");
    assert!(!html.contains("data-hid"));
    assert!(!html.contains("data-sid"));
    assert!(!html.contains("h-key"));
}}
"###,
        );
        let source_path = base.join("lowering.rs");
        let bin_path = base.join("lowering-bin");
        std::fs::write(&source_path, source).unwrap();
        let status = std::process::Command::new("rustc")
            .arg(&source_path)
            .arg("-o")
            .arg(&bin_path)
            .status()
            .unwrap();
        assert!(status.success());
        let status = std::process::Command::new(&bin_path).status().unwrap();
        assert!(status.success());

        let _ = std::fs::remove_dir_all(&base);
    }

    fn test_dir(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!("{prefix}-{}", std::process::id()))
    }
}
