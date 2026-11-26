fn main() {
    println!("cargo:rerun-if-changed=ui");
    // One widget style on every platform, so Windows and Linux pictures match.
    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    slint_build::compile_with_config("ui/trial.slint", config).expect("compile ui/trial.slint");
}
