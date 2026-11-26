//! Tells the portals which app Scribe is. Sandboxed apps (Flatpak, Snap)
//! carry an app id the portals read themselves; a host app has to register
//! one through `org.freedesktop.host.portal.Registry` (xdg-desktop-portal
//! 1.19 and later) before its first portal call, or the portals can't store
//! permissions and shortcuts for it. Older portals lack the registry and
//! derive an id from the process; Scribe carries on either way.

use std::{sync::OnceLock, time::Duration};

static APP_ID: OnceLock<String> = OnceLock::new();
static REGISTERED: OnceLock<()> = OnceLock::new();

/// Scribe's app id (the Tauri identifier, such as `com.natkins.scribe`).
/// Set once at startup, before any portal is used.
pub fn set_app_id(app_id: &str) {
    let _ = APP_ID.set(app_id.to_string());
}

/// Registers the app id with the portals, once per process. Every portal
/// client calls this before its first request.
pub fn ensure_registered() {
    REGISTERED.get_or_init(|| {
        let Some(app_id) = APP_ID.get() else {
            return;
        };
        let app_id = match ashpd::AppID::try_from(app_id.as_str()) {
            Ok(app_id) => app_id,
            Err(error) => {
                log::warn!("Not registering {app_id} with the portals: {error}");
                return;
            }
        };
        let registered = async_io::block_on(super::portal::with_timeout(
            Duration::from_secs(3),
            ashpd::register_host_app(app_id.clone()),
        ));
        match registered {
            Some(Ok(())) => log::info!("Registered {app_id} with the desktop portals"),
            Some(Err(error)) => log::info!(
                "The desktop portals have no app registry ({error}); they identify Scribe on their own"
            ),
            None => log::warn!("Registering with the desktop portals timed out"),
        }
    });
}
