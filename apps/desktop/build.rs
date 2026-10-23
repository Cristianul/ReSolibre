/// Compiles built-in controls, retaining debug introspection for local UI tests only.
fn main() {
    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent-dark".into())
        .with_debug_info(std::env::var("PROFILE").is_ok_and(|profile| profile == "debug"));
    slint_build::compile_with_config("ui/player.slint", config)
        .expect("compile built-in desktop UI");
}
