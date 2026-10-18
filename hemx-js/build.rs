use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let mut runtime = String::from("(() => {\n");
    for path in [
        "runtime/native.js",
        "runtime/resources.js",
        "runtime/morph.js",
        "runtime/forms.js",
        "runtime/hemx.js",
    ] {
        println!("cargo:rerun-if-changed={path}");
        runtime
            .push_str(&fs::read_to_string(path).unwrap_or_else(|error| panic!("{path}: {error}")));
        runtime.push('\n');
    }
    runtime.push_str("})();\n");
    let hash = format!("{:x}", Sha256::digest(runtime.as_bytes()));
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR");
    fs::write(Path::new(&out_dir).join("hemx.js"), runtime).expect("write bundled runtime");
    fs::write(
        Path::new(&out_dir).join("runtime_asset.rs"),
        format!(
            "pub const RUNTIME_JS_HASH: &str = \"{hash}\";\n\
             pub const RUNTIME_JS_PATH: &str = \"/hemx.{hash}.js\";\n"
        ),
    )
    .expect("write runtime asset constants");
}
