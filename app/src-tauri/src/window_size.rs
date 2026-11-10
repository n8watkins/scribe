//! The main window's size at launch: the size the user saved (Appearance >
//! Window size), or the shipped default, shrunk to fit the screen it opens on.
//!
//! Sizes are kept in logical pixels so a size chosen on a 150% display looks
//! the same on a 100% one. Sizes saved by older versions are physical pixels
//! and are restored as such until the user saves again.

use tauri::{LogicalSize, Manager, PhysicalSize, WebviewWindow};

use crate::settings::AppSettings;

/// The presets in Appearance > Window size, largest first. Keep in step with
/// WINDOW_PRESETS in src/views/DataPrivacy.tsx.
pub const PRESETS: [(f64, f64); 3] = [(1240.0, 820.0), (1024.0, 700.0), COMPACT];
/// The Compact preset.
pub const COMPACT: (f64, f64) = (900.0, 620.0);
/// The smallest window Scribe allows (tauri.conf.json minWidth/minHeight).
pub const MINIMUM: (f64, f64) = (820.0, 540.0);
/// Room left around a window shrunk to fit, so it doesn't touch the taskbar.
const MARGIN: f64 = 24.0;

/// The logical size to open at, given the wanted size and the screen's work
/// area (both logical): the wanted size when it fits, else the largest preset
/// no bigger than it that fits, else the work area less a margin, never below
/// the minimum.
pub fn fit_to_work_area(wanted: (f64, f64), work_area: (f64, f64)) -> (f64, f64) {
    let fits = |size: (f64, f64)| size.0 <= work_area.0 && size.1 <= work_area.1;
    if fits(wanted) {
        return wanted;
    }
    if let Some(preset) = PRESETS
        .into_iter()
        .find(|preset| preset.0 <= wanted.0 && preset.1 <= wanted.1 && fits(*preset))
    {
        return preset;
    }
    (
        wanted.0.min(work_area.0 - MARGIN).max(MINIMUM.0),
        wanted.1.min(work_area.1 - MARGIN).max(MINIMUM.1),
    )
}

/// Sizes the main window at launch: the saved size if there is one, then
/// shrunk to the work area of the monitor it opens on.
pub fn restore(window: &WebviewWindow, settings: &AppSettings) {
    let scale = window.scale_factor().unwrap_or(1.0);
    let saved = match (settings.window_width, settings.window_height) {
        (Some(width), Some(height)) if width > 0 && height > 0 => {
            let (width, height) = (f64::from(width), f64::from(height));
            Some(if settings.window_size_logical {
                (width, height)
            } else {
                (width / scale, height / scale)
            })
        }
        _ => None,
    };
    let wanted = match saved {
        Some(size) => size,
        None => match window.inner_size() {
            Ok(size) => {
                let logical = size.to_logical::<f64>(scale);
                (logical.width, logical.height)
            }
            Err(_) => return,
        },
    };

    let work_area = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten())
        .map(|monitor| {
            let area = monitor.work_area().size;
            let monitor_scale = monitor.scale_factor();
            (
                f64::from(area.width) / monitor_scale,
                f64::from(area.height) / monitor_scale,
            )
        });
    let size = match work_area {
        Some(area) => fit_to_work_area(wanted, area),
        None => wanted,
    };

    if saved.is_some() || size != wanted {
        let _ = window.set_size(LogicalSize::new(size.0, size.1));
        if size != wanted {
            let _ = window.center();
            log::info!(
                "Opened the window at {:.0}x{:.0} to fit the screen ({:.0}x{:.0} wanted)",
                size.0,
                size.1,
                wanted.0,
                wanted.1
            );
        }
    }
}

/// The main window's current inner size in logical pixels, for saving.
pub fn current_logical(app: &tauri::AppHandle) -> Option<(u32, u32)> {
    let window = app.get_webview_window("main")?;
    let scale = window.scale_factor().ok()?;
    let size: PhysicalSize<u32> = window.inner_size().ok()?;
    let logical = size.to_logical::<f64>(scale);
    Some((logical.width.round() as u32, logical.height.round() as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_a_size_that_fits() {
        assert_eq!(
            fit_to_work_area((1024.0, 700.0), (1366.0, 728.0)),
            (1024.0, 700.0)
        );
    }

    #[test]
    fn drops_to_compact_on_a_short_screen() {
        // 1920x1080 at 150% leaves about 1280x672 above the taskbar.
        assert_eq!(fit_to_work_area((1024.0, 700.0), (1280.0, 672.0)), COMPACT);
    }

    #[test]
    fn shrinks_to_the_work_area_when_compact_does_not_fit() {
        assert_eq!(
            fit_to_work_area((1024.0, 700.0), (1000.0, 600.0)),
            (976.0, 576.0)
        );
    }

    #[test]
    fn a_large_window_steps_down_to_the_largest_preset_that_fits() {
        assert_eq!(
            fit_to_work_area((1240.0, 820.0), (1366.0, 728.0)),
            (1024.0, 700.0)
        );
    }

    #[test]
    fn never_goes_below_the_minimum() {
        assert_eq!(fit_to_work_area((1024.0, 700.0), (700.0, 500.0)), MINIMUM);
    }

    #[test]
    fn a_saved_size_smaller_than_compact_is_kept_when_it_fits() {
        assert_eq!(
            fit_to_work_area((860.0, 580.0), (1280.0, 672.0)),
            (860.0, 580.0)
        );
    }
}
