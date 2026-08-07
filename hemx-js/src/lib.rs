pub const RUNTIME_ABI_VERSION: u32 = 1;
pub const RUNTIME_JS: &str = include_str!("../runtime/hemx.js");
pub const RUNTIME_D_TS: &str = include_str!("../runtime/hemx.d.ts");

include!(concat!(env!("OUT_DIR"), "/runtime_asset.rs"));
