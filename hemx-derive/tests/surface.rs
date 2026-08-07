use hemx_derive::{component, surface};

#[component]
mod local_component {
    pub const EXISTING: u8 = 1;
}

#[surface]
mod ui {
    pub const EXISTING: u8 = 1;
}

#[test]
fn surface_macro_preserves_inline_module_with_generated_file() {
    assert_eq!(ui::EXISTING, 1);
}

#[test]
fn component_macro_preserves_inline_module_without_generated_symbols() {
    assert_eq!(local_component::EXISTING, 1);
}
