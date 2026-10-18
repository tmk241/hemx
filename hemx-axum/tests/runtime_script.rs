//! The runtime script path can be interpolated into `<script src>`.

use hemplate::Hemplate;
use hemx_axum::{runtime_js_script_src, RuntimeScript};

#[derive(Hemplate)]
struct RuntimeScriptPage {
    runtime: RuntimeScript,
}

#[test]
fn the_runtime_script_is_a_typed_value_a_script_src_accepts() {
    let html = RuntimeScriptPage {
        runtime: RuntimeScript,
    }
    .render()
    .unwrap();
    assert_eq!(
        html.trim(),
        format!(
            r#"<script src="{}" defer></script>"#,
            runtime_js_script_src()
        )
    );
    assert!(html.contains("/hemx."), "content-addressed path: {html}");
}
