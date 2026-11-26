//! Where the pill goes, copied from PillApp.tsx: a saved position when it is on
//! some monitor, otherwise centred near the bottom of the fallback monitor.
//! All numbers are physical pixels, as in the React code.

pub const BOTTOM_MARGIN_PX: i32 = 90;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// `positionOnScreen`: true when the point is inside a monitor (or none are known).
pub fn on_screen(monitors: &[Rect], x: i32, y: i32) -> bool {
    monitors.is_empty()
        || monitors
            .iter()
            .any(|m| x >= m.x && y >= m.y && x < m.x + m.w && y < m.y + m.h)
}

/// The first placement: the saved point if it is on a monitor, else centred
/// horizontally on `fallback`, `BOTTOM_MARGIN_PX` above its bottom edge.
pub fn first_position(
    monitors: &[Rect],
    saved: Option<(i32, i32)>,
    fallback: Rect,
    size: (i32, i32),
) -> (i32, i32) {
    if let Some((x, y)) = saved
        && on_screen(monitors, x, y)
    {
        return (x, y);
    }
    (
        fallback.x + (fallback.w - size.0) / 2,
        fallback.y + fallback.h - size.1 - BOTTOM_MARGIN_PX,
    )
}

/// Keeps the bottom edge where it was when the height changes (grows upward).
pub fn anchored_y(y: i32, old_h: i32, new_h: i32) -> i32 {
    y + (old_h - new_h)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRIMARY: Rect = Rect {
        x: 0,
        y: 0,
        w: 2560,
        h: 1440,
    };
    const SECOND: Rect = Rect {
        x: 2560,
        y: -200,
        w: 1920,
        h: 1080,
    };

    #[test]
    fn saved_position_on_the_second_monitor_is_kept() {
        let p = first_position(&[PRIMARY, SECOND], Some((3000, 500)), PRIMARY, (237, 84));
        assert_eq!(p, (3000, 500));
    }

    #[test]
    fn windows_minimise_sentinel_falls_back_to_the_bottom_centre() {
        let p = first_position(
            &[PRIMARY, SECOND],
            Some((-32000, -32000)),
            PRIMARY,
            (237, 84),
        );
        assert_eq!(p, ((2560 - 237) / 2, 1440 - 84 - 90));
    }

    #[test]
    fn unplugged_monitor_falls_back() {
        let p = first_position(&[PRIMARY], Some((3000, 500)), PRIMARY, (100, 50));
        assert_eq!(p, (1230, 1300));
    }

    #[test]
    fn growing_keeps_the_bottom_edge() {
        assert_eq!(anchored_y(1000, 120, 225), 895);
        assert_eq!(anchored_y(895, 225, 120), 1000);
    }
}
