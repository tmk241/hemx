use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn component_macro_reports_missing_handler_implementation() {
    let fixture = Fixture::new("hemx-derive-component-missing-handler-fail");
    fixture.write(
        "Cargo.toml",
        &format!(
            r#"[package]
name = "hemx-derive-component-missing-handler-fail"
version = "0.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[dependencies]
hemx = {{ path = {:?} }}
"#,
            repo_path("hemx")
        ),
    );
    fixture.write(
        "build.rs",
        r#"fn main() {
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(
        out.join("hemx.syms"),
        "hemx-syms-v1\nhandle\ttemplates/app.heml::create\tcreate\t1\nhandle\ttemplates/app.heml::delete\tdelete\t2\n",
    )
    .unwrap();
}
"#,
    );
    fixture.write(
        "src/lib.rs",
        r#"#[hemx::component]
mod todos {
    #[hemx::handler]
    fn create() -> impl hemx::IntoEffect {
        hemx::advanced::EffectBatch::default()
    }
}
"#,
    );

    let output = check_fixture(&fixture);

    assert!(!output.status.success(), "fixture unexpectedly compiled");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("#[hemx::component] missing handler implementation(s): delete"),
        "missing component diagnostic in stderr:\n{stderr}"
    );
}

#[test]
fn component_macro_can_validate_one_generated_component() {
    let fixture = Fixture::new("hemx-derive-component-scoped-pass");
    fixture.write(
        "Cargo.toml",
        &format!(
            r#"[package]
name = "hemx-derive-component-scoped-pass"
version = "0.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[dependencies]
hemx = {{ path = {:?} }}
"#,
            repo_path("hemx")
        ),
    );
    fixture.write(
        "build.rs",
        r#"fn main() {
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(
        out.join("hemx.syms"),
        "hemx-syms-v1\nhandle\ttemplates/todos.heml::create\tcreate\t1\nhandle\ttemplates/admin.heml::delete\tdelete\t2\n",
    )
    .unwrap();
}
"#,
    );
    fixture.write(
        "src/lib.rs",
        r#"#[hemx::component("todos")]
mod todos {
    #[hemx::handler]
    fn create() -> impl hemx::IntoEffect {
        hemx::EventName::new("created").emit("")
    }
}
"#,
    );

    let output = check_fixture(&fixture);

    assert!(
        output.status.success(),
        "fixture failed to compile:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn component_macro_rejects_handlers_outside_scoped_component() {
    let fixture = Fixture::new("hemx-derive-component-extra-handler-fail");
    fixture.write(
        "Cargo.toml",
        &format!(
            r#"[package]
name = "hemx-derive-component-extra-handler-fail"
version = "0.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[dependencies]
hemx = {{ path = {:?} }}
"#,
            repo_path("hemx")
        ),
    );
    fixture.write(
        "build.rs",
        r#"fn main() {
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(
        out.join("hemx.syms"),
        "hemx-syms-v1\nhandle\ttemplates/todos.heml::create\tcreate\t1\nhandle\ttemplates/admin.heml::delete\tdelete\t2\n",
    )
    .unwrap();
}
"#,
    );
    fixture.write(
        "src/lib.rs",
        r#"#[hemx::component("todos")]
mod handlers {
    #[hemx::handler]
    fn create() -> impl hemx::IntoEffect {
        hemx::EventName::new("created").emit("")
    }

    #[hemx::handler]
    fn delete() -> impl hemx::IntoEffect {
        hemx::EventName::new("deleted").emit("")
    }
}
"#,
    );

    let output = check_fixture(&fixture);

    assert!(!output.status.success(), "fixture unexpectedly compiled");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("handler(s) not declared by this component's generated handles: delete"),
        "missing scoped extra-handler diagnostic in stderr:\n{stderr}"
    );
}

#[test]
fn component_macro_rejects_unknown_scoped_component() {
    let fixture = Fixture::new("hemx-derive-component-unknown-scope-fail");
    fixture.write(
        "Cargo.toml",
        &format!(
            r#"[package]
name = "hemx-derive-component-unknown-scope-fail"
version = "0.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[dependencies]
hemx = {{ path = {:?} }}
"#,
            repo_path("hemx")
        ),
    );
    fixture.write(
        "build.rs",
        r#"fn main() {
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(
        out.join("hemx.syms"),
        "hemx-syms-v1\nhandle\ttemplates/admin.heml::create\tcreate\t1\n",
    )
    .unwrap();
}
"#,
    );
    fixture.write(
        "src/lib.rs",
        r#"#[hemx::component("todos")]
mod handlers {
    #[hemx::handler]
    fn create() -> impl hemx::IntoEffect {
        hemx::EventName::new("created").emit("")
    }
}
"#,
    );

    let output = check_fixture(&fixture);

    assert!(!output.status.success(), "fixture unexpectedly compiled");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("#[hemx::component(\"todos\")] does not match any generated handles"),
        "missing unknown component diagnostic in stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("available generated components: admin"),
        "unknown scope diagnostic must name the valid repair choice:\n{stderr}"
    );
}

#[test]
fn component_macro_rejects_ambiguous_generated_handles() {
    let fixture = Fixture::new("hemx-derive-component-ambiguous-handle-fail");
    fixture.write(
        "Cargo.toml",
        &format!(
            r#"[package]
name = "hemx-derive-component-ambiguous-handle-fail"
version = "0.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[dependencies]
hemx = {{ path = {:?} }}
"#,
            repo_path("hemx")
        ),
    );
    fixture.write(
        "build.rs",
        r#"fn main() {
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(
        out.join("hemx.syms"),
        "hemx-syms-v1\nhandle\ttemplates/todos.heml::save\tsave\t1\nhandle\ttemplates/todos.heml::section::save\tsave\t2\n",
    )
    .unwrap();
}
"#,
    );
    fixture.write(
        "src/lib.rs",
        r#"#[hemx::component("todos")]
mod handlers {
    #[hemx::handler]
    fn save() -> impl hemx::IntoEffect {
        hemx::EventName::new("saved").emit("")
    }
}
"#,
    );

    let output = check_fixture(&fixture);

    assert!(!output.status.success(), "fixture unexpectedly compiled");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("ambiguous generated handle name(s): save"),
        "missing ambiguous-handle diagnostic in stderr:\n{stderr}"
    );
}

#[test]
fn handler_macro_reports_unknown_handle_and_bad_shape() {
    let fixture = Fixture::new("hemx-derive-handler-fail");
    fixture.write(
        "Cargo.toml",
        &format!(
            r#"[package]
name = "hemx-derive-handler-fail"
version = "0.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[dependencies]
hemx = {{ path = {:?} }}
"#,
            repo_path("hemx")
        ),
    );
    fixture.write(
        "build.rs",
        r#"fn main() {
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(
        out.join("hemx.syms"),
        "hemx-syms-v1\nhandle\ttemplates/app.heml::known\tknown\t1\n",
    )
    .unwrap();
}
"#,
    );
    fixture.write(
        "src/lib.rs",
        r#"#[hemx::handler]
fn missing() -> impl hemx::IntoEffect {
    hemx::advanced::EffectBatch::default()
}

#[hemx::handler]
fn known() {}
"#,
    );

    let output = check_fixture(&fixture);

    assert!(!output.status.success(), "fixture unexpectedly compiled");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown hemx handle `missing`; add `data-hemx-handle=\"missing\"`"),
        "missing unknown-handle diagnostic in stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("hemx handler `known` must accept a form/context parameter or return a value implementing IntoEffect"),
        "missing handler-shape diagnostic in stderr:\n{stderr}"
    );
}

#[test]
fn handler_macro_reports_missing_syms() {
    let fixture = Fixture::new("hemx-derive-handler-syms-fail");
    fixture.write(
        "Cargo.toml",
        &format!(
            r#"[package]
name = "hemx-derive-handler-syms-fail"
version = "0.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[dependencies]
hemx = {{ path = {:?} }}
"#,
            repo_path("hemx")
        ),
    );
    fixture.write(
        "build.rs",
        r#"fn main() {
    println!("cargo:rerun-if-changed=build.rs");
}
"#,
    );
    fixture.write(
        "src/lib.rs",
        r#"#[hemx::handler]
fn create() -> impl hemx::IntoEffect {
    hemx::advanced::EffectBatch::default()
}
"#,
    );

    let output = check_fixture(&fixture);

    assert!(!output.status.success(), "fixture unexpectedly compiled");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("#[hemx::handler] could not find"),
        "missing handler syms diagnostic in stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("add hemx_build::app().run()? to build.rs"),
        "missing build.rs hint in stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("OUT_DIR"),
        "diagnostic should not teach Cargo internals:\n{stderr}"
    );
}

#[test]
fn handler_requires_generated_param_arguments() {
    let fixture = Fixture::new("hemx-derive-param-handler-fail");
    fixture.write(
        "Cargo.toml",
        &format!(
            r#"[package]
name = "hemx-derive-param-handler-fail"
version = "0.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[dependencies]
hemx = {{ path = {:?} }}
"#,
            repo_path("hemx")
        ),
    );
    fixture.write(
        "build.rs",
        r#"fn main() {
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(
        out.join("hemx.syms"),
        "hemx-syms-v1\nhandle\ttemplates/app.heml::show\tshow\t1\nhandle_param\tshow\ttodo_id\nhandle_param\tshow\tmode\n",
    )
    .unwrap();
}
"#,
    );
    fixture.write(
        "src/lib.rs",
        r#"#[hemx::handler]
fn show(todo_id: String) -> impl hemx::IntoEffect {
    hemx::advanced::EffectBatch::default()
}
"#,
    );

    let output = check_fixture(&fixture);

    assert!(!output.status.success(), "fixture unexpectedly compiled");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("hemx handler `show` is missing generated param argument(s): mode"),
        "missing param diagnostic in stderr:\n{stderr}"
    );
}

#[test]
fn form_struct_rejects_missing_generated_field() {
    let fixture = Fixture::new("hemx-derive-form-struct-missing-field-fail");
    fixture.write(
        "Cargo.toml",
        &format!(
            r#"[package]
name = "hemx-derive-form-struct-missing-field-fail"
version = "0.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[dependencies]
hemx = {{ path = {:?} }}
"#,
            repo_path("hemx")
        ),
    );
    fixture.write(
        "build.rs",
        r#"fn main() {
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(
        out.join("hemx.syms"),
        "hemx-syms-v1\nform\ttemplates/app.heml::new_todo\tnew_todo\t1\nform_field\tnew_todo\ttitle\ttrue\tfalse\n",
    )
    .unwrap();
}
"#,
    );
    fixture.write(
        "src/lib.rs",
        r#"#[hemx::form("new_todo")]
struct CreateTodo {}
"#,
    );

    let output = check_fixture(&fixture);

    assert!(!output.status.success(), "fixture unexpectedly compiled");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("hemx form `new_todo` is missing field `title` for form control `title`"),
        "missing form field diagnostic in stderr:\n{stderr}"
    );
}

#[test]
fn form_struct_rejects_wrong_optionality_and_multiplicity() {
    let fixture = Fixture::new("hemx-derive-form-struct-shape-fail");
    fixture.write(
        "Cargo.toml",
        &format!(
            r#"[package]
name = "hemx-derive-form-struct-shape-fail"
version = "0.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[dependencies]
hemx = {{ path = {:?} }}
"#,
            repo_path("hemx")
        ),
    );
    fixture.write(
        "build.rs",
        r#"fn main() {
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(
        out.join("hemx.syms"),
        "hemx-syms-v1\nform\ttemplates/app.heml::profile\tprofile\t1\nform_field\tprofile\ttitle\ttrue\tfalse\nform_field\tprofile\tlabels\tfalse\ttrue\n",
    )
    .unwrap();
}
"#,
    );
    fixture.write(
        "src/lib.rs",
        r#"#[hemx::form("profile")]
struct Profile {
    title: Option<String>,
    labels: Option<String>,
}
"#,
    );

    let output = check_fixture(&fixture);

    assert!(!output.status.success(), "fixture unexpectedly compiled");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("field `title` is required in HTML and must not be Option<_>"),
        "missing required-field diagnostic in stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("field `labels` accepts multiple values and must be Vec<_>"),
        "missing multiplicity diagnostic in stderr:\n{stderr}"
    );
}

#[test]
fn form_struct_requires_field_parser() {
    let fixture = Fixture::new("hemx-derive-form-struct-parser-fail");
    fixture.write(
        "Cargo.toml",
        &format!(
            r#"[package]
name = "hemx-derive-form-struct-parser-fail"
version = "0.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[dependencies]
hemx = {{ path = {:?} }}
"#,
            repo_path("hemx")
        ),
    );
    fixture.write(
        "build.rs",
        r#"fn main() {
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(
        out.join("hemx.syms"),
        "hemx-syms-v1\nform\ttemplates/app.heml::profile\tprofile\t1\nform_field\tprofile\temail\ttrue\tfalse\n",
    )
    .unwrap();
}
"#,
    );
    fixture.write(
        "src/lib.rs",
        r#"struct Email;

#[hemx::form("profile")]
struct Profile {
    email: Email,
}
"#,
    );

    let output = check_fixture(&fixture);

    assert!(!output.status.success(), "fixture unexpectedly compiled");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Email: FormValue") || stderr.contains("Email: hemx::FormValue"),
        "missing parser availability diagnostic in stderr:\n{stderr}"
    );
}

#[test]
fn form_struct_accepts_raw_identifier_for_reserved_control_name() {
    let fixture = Fixture::new("hemx-derive-form-raw-identifier-pass");
    fixture.write(
        "Cargo.toml",
        &format!(
            r#"[package]
name = "hemx-derive-form-raw-identifier-pass"
version = "0.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[dependencies]
hemx = {{ path = {:?} }}
"#,
            repo_path("hemx")
        ),
    );
    fixture.write(
        "build.rs",
        r#"fn main() {
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(
        out.join("hemx.syms"),
        "hemx-syms-v1\nform\ttemplates/app.heml::filter\tfilter\t1\nform_field\tfilter\ttype\ttrue\tfalse\n",
    )
    .unwrap();
}
"#,
    );
    fixture.write(
        "src/lib.rs",
        r#"#[hemx::form("filter")]
struct Filter {
    r#type: String,
}
"#,
    );

    let output = check_fixture(&fixture);

    assert!(
        output.status.success(),
        "fixture failed to compile:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn form_handle_accepts_checked_form_model() {
    let fixture = Fixture::new("hemx-derive-form-handler-checked-model-pass");
    fixture.write(
        "Cargo.toml",
        &format!(
            r#"[package]
name = "hemx-derive-form-handler-checked-model-pass"
version = "0.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[dependencies]
hemx = {{ path = {:?} }}
"#,
            repo_path("hemx")
        ),
    );
    fixture.write(
        "build.rs",
        r#"fn main() {
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(
        out.join("hemx.syms"),
        "hemx-syms-v1\nhandle\ttemplates/app.heml::create\tcreate\t1\nhandle_form\tcreate\tnew_todo\nform\ttemplates/app.heml::new_todo\tnew_todo\t1\nform_field\tnew_todo\ttitle\ttrue\tfalse\n",
    )
    .unwrap();
}
"#,
    );
    fixture.write(
        "src/lib.rs",
        r#"#[hemx::form("new_todo")]
struct CreateTodo {
    title: String,
}

#[hemx::handler]
fn create(_form: hemx::Form<CreateTodo>) -> impl hemx::IntoEffect {
    hemx::EventName::new("created").emit("")
}

fn smoke() {
    let _ = create(CreateTodo::FORM);
}
"#,
    );

    let output = check_fixture(&fixture);

    assert!(
        output.status.success(),
        "fixture failed to compile:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn form_handle_requires_form_parameter() {
    let fixture = Fixture::new("hemx-derive-form-handler-fail");
    fixture.write(
        "Cargo.toml",
        &format!(
            r#"[package]
name = "hemx-derive-form-handler-fail"
version = "0.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[dependencies]
hemx = {{ path = {:?} }}
"#,
            repo_path("hemx")
        ),
    );
    fixture.write(
        "build.rs",
        r#"fn main() {
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(
        out.join("hemx.syms"),
        "hemx-syms-v1\nhandle\ttemplates/app.heml::create\tcreate\t1\nhandle_form\tcreate\tnew_todo\n",
    )
    .unwrap();
}
"#,
    );
    fixture.write(
        "src/lib.rs",
        r#"#[hemx::handler]
fn create() -> impl hemx::IntoEffect {
    hemx::advanced::EffectBatch::default()
}
"#,
    );

    let output = check_fixture(&fixture);

    assert!(!output.status.success(), "fixture unexpectedly compiled");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(
            "hemx handler `create` handles a generated form and must accept a typed form argument"
        ),
        "missing form-handler diagnostic in stderr:\n{stderr}"
    );
}

#[test]
fn form_handle_rejects_state_only_handler() {
    let fixture = Fixture::new("hemx-derive-form-handler-state-only-fail");
    fixture.write(
        "Cargo.toml",
        &format!(
            r#"[package]
name = "hemx-derive-form-handler-state-only-fail"
version = "0.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[dependencies]
hemx = {{ path = {:?} }}
"#,
            repo_path("hemx")
        ),
    );
    fixture.write(
        "build.rs",
        r#"fn main() {
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(
        out.join("hemx.syms"),
        "hemx-syms-v1\nhandle\ttemplates/app.heml::create\tcreate\t1\nhandle_form\tcreate\tnew_todo\n",
    )
    .unwrap();
}
"#,
    );
    fixture.write(
        "src/lib.rs",
        r#"struct App;
struct State<T>(T);

#[hemx::handler]
fn create(_state: State<App>) -> impl hemx::IntoEffect {
    hemx::advanced::EffectBatch::default()
}
"#,
    );

    let output = check_fixture(&fixture);

    assert!(!output.status.success(), "fixture unexpectedly compiled");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(
            "hemx handler `create` handles a generated form and must accept a typed form argument"
        ),
        "missing state-only form-handler diagnostic in stderr:\n{stderr}"
    );
}

#[test]
fn form_handle_still_requires_generated_param_arguments() {
    let fixture = Fixture::new("hemx-derive-form-handler-param-fail");
    fixture.write(
        "Cargo.toml",
        &format!(
            r#"[package]
name = "hemx-derive-form-handler-param-fail"
version = "0.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[dependencies]
hemx = {{ path = {:?} }}
"#,
            repo_path("hemx")
        ),
    );
    fixture.write(
        "build.rs",
        r#"fn main() {
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(
        out.join("hemx.syms"),
        "hemx-syms-v1\nhandle\ttemplates/app.heml::create\tcreate\t1\nhandle_form\tcreate\tnew_todo\nhandle_param\tcreate\ttodo_id\n",
    )
    .unwrap();
}
"#,
    );
    fixture.write(
        "src/lib.rs",
        r#"struct CreateTodo;

#[hemx::handler]
fn create(_form: hemx::Form<CreateTodo>) -> impl hemx::IntoEffect {
    hemx::advanced::EffectBatch::default()
}
"#,
    );

    let output = check_fixture(&fixture);

    assert!(!output.status.success(), "fixture unexpectedly compiled");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("hemx handler `create` is missing generated param argument(s): todo_id"),
        "missing form-param diagnostic in stderr:\n{stderr}"
    );
}

#[test]
fn surface_macro_reports_missing_generated_include() {
    let fixture = Fixture::new("hemx-derive-surface-include-fail");
    fixture.write(
        "Cargo.toml",
        &format!(
            r#"[package]
name = "hemx-derive-surface-include-fail"
version = "0.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[dependencies]
hemx = {{ path = {:?} }}
"#,
            repo_path("hemx")
        ),
    );
    fixture.write(
        "build.rs",
        r#"fn main() {
    println!("cargo:rerun-if-changed=build.rs");
}
"#,
    );
    fixture.write(
        "src/lib.rs",
        r#"#[hemx::surface]
pub mod ui {}
"#,
    );

    let output = check_fixture(&fixture);

    assert!(!output.status.success(), "fixture unexpectedly compiled");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("#[hemx::surface] could not find"),
        "missing surface include diagnostic in stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("add hemx_build::app().run()? to build.rs"),
        "missing build.rs hint in stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("OUT_DIR"),
        "diagnostic should not teach Cargo internals:\n{stderr}"
    );
}

#[test]
fn surface_macro_requires_inline_module() {
    let fixture = Fixture::new("hemx-derive-surface-inline-fail");
    fixture.write(
        "Cargo.toml",
        &format!(
            r#"[package]
name = "hemx-derive-surface-inline-fail"
version = "0.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[dependencies]
hemx = {{ path = {:?} }}
"#,
            repo_path("hemx")
        ),
    );
    fixture.write(
        "src/lib.rs",
        r#"#[hemx::surface]
pub mod ui;
"#,
    );

    let output = check_fixture(&fixture);

    assert!(!output.status.success(), "fixture unexpectedly compiled");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("#[hemx::surface] must be used on an inline module"),
        "missing inline-module diagnostic in stderr:\n{stderr}"
    );
}

#[test]
fn generated_resource_references_fail_when_name_is_absent() {
    let fixture = Fixture::new("hemx-derive-generated-resource-fail");
    fixture.write(
        "Cargo.toml",
        &format!(
            r#"[package]
name = "hemx-derive-generated-resource-fail"
version = "0.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[dependencies]
hemx = {{ path = {:?} }}
"#,
            repo_path("hemx")
        ),
    );
    fixture.write(
        "build.rs",
        r#"fn main() {
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(
        out.join("hemx.generated.rs"),
        "pub mod classes { pub const card: ::hemx::CssClass = ::hemx::CssClass::new(\"card\"); }\n",
    )
    .unwrap();
}
"#,
    );
    fixture.write(
        "src/lib.rs",
        r#"#[hemx::surface]
pub mod ui {}

pub const KNOWN: hemx::CssClass = ui::classes::card;
pub const MISSING: hemx::CssClass = ui::classes::missing;
"#,
    );

    let output = check_fixture(&fixture);

    assert!(!output.status.success(), "fixture unexpectedly compiled");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot find value `missing` in module `ui::classes`"),
        "missing generated-resource diagnostic in stderr:\n{stderr}"
    );
}

#[test]
fn effect_batch_has_no_parallel_postcard_wire_api() {
    let fixture = Fixture::new("hemx-effect-batch-postcard-api-fail");
    fixture.write(
        "Cargo.toml",
        &format!(
            r#"[package]
name = "hemx-effect-batch-postcard-api-fail"
version = "0.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[dependencies]
hemx = {{ path = {:?} }}
"#,
            repo_path("hemx")
        ),
    );
    fixture.write(
        "src/lib.rs",
        r#"pub fn encode(batch: &hemx::advanced::EffectBatch) {
    let _ = batch.to_postcard();
}
"#,
    );

    let output = check_fixture(&fixture);

    assert!(!output.status.success(), "fixture unexpectedly compiled");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no method named `to_postcard`"),
        "missing sole-wire-API diagnostic in stderr:\n{stderr}"
    );
}

fn check_fixture(fixture: &Fixture) -> std::process::Output {
    Command::new("cargo")
        .arg("check")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(fixture.path.join("Cargo.toml"))
        // Cargo serializes access to a shared target directory, so all fixtures reuse
        // the same compiled hemx dependency graph instead of rebuilding it per test.
        .env("CARGO_TARGET_DIR", fixture_target_dir())
        .output()
        .expect("cargo check fixture runs")
}

fn repo_path(crate_name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate has workspace parent")
        .join(crate_name)
}

fn fixture_target_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate has workspace parent")
        .join("target/compile-fixtures")
}

struct Fixture {
    path: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "{name}-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("src")).unwrap();
        Self { path }
    }

    fn write(&self, relative: &str, contents: &str) {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
