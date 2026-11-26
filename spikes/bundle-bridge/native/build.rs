// Embeds the same version resource fields the Tauri build gives Scribe.exe
// (tauri-build reads them from tauri.conf.json): product Scribe, company
// n8builds.dev, and the package version as file and product version.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set("ProductName", "Scribe")
        .set("FileDescription", "Scribe")
        .set("CompanyName", "n8builds.dev")
        .set("InternalName", "Scribe")
        .set("OriginalFilename", "Scribe.exe")
        .set("LegalCopyright", "n8builds.dev");
    res.set_icon("../../../app/src-tauri/icons/icon.ico");
    res.compile().expect("failed to compile the Windows version resource");
}
