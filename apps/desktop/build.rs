/// Compiles controls with introspection for debug or explicitly requested optimized UI tests.
fn main() {
    let debug_profile = std::env::var("PROFILE").is_ok_and(|profile| profile == "debug");
    let inspect_controls = std::env::var("SLINT_EMIT_DEBUG_INFO").is_ok_and(|value| value == "1");
    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent-dark".into())
        .with_debug_info(debug_profile || inspect_controls);
    slint_build::compile_with_config("ui/player.slint", config)
        .expect("compile built-in desktop UI");
}
