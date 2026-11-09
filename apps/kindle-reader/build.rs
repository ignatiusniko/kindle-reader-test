fn main() {
    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    slint_build::compile_with_config("../../ui/app.slint", config).expect("Slint UI compiles");

    // The Kindle backend needs an explicitly bundled font (no system fonts).
    if std::env::var_os("CARGO_FEATURE_KINDLE").is_some() {
        let font = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/fonts/ReaderFont.ttf");
        println!("cargo:rerun-if-changed={}", font.display());
        if !font.exists() {
            panic!(
                "feature `kindle` needs a redistributable font at assets/fonts/ReaderFont.ttf \
                 (see assets/fonts/README.md or run scripts/fetch-font.sh)"
            );
        }
    }
}
