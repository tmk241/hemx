use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let runtime_path = Path::new("runtime/hemx.js");
    println!("cargo:rerun-if-changed={}", runtime_path.display());

    let runtime = fs::read(runtime_path).expect("read hemx runtime");
    let hash = format!("{:x}", Sha256::digest(&runtime));
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR");
    fs::write(
        Path::new(&out_dir).join("runtime_asset.rs"),
        format!(
            "pub const RUNTIME_JS_HASH: &str = \"{hash}\";\n\
             pub const RUNTIME_JS_PATH: &str = \"/hemx.{hash}.js\";\n"
        ),
    )
    .expect("write runtime asset constants");
}
