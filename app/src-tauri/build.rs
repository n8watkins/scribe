fn main() {
    // The integration tests (tests/*.rs) build Tauri mock apps, which load
    // Common Controls v6 entry points; without the app manifest a Windows
    // test executable fails to start (STATUS_ENTRYPOINT_NOT_FOUND).
    // tauri-build embeds the manifest into the app binary only, so embed the
    // same one into the integration tests. The shipped binary is unchanged.
    let windows_msvc = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if windows_msvc {
        let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("tests")
            .join("windows-test-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg-tests=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-tests=/MANIFESTINPUT:{}",
            manifest.display()
        );
    }
    tauri_build::build()
}
