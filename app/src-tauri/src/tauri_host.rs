//! The Tauri side of the engine's seams: the folders Tauri resolves, checked
//! against the engine's own [`Paths`].

use scribe_core::paths::Paths;
use tauri::{Manager, Runtime};

/// Resolves the engine's folders for this app (its identifier and product
/// name), the same folders `app.path()` gives.
pub fn resolve_paths<R: Runtime, M: Manager<R>>(
    app: &M,
) -> Result<Paths, scribe_core::paths::UnknownFolder> {
    Paths::resolve(&app.config().identifier, &app.package_info().name)
}

/// Every folder where `paths` and Tauri's resolver disagree, as
/// "folder: engine vs Tauri" lines. Empty when they agree.
pub fn paths_mismatches<R: Runtime, M: Manager<R>>(app: &M, paths: &Paths) -> Vec<String> {
    let resolver = app.path();
    let folders = [
        (
            "data",
            Some(paths.data_dir().to_path_buf()),
            resolver.app_data_dir().ok(),
        ),
        (
            "cache",
            Some(paths.cache_dir().to_path_buf()),
            resolver.app_cache_dir().ok(),
        ),
        (
            "log",
            Some(paths.log_dir().to_path_buf()),
            resolver.app_log_dir().ok(),
        ),
        (
            "resource",
            paths.resource_dir().map(|dir| dir.to_path_buf()),
            resolver.resource_dir().ok(),
        ),
        (
            "home",
            paths.home_dir().map(|dir| dir.to_path_buf()),
            resolver.home_dir().ok(),
        ),
    ];
    folders
        .into_iter()
        .filter(|(_, ours, tauris)| ours != tauris)
        .map(|(name, ours, tauris)| format!("{name}: {ours:?} vs {tauris:?}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::test::{mock_builder, mock_context, noop_assets};

    fn config(source: &str) -> serde_json::Value {
        serde_json::from_str(source).unwrap()
    }

    /// A mock app carrying a flavour's identifier and product name.
    fn app_for(identifier: &str, product_name: &str) -> tauri::App<tauri::test::MockRuntime> {
        let mut context = mock_context(noop_assets());
        context.config_mut().identifier = identifier.to_string();
        context.config_mut().product_name = Some(product_name.to_string());
        context.package_info_mut().name = product_name.to_string();
        mock_builder().build(context).unwrap()
    }

    /// The identifier and product name of stable Scribe and of Scribe Dev,
    /// read from the configs the installers are built from.
    fn flavours() -> Vec<(String, String)> {
        let stable = config(include_str!("../tauri.conf.json"));
        let dev = config(include_str!("../tauri.dev-flavor.conf.json"));
        [stable, dev]
            .iter()
            .map(|config| {
                (
                    config["identifier"].as_str().unwrap().to_string(),
                    config["productName"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    }

    #[test]
    fn the_engine_resolves_every_folder_tauri_does() {
        let flavours = flavours();
        assert_eq!(flavours.len(), 2);
        assert_ne!(flavours[0].0, flavours[1].0);
        for (identifier, product_name) in flavours {
            let app = app_for(&identifier, &product_name);
            let paths = resolve_paths(&app).unwrap();
            assert_eq!(
                paths_mismatches(&app, &paths),
                Vec::<String>::new(),
                "{identifier}"
            );
            assert!(paths.data_dir().ends_with(&identifier));
        }
    }

    /// Tauri can move the app folders through `app > appDirectoriesOverride`;
    /// the engine's `Paths` does not follow it, so no config may set it.
    #[test]
    fn no_config_overrides_the_app_folders() {
        for source in [
            include_str!("../tauri.conf.json"),
            include_str!("../tauri.dev-flavor.conf.json"),
            include_str!("../tauri.e2e.conf.json"),
            include_str!("../tauri.linux.conf.json"),
            include_str!("../tauri.macos.conf.json"),
            include_str!("../tauri.unsigned-build.conf.json"),
        ] {
            assert!(
                !source.contains("appDirectoriesOverride"),
                "a Tauri config overrides the app folders"
            );
        }
    }
}
