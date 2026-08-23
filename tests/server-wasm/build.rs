fn main() {
    hemx_build::app()
        .template_dir("templates")
        .run()
        .expect("compile server-side Wasm surfaces");
}
