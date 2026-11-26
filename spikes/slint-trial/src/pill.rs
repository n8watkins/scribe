//! The pill's behaviour, copied from PillApp.tsx: layouts and their window
//! sizes, show and hide, growing upward with the bottom edge anchored, the
//! first placement and its off-screen fallback, click to stop, press-and-move
//! to drag, the Copy button, and the waveform fed every 40 ms.

use crate::place::{self, Rect};
use crate::status::{js, log};
use crate::wave::{BAR_COUNT, FEED_MS, FakeMic, Wave};
use crate::{PillLayout, PillTone, PillWindow, fixtures};
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

#[cfg(windows)]
use crate::win::{self, Hwnd};

const READY_HIDE_DELAY_MS: u64 = 2000;
const CONFIRM_HIDE_DELAY_MS: u64 = 8000;
const COPIED_RESET_MS: u64 = 1500;
const DRAG_THRESHOLD_PX: f32 = 4.0;
const TEXT_MAX_HEIGHT: f32 = 150.0;
/// requestAnimationFrame stand-in for the bar easing.
const FRAME_MS: u64 = 16;
/// The fake recogniser adds a word this often.
const WORD_MS: u64 = 280;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Dot,
    Visualizer,
    VisualizerWithText,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Idle,
    Recording,
    Stopping,
    Transcribing,
    Pasting,
    Ready,
    Error,
}

impl Status {
    fn visible(self) -> bool {
        !matches!(self, Status::Idle)
    }
    fn pending(self) -> bool {
        matches!(
            self,
            Status::Stopping | Status::Transcribing | Status::Pasting
        )
    }
    fn tone(self) -> PillTone {
        match self {
            Status::Recording => PillTone::Recording,
            Status::Stopping | Status::Transcribing | Status::Pasting => PillTone::Pending,
            Status::Ready => PillTone::Ready,
            Status::Error => PillTone::Error,
            Status::Idle => PillTone::Idle,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layout {
    Dot,
    Bar,
    Text,
}

impl Layout {
    /// LAYOUT_SIZES, logical px.
    fn size(self) -> (f32, f32) {
        match self {
            Layout::Dot => (58.0, 56.0),
            Layout::Bar => (158.0, 56.0),
            Layout::Text => (320.0, 80.0),
        }
    }
    fn slint(self) -> PillLayout {
        match self {
            Layout::Dot => PillLayout::Dot,
            Layout::Bar => PillLayout::Bar,
            Layout::Text => PillLayout::Text,
        }
    }
}

/// `pillLayout()`.
pub fn layout_for(mode: Mode, status: Status, confirming: bool) -> Layout {
    let recording = status == Status::Recording;
    if mode == Mode::VisualizerWithText && (recording || confirming || status.pending()) {
        return Layout::Text;
    }
    if mode == Mode::Dot && !confirming {
        return Layout::Dot;
    }
    Layout::Bar
}

/// Every state the trial can be started in (`--pill=<name>`), for screenshots and measuring.
pub const SCENARIOS: &[&str] = &[
    "dot",
    "dot-error",
    "bar",
    "note",
    "text",
    "text-full",
    "pending",
    "pending-text",
    "ready",
    "error",
    "confirm",
    "confirm-copied",
    "confirm-text",
];

struct Press {
    cursor: (i32, i32),
    origin: (i32, i32),
    dragging: bool,
}

pub struct Pill {
    ui: PillWindow,
    #[cfg(windows)]
    hwnd: Option<Hwnd>,
    mode: Mode,
    status: Status,
    note: bool,
    confirm: Option<String>,
    copied: bool,
    partial: String,
    error: String,
    enabled: bool,
    hold: bool,
    no_clipboard: bool,
    no_trim: bool,
    wave: Wave,
    mic: FakeMic,
    bars: Rc<VecModel<f32>>,
    bars_running: bool,
    words_from_scenario: bool,
    word_index: usize,
    feed: Timer,
    anim: Timer,
    words: Timer,
    hide_timer: Timer,
    confirm_timer: Timer,
    copied_timer: Timer,
    sized: Option<Layout>,
    /// Logical height the window was last sized to.
    height: f32,
    positioned: bool,
    shown: bool,
    press: Option<Press>,
    saved: Option<(i32, i32)>,
}

thread_local! {
    static PILL: RefCell<Option<Pill>> = const { RefCell::new(None) };
}

/// Runs `f` on the pill, if it exists and is not already borrowed.
pub fn with<R>(f: impl FnOnce(&mut Pill) -> R) -> Option<R> {
    PILL.with(|p| p.try_borrow_mut().ok().and_then(|mut p| p.as_mut().map(f)))
}

pub fn exists() -> bool {
    PILL.with(|p| p.borrow().is_some())
}

pub struct Options {
    pub saved: Option<(i32, i32)>,
    pub hold: bool,
    pub no_clipboard: bool,
    pub no_trim: bool,
    pub font: Option<String>,
}

/// Creates the pill window (hidden, off screen) and runs `ready` once its
/// native window exists and is styled.
pub fn create(opts: Options, ready: impl FnOnce() + 'static) -> Result<(), slint::PlatformError> {
    crate::next_window(crate::WindowKind::Pill);
    let ui = PillWindow::new()?;
    if let Some(f) = &opts.font {
        ui.set_family(f.into());
    }
    let bars = Rc::new(VecModel::from(vec![0.0f32; BAR_COUNT]));
    ui.set_bars(ModelRc::from(bars.clone()));
    ui.on_press(|| {
        with(|p| p.on_press());
    });
    ui.on_drag_moved(|| {
        with(|p| p.on_move());
    });
    ui.on_release(|| {
        with(|p| p.on_release());
    });
    ui.on_copy(|| {
        with(|p| p.on_copy());
    });
    let pill = Pill {
        ui,
        #[cfg(windows)]
        hwnd: None,
        mode: Mode::VisualizerWithText,
        status: Status::Idle,
        note: false,
        confirm: None,
        copied: false,
        partial: String::new(),
        error: "The selected microphone is unavailable.".into(),
        enabled: true,
        hold: opts.hold,
        no_clipboard: opts.no_clipboard,
        no_trim: opts.no_trim,
        wave: Wave::default(),
        mic: FakeMic::default(),
        bars,
        bars_running: false,
        words_from_scenario: false,
        word_index: 0,
        feed: Timer::default(),
        anim: Timer::default(),
        words: Timer::default(),
        hide_timer: Timer::default(),
        confirm_timer: Timer::default(),
        copied_timer: Timer::default(),
        sized: None,
        height: 56.0,
        positioned: false,
        shown: false,
        press: None,
        saved: opts.saved,
    };
    // Slint makes the window visible itself; it is created off screen and
    // not activated, then hidden and restyled as soon as it exists.
    pill.ui.show()?;
    PILL.with(|p| *p.borrow_mut() = Some(pill));

    #[cfg(windows)]
    {
        let poll = Rc::new(Timer::default());
        let poll2 = poll.clone();
        let ready = RefCell::new(Some(ready));
        poll.start(TimerMode::Repeated, Duration::from_millis(5), move || {
            let done = with(|p| {
                let Some(h) = crate::hwnd_of(p.ui.window()) else {
                    return false;
                };
                win::hide(h);
                win::style_pill(h);
                p.hwnd = Some(h);
                log(
                    "pill-created",
                    &[
                        ("hwnd", h.0.to_string()),
                        ("exStyle", win::ex_style(h).to_string()),
                    ],
                );
                true
            })
            .unwrap_or(false);
            if done {
                poll2.stop();
                if let Some(f) = ready.borrow_mut().take() {
                    f();
                }
            }
        });
    }
    #[cfg(not(windows))]
    {
        slint::Timer::single_shot(Duration::from_millis(50), move || {
            with(|p| {
                let _ = p.ui.hide();
            });
            ready();
        });
    }
    Ok(())
}

impl Pill {
    pub fn ui(&self) -> &PillWindow {
        &self.ui
    }

    #[cfg(windows)]
    pub fn hwnd(&self) -> Option<Hwnd> {
        self.hwnd
    }

    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        self.apply();
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn status(&self) -> Status {
        self.status
    }

    /// Starts a scenario by name (see `SCENARIOS`).
    pub fn scenario(&mut self, name: &str) {
        self.confirm = None;
        self.copied = false;
        self.note = false;
        self.partial.clear();
        self.words_from_scenario = false;
        self.word_index = 0;
        let (mode, status) = match name {
            "dot" => (Mode::Dot, Status::Recording),
            "dot-error" => (Mode::Dot, Status::Error),
            "bar" => (Mode::Visualizer, Status::Recording),
            "note" => {
                self.note = true;
                (Mode::Visualizer, Status::Recording)
            }
            "text" => {
                self.words_from_scenario = true;
                (Mode::VisualizerWithText, Status::Recording)
            }
            "text-full" => {
                self.partial = fixtures::LIVE_WORDS.to_string();
                (Mode::VisualizerWithText, Status::Recording)
            }
            "pending" => (Mode::Visualizer, Status::Transcribing),
            "pending-text" => (Mode::VisualizerWithText, Status::Transcribing),
            "ready" => (Mode::Visualizer, Status::Ready),
            "error" => (Mode::Visualizer, Status::Error),
            "confirm" | "confirm-copied" => {
                self.confirm = Some(fixtures::CONFIRMED.to_string());
                self.copied = name == "confirm-copied";
                (Mode::Visualizer, Status::Ready)
            }
            "confirm-text" => {
                self.confirm = Some(fixtures::CONFIRMED.to_string());
                (Mode::VisualizerWithText, Status::Ready)
            }
            _ => (Mode::VisualizerWithText, Status::Recording),
        };
        self.mode = mode;
        self.status = status;
        log("pill-scenario", &[("name", js(name))]);
        self.apply();
        if self.confirm.is_some() && !self.hold {
            self.arm_confirm_timer();
        }
    }

    /// Recording, as the hotkey would start it.
    pub fn start_recording(&mut self, mode: Mode) {
        self.mode = mode;
        self.status = Status::Recording;
        self.confirm = None;
        self.copied = false;
        self.partial.clear();
        self.word_index = 0;
        self.words_from_scenario = mode == Mode::VisualizerWithText;
        self.apply();
    }

    pub fn hide_now(&mut self) {
        self.status = Status::Idle;
        self.confirm = None;
        self.apply();
    }

    /// Syncs the window and the component with the state (PillApp's effects).
    pub fn apply(&mut self) {
        let confirming = self.confirm.is_some();
        let recording = self.status == Status::Recording;
        let layout = layout_for(self.mode, self.status, confirming);
        let visible = self.enabled && (confirming || self.status.visible());
        let note_tone = self.note && !confirming && (recording || self.status.pending());
        let label = match self.status {
            Status::Stopping => "Saving...",
            Status::Transcribing => "Transcribing...",
            Status::Pasting => "Inserting...",
            Status::Ready => "Done",
            Status::Error => self.error.as_str(),
            _ => "",
        };
        let ui = &self.ui;
        ui.set_layout(layout.slint());
        ui.set_tone(self.status.tone());
        ui.set_confirming(confirming);
        ui.set_copied(self.copied);
        ui.set_recording(recording);
        ui.set_dot_mode(self.mode == Mode::Dot);
        ui.set_with_text(self.mode == Mode::VisualizerWithText);
        ui.set_note_tone(note_tone);
        ui.set_label(label.into());
        let live = self.partial.trim();
        ui.set_live_text(if live.is_empty() {
            "Listening...".into()
        } else {
            SharedString::from(live)
        });
        ui.set_confirm_text(self.confirm.clone().unwrap_or_default().into());
        ui.set_animate(visible);

        // The Visualizer is mounted only while recording with bars (a fresh
        // history every time it mounts, as in React).
        let bars = visible && recording && !confirming && self.mode != Mode::Dot;
        self.run_bars(bars);
        let words = visible && recording && self.words_from_scenario;
        self.run_words(words);

        if !visible {
            self.partial.clear();
            self.hide_timer.stop();
            self.hide_window();
            return;
        }

        if self.sized != Some(layout) {
            let prev = self.height;
            self.sized = Some(layout);
            let (w, h) = layout.size();
            self.height = h;
            let b = self.bounds();
            let s = self.scale();
            let y = if self.positioned {
                place::anchored_y(b.y, phys(prev, s), phys(h, s))
            } else {
                b.y
            };
            self.set_bounds(Rect {
                x: b.x,
                y,
                w: phys(w, s),
                h: phys(h, s),
            });
        }
        if !self.positioned {
            self.positioned = true;
            let monitors = self.monitors();
            let fallback = self.primary().unwrap_or(Rect {
                x: 0,
                y: 0,
                w: 1920,
                h: 1080,
            });
            let b = self.bounds();
            let (x, y) = place::first_position(&monitors, self.saved, fallback, (b.w, b.h));
            let on = self
                .saved
                .is_some_and(|(sx, sy)| place::on_screen(&monitors, sx, sy));
            log(
                "pill-placed",
                &[
                    ("x", x.to_string()),
                    ("y", y.to_string()),
                    ("usedSaved", on.to_string()),
                    ("monitors", monitors.len().to_string()),
                ],
            );
            self.set_bounds(Rect { x, y, ..b });
        }
        self.show_window();

        self.hide_timer.stop();
        if self.status == Status::Ready && !confirming && !self.hold {
            self.hide_timer.start(
                TimerMode::SingleShot,
                Duration::from_millis(READY_HIDE_DELAY_MS),
                || {
                    with(|p| {
                        p.status = Status::Idle;
                        p.apply();
                    });
                },
            );
        }
        if layout == Layout::Text {
            // After this frame's layout, as React waits for requestAnimationFrame.
            Timer::single_shot(Duration::from_millis(0), || {
                with(|p| p.grow());
            });
        }
    }

    /// Text layout: grows (or shrinks) between 80 and 150 logical px, bottom anchored.
    fn grow(&mut self) {
        let confirming = self.confirm.is_some();
        // Only a layout with a text box grows (React returns when textRef is
        // null, as it is for the pending states).
        let has_text = confirming || self.status == Status::Recording;
        if self.sized != Some(Layout::Text) || !self.shown || !has_text {
            return;
        }
        // Window height minus the text box, plus the text's natural height.
        let chrome = if confirming {
            8.0 + 2.0 + 20.0 + (11.0 * 1.3 + 8.0) + 6.0
        } else {
            8.0 + 2.0 + 20.0 + 28.0 + 6.0
        };
        let natural = if confirming {
            self.ui.get_confirm_natural()
        } else {
            self.ui.get_live_natural()
        };
        let desired = (chrome + natural).clamp(Layout::Text.size().1, TEXT_MAX_HEIGHT);
        if (desired - self.height).abs() < 2.0 {
            return;
        }
        let s = self.scale();
        let b = self.bounds();
        let y = place::anchored_y(b.y, phys(self.height, s), phys(desired, s));
        self.height = desired;
        self.set_bounds(Rect {
            x: b.x,
            y,
            w: phys(Layout::Text.size().0, s),
            h: phys(desired, s),
        });
        log(
            "pill-grow",
            &[("height", format!("{desired:.1}")), ("y", y.to_string())],
        );
    }

    fn run_bars(&mut self, on: bool) {
        if on == self.bars_running {
            return;
        }
        self.bars_running = on;
        if !on {
            self.feed.stop();
            self.anim.stop();
            return;
        }
        self.wave.reset();
        self.feed
            .start(TimerMode::Repeated, Duration::from_millis(FEED_MS), || {
                with(|p| {
                    let rms = p.mic.next_rms();
                    p.wave.push_rms(rms);
                });
            });
        self.anim
            .start(TimerMode::Repeated, Duration::from_millis(FRAME_MS), || {
                with(|p| {
                    let scales = p.wave.tick();
                    for (i, v) in scales.iter().enumerate() {
                        if p.bars
                            .row_data(i)
                            .is_none_or(|old| (old - v).abs() > 0.0005)
                        {
                            p.bars.set_row_data(i, *v);
                        }
                    }
                });
            });
    }

    fn run_words(&mut self, on: bool) {
        if !on {
            self.words.stop();
            return;
        }
        if self.words.running() {
            return;
        }
        self.words
            .start(TimerMode::Repeated, Duration::from_millis(WORD_MS), || {
                with(|p| {
                    let words: Vec<&str> = fixtures::LIVE_WORDS.split(' ').collect();
                    if p.word_index >= words.len() {
                        p.word_index = 0;
                        p.partial.clear();
                    }
                    if !p.partial.is_empty() {
                        p.partial.push(' ');
                    }
                    p.partial.push_str(words[p.word_index]);
                    p.word_index += 1;
                    p.apply();
                });
            });
    }

    fn arm_confirm_timer(&mut self) {
        self.confirm_timer.start(
            TimerMode::SingleShot,
            Duration::from_millis(CONFIRM_HIDE_DELAY_MS),
            || {
                with(|p| {
                    p.confirm = None;
                    p.copied = false;
                    p.status = Status::Idle;
                    p.apply();
                });
            },
        );
    }

    /// Click while recording: the stop flow, then the confirmation.
    fn stop(&mut self) {
        log("pill-stop", &[]);
        let text = if self.partial.trim().is_empty() {
            fixtures::CONFIRMED.to_string()
        } else {
            self.partial.trim().to_string()
        };
        self.status = Status::Stopping;
        self.apply();
        let steps: Rc<RefCell<Vec<(u64, Status)>>> = Rc::new(RefCell::new(vec![
            (250, Status::Transcribing),
            (500, Status::Pasting),
            (150, Status::Ready),
        ]));
        fn next(steps: Rc<RefCell<Vec<(u64, Status)>>>, text: String) {
            let Some((ms, st)) = (if steps.borrow().is_empty() {
                None
            } else {
                Some(steps.borrow_mut().remove(0))
            }) else {
                return;
            };
            Timer::single_shot(Duration::from_millis(ms), move || {
                with(|p| {
                    p.status = st;
                    if st == Status::Ready {
                        p.confirm = Some(text.clone());
                        p.copied = false;
                        p.arm_confirm_timer();
                    }
                    p.apply();
                    log("pill-status", &[("status", js(&format!("{st:?}")))]);
                });
                next(steps, text);
            });
        }
        next(steps, text);
    }

    fn on_press(&mut self) {
        let origin = self.bounds();
        self.press = Some(Press {
            cursor: self.cursor(),
            origin: (origin.x, origin.y),
            dragging: false,
        });
    }

    fn on_move(&mut self) {
        let cur = self.cursor();
        let s = self.scale();
        let Some(p) = self.press.as_mut() else { return };
        let (dx, dy) = (cur.0 - p.cursor.0, cur.1 - p.cursor.1);
        if !p.dragging {
            // React compares CSS (logical) px.
            let (lx, ly) = (dx as f32 / s, dy as f32 / s);
            if lx * lx + ly * ly >= DRAG_THRESHOLD_PX * DRAG_THRESHOLD_PX {
                p.dragging = true;
            }
        }
        if p.dragging {
            let (x, y) = (p.origin.0 + dx, p.origin.1 + dy);
            self.move_to(x, y);
        }
    }

    fn on_release(&mut self) {
        let Some(p) = self.press.take() else { return };
        if p.dragging {
            let b = self.bounds();
            if place::on_screen(&self.monitors(), b.x, b.y) {
                self.saved = Some((b.x, b.y));
            }
            log(
                "pill-dragged",
                &[("x", b.x.to_string()), ("y", b.y.to_string())],
            );
        } else if self.status == Status::Recording {
            self.stop();
        } else {
            log(
                "pill-click",
                &[("status", js(&format!("{:?}", self.status)))],
            );
        }
    }

    fn on_copy(&mut self) {
        let Some(text) = self.confirm.clone() else {
            return;
        };
        let wrote = if self.no_clipboard {
            false
        } else {
            crate::set_clipboard(&text)
        };
        log("pill-copy", &[("wroteClipboard", wrote.to_string())]);
        self.copied = true;
        self.apply();
        self.copied_timer.start(
            TimerMode::SingleShot,
            Duration::from_millis(COPIED_RESET_MS),
            || {
                with(|p| {
                    p.copied = false;
                    p.apply();
                });
            },
        );
    }

    // ----- the native window -----

    fn scale(&self) -> f32 {
        #[cfg(windows)]
        if let Some(h) = self.hwnd {
            return win::scale(h);
        }
        self.ui.window().scale_factor()
    }

    fn bounds(&self) -> Rect {
        #[cfg(windows)]
        if let Some(h) = self.hwnd {
            return win::bounds(h);
        }
        let p = self.ui.window().position();
        let s = self.ui.window().size();
        Rect {
            x: p.x,
            y: p.y,
            w: s.width as i32,
            h: s.height as i32,
        }
    }

    fn set_bounds(&self, r: Rect) {
        #[cfg(windows)]
        if let Some(h) = self.hwnd {
            win::set_bounds(h, r);
            return;
        }
        let w = self.ui.window();
        w.set_size(slint::PhysicalSize::new(
            r.w.max(1) as u32,
            r.h.max(1) as u32,
        ));
        w.set_position(slint::PhysicalPosition::new(r.x, r.y));
    }

    fn move_to(&self, x: i32, y: i32) {
        #[cfg(windows)]
        if let Some(h) = self.hwnd {
            win::move_to(h, x, y);
            return;
        }
        self.ui
            .window()
            .set_position(slint::PhysicalPosition::new(x, y));
    }

    fn cursor(&self) -> (i32, i32) {
        #[cfg(windows)]
        {
            win::cursor()
        }
        #[cfg(not(windows))]
        {
            (0, 0)
        }
    }

    fn monitors(&self) -> Vec<Rect> {
        #[cfg(windows)]
        {
            win::monitors()
        }
        #[cfg(not(windows))]
        {
            Vec::new()
        }
    }

    fn primary(&self) -> Option<Rect> {
        #[cfg(windows)]
        {
            win::primary_monitor()
        }
        #[cfg(not(windows))]
        {
            None
        }
    }

    fn show_window(&mut self) {
        if self.shown {
            return;
        }
        self.shown = true;
        #[cfg(windows)]
        if let Some(h) = self.hwnd {
            win::show_inactive(h);
            log("pill-shown", &[]);
            return;
        }
        let _ = self.ui.show();
        log("pill-shown", &[]);
    }

    fn hide_window(&mut self) {
        if !self.shown {
            return;
        }
        self.shown = false;
        #[cfg(windows)]
        if let Some(h) = self.hwnd {
            win::hide(h);
            log("pill-hidden", &[]);
            if !self.no_trim {
                win::trim_working_set();
            }
            return;
        }
        let _ = self.ui.hide();
        let _ = self.no_trim;
        log("pill-hidden", &[]);
    }
}

fn phys(logical: f32, scale: f32) -> i32 {
    (logical * scale).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_follow_pill_layout() {
        use Mode::*;
        use Status::*;
        assert_eq!(layout_for(Dot, Recording, false), Layout::Dot);
        assert_eq!(layout_for(Dot, Ready, true), Layout::Bar);
        assert_eq!(layout_for(Visualizer, Recording, false), Layout::Bar);
        assert_eq!(
            layout_for(VisualizerWithText, Recording, false),
            Layout::Text
        );
        assert_eq!(
            layout_for(VisualizerWithText, Transcribing, false),
            Layout::Text
        );
        assert_eq!(layout_for(VisualizerWithText, Ready, false), Layout::Bar);
        assert_eq!(layout_for(VisualizerWithText, Ready, true), Layout::Text);
        assert_eq!(layout_for(VisualizerWithText, Error, false), Layout::Bar);
    }
}
