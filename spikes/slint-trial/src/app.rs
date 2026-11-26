//! Life cycle: the backend, the tray, the pill, the main window made on
//! demand and destroyed on close, and the measuring hooks.

use crate::pill::{self, Mode};
use crate::status::{self, Config, js, log};
use crate::{HistoryRow, MainWindow, WindowKind, fixtures, next_kind, next_window, probe};
use slint::{CloseRequestResponse, ComponentHandle, ModelRc, Timer, TimerMode, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

#[cfg(windows)]
use crate::win;

struct State {
    cfg: Config,
    pill_enabled: bool,
    #[cfg(windows)]
    tray: Option<tray::Tray>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    static MAIN: RefCell<Option<MainWindow>> = const { RefCell::new(None) };
    static KEEP: RefCell<Vec<Timer>> = const { RefCell::new(Vec::new()) };
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|s| s.try_borrow_mut().ok().and_then(|mut s| s.as_mut().map(f)))
}

/// Keeps a timer alive for the whole run.
fn keep(t: Timer) {
    KEEP.with(|k| k.borrow_mut().push(t));
}

fn after(ms: u64, f: impl FnOnce() + 'static) {
    Timer::single_shot(Duration::from_millis(ms), f);
}

fn pill_options() -> pill::Options {
    with_state(|s| {
        #[cfg_attr(not(windows), allow(unused_mut))]
        let mut saved = s.cfg.pill_pos;
        #[cfg(windows)]
        if saved.is_none()
            && let Some(n) = s.cfg.monitor
            && let Some(m) = win::monitors().get(n)
        {
            saved = Some((m.x + m.w / 2 - 80, m.y + m.h / 2));
        }
        pill::Options {
            saved,
            hold: s.cfg.hold,
            no_clipboard: s.cfg.no_clipboard,
            no_trim: s.cfg.no_trim,
            font: s.cfg.pill_font.clone(),
        }
    })
    .expect("state")
}

/// Runs `then` once the pill exists (creating it the first time).
fn ensure_pill(then: impl FnOnce() + 'static) {
    if pill::exists() {
        then();
        return;
    }
    let enabled = with_state(|s| s.pill_enabled).unwrap_or(true);
    if let Err(e) = pill::create(pill_options(), move || {
        pill::with(|p| p.set_enabled(enabled));
        then();
    }) {
        log("error", &[("what", js(&format!("pill: {e}")))]);
    }
}

pub fn show_pill_recording() {
    ensure_pill(|| {
        pill::with(|p| p.start_recording(Mode::VisualizerWithText));
    });
}

fn set_pill_enabled(on: bool) {
    with_state(|s| {
        s.pill_enabled = on;
        #[cfg(windows)]
        if let Some(t) = &s.tray {
            t.check.set_checked(on);
        }
    });
    pill::with(|p| p.set_enabled(on));
    MAIN.with(|m| {
        if let Some(m) = m.borrow().as_ref()
            && m.get_pill_enabled() != on
        {
            m.set_pill_enabled(on);
        }
    });
    log("pill-enabled", &[("on", on.to_string())]);
}

fn copy_text(text: &str) {
    let no = with_state(|s| s.cfg.no_clipboard).unwrap_or(true);
    let wrote = if no {
        false
    } else {
        crate::set_clipboard(text)
    };
    log(
        "copy",
        &[
            ("chars", text.chars().count().to_string()),
            ("wroteClipboard", wrote.to_string()),
        ],
    );
}

/// Makes the main window, or brings the existing one forward.
pub fn open_main(page: i32, activate: bool) {
    let existing = MAIN.with(|m| {
        m.borrow().as_ref().map(|m| {
            m.set_page(page);
            if activate {
                use slint::winit_030::WinitWindowAccessor;
                m.window().with_winit_window(|w| {
                    w.set_minimized(false);
                    w.focus_window();
                });
            }
        })
    });
    if existing.is_some() {
        return;
    }
    let t0 = probe::now_ms();
    next_window(WindowKind::Main { activate });
    let m = match MainWindow::new() {
        Ok(m) => m,
        Err(e) => {
            log("error", &[("what", js(&format!("main window: {e}")))]);
            return;
        }
    };
    // `--row-fonts=0` shows the rows without per-language fonts (the default fallback).
    let no_family = std::env::args().any(|a| a == "--row-fonts=0");
    let rows: Vec<HistoryRow> = fixtures::history()
        .into_iter()
        .map(|r| HistoryRow {
            rtl: fixtures::starts_rtl(&r.preview),
            family: if no_family {
                "".into()
            } else {
                fixtures::family_for(r.lang).into()
            },
            time: r.time.into(),
            lang: r.lang.into(),
            preview: r.preview.into(),
        })
        .collect();
    m.set_rows(ModelRc::from(Rc::new(VecModel::from(rows))));
    let transcript = fixtures::transcript();
    m.set_transcript_chars(transcript.chars().count() as i32);
    m.set_transcript(transcript.into());
    m.set_page(page);
    m.set_pill_enabled(with_state(|s| s.pill_enabled).unwrap_or(true));
    m.on_show_pill(show_pill_recording);
    m.on_copy_transcript(|| {
        let text = MAIN.with(|m| m.borrow().as_ref().map(|m| m.get_transcript().to_string()));
        if let Some(t) = text {
            copy_text(&t);
        }
    });
    m.on_pill_toggled(|on| {
        let current = with_state(|s| s.pill_enabled).unwrap_or(true);
        if current != on {
            set_pill_enabled(on);
        }
    });
    m.on_page_changed(|i| log("page", &[("page", i.to_string())]));
    m.window().on_close_requested(|| {
        after(0, close_main);
        CloseRequestResponse::HideWindow
    });
    probe::watch(m.window());
    if let Err(e) = m.show() {
        log("error", &[("what", js(&format!("show: {e}")))]);
        return;
    }
    log(
        "main-open",
        &[
            ("page", page.to_string()),
            ("activate", activate.to_string()),
        ],
    );
    probe::on_next_frame(move |end| {
        log("main-first-frame", &[("ms", format!("{:.1}", end - t0))]);
    });
    MAIN.with(|slot| *slot.borrow_mut() = Some(m));
    if !activate {
        // Off every monitor Windows sends no first WM_PAINT; ask Slint for the
        // frame itself (later redraws, from property changes, paint normally).
        after(50, || {
            main_window(|m| m.window().request_redraw());
        });
    }
}

/// Destroys the main window (not just hides it).
pub fn close_main() {
    let m = MAIN.with(|m| m.borrow_mut().take());
    let Some(m) = m else { return };
    let _ = m.hide();
    drop(m);
    log("main-closed", &[]);
    #[cfg(windows)]
    if !with_state(|s| s.cfg.no_trim).unwrap_or(false) {
        after(500, win::trim_working_set);
    }
}

fn main_window<R>(f: impl FnOnce(&MainWindow) -> R) -> Option<R> {
    MAIN.with(|m| m.borrow().as_ref().map(f))
}

/// Scrolls the history list down and back for 8 s, 40 px every 16 ms, timing every frame.
fn probe_scroll() {
    after(1500, || {
        let Some((content, visible)) =
            main_window(|m| (m.get_list_content_height(), m.get_list_visible_height()))
        else {
            return;
        };
        probe::record(true);
        let t = Timer::default();
        let start = Instant::now();
        let down = Rc::new(std::cell::Cell::new(true));
        let steps = Rc::new(std::cell::Cell::new(0u32));
        t.start(TimerMode::Repeated, Duration::from_millis(16), move || {
            if start.elapsed() > Duration::from_secs(8) {
                return;
            }
            main_window(|m| {
                let max = (m.get_list_content_height() - m.get_list_visible_height()).max(0.0);
                let mut y = -m.get_list_y();
                if down.get() {
                    y += 40.0;
                    if y >= max {
                        y = max;
                        down.set(false);
                    }
                } else {
                    y -= 40.0;
                    if y <= 0.0 {
                        y = 0.0;
                        down.set(true);
                    }
                }
                m.set_list_y(-y);
            });
            steps.set(steps.get() + 1);
        });
        keep(t);
        after(8300, move || {
            probe::record(false);
            let s = probe::stats();
            log(
                "probe-scroll",
                &[
                    ("frames", s.frames.to_string()),
                    ("maxDrawMs", format!("{:.1}", s.max_draw)),
                    ("p95DrawMs", format!("{:.1}", s.p95_draw)),
                    ("maxGapMs", format!("{:.1}", s.max_gap)),
                    ("p95GapMs", format!("{:.1}", s.p95_gap)),
                    ("drawsOver50", s.draws_over_50.to_string()),
                    ("gapsOver50", s.gaps_over_50.to_string()),
                    ("contentHeight", format!("{content:.0}")),
                    ("visibleHeight", format!("{visible:.0}")),
                ],
            );
        });
    });
}

/// Switches History -> Editor five times; each time the 60,000-character
/// editor is built from nothing. Times the click to the first frame drawn.
fn probe_editor() {
    fn round(n: u32, mut times: Vec<f64>) {
        if n == 0 {
            let list: Vec<String> = times.iter().map(|t| format!("{t:.1}")).collect();
            log(
                "probe-editor",
                &[("openMs", format!("[{}]", list.join(",")))],
            );
            return;
        }
        let t0 = probe::now_ms();
        main_window(|m| m.set_page(1));
        probe::on_next_frame(move |end| {
            times.push(end - t0);
            after(700, move || {
                main_window(|m| m.set_page(0));
                after(700, move || round(n - 1, times));
            });
        });
    }
    after(1500, || round(5, Vec::new()));
}

/// Records every main-window frame from 3 s to 23 s after start, while real
/// input (mouse wheel, typing) drives the window, and logs the frame stats.
fn probe_frames() {
    after(3000, || probe::record(true));
    after(23_000, || {
        probe::record(false);
        let s = probe::stats();
        log(
            "probe-frames",
            &[
                ("frames", s.frames.to_string()),
                ("maxDrawMs", format!("{:.1}", s.max_draw)),
                ("p95DrawMs", format!("{:.1}", s.p95_draw)),
                ("drawsOver50", s.draws_over_50.to_string()),
            ],
        );
    });
}

/// Opens and closes the main window five times, timing open to first frame.
fn probe_open() {
    fn round(n: u32, mut times: Vec<f64>) {
        if n == 0 {
            let list: Vec<String> = times.iter().map(|t| format!("{t:.1}")).collect();
            log("probe-open", &[("openMs", format!("[{}]", list.join(",")))]);
            return;
        }
        let t0 = probe::now_ms();
        open_main(0, false);
        probe::on_next_frame(move |end| {
            times.push(end - t0);
            after(800, move || {
                close_main();
                after(800, move || round(n - 1, times));
            });
        });
    }
    after(1500, || round(5, Vec::new()));
}

/// Shows every pill state in turn and saves Slint's own picture of each.
fn snapshot_all(dir: std::path::PathBuf) {
    let _ = std::fs::create_dir_all(&dir);
    fn step(i: usize, dir: std::path::PathBuf) {
        let Some(name) = pill::SCENARIOS.get(i) else {
            log("snapshots-done", &[]);
            after(200, || {
                let _ = slint::quit_event_loop();
            });
            return;
        };
        pill::with(|p| p.scenario(name));
        // Long enough for the bars to move and the text to grow.
        let wait = if *name == "text" { 3500 } else { 900 };
        after(wait, move || {
            let saved = pill::with(|p| match p.ui().window().take_snapshot() {
                Ok(buf) => {
                    let path = dir.join(format!("trial-{name}.png"));
                    write_png(&path, buf.width(), buf.height(), buf.as_bytes()).map(|_| path)
                }
                Err(e) => Err(e.to_string()),
            });
            match saved {
                Some(Ok(path)) => log(
                    "snapshot",
                    &[("state", js(name)), ("path", js(&path.to_string_lossy()))],
                ),
                Some(Err(e)) => log("error", &[("what", js(&format!("snapshot {name}: {e}")))]),
                None => {}
            }
            pill::with(|p| p.hide_now());
            after(300, move || step(i + 1, dir));
        });
    }
    ensure_pill(move || step(0, dir));
}

fn write_png(path: &std::path::Path, w: u32, h: u32, rgba: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(rgba).map_err(|e| e.to_string())
}

/// Cycles the pill through every state with hides in between (foreground and
/// animation checks).
fn pill_cycle() {
    let t = Timer::default();
    let i = Rc::new(std::cell::Cell::new(0usize));
    t.start(
        TimerMode::Repeated,
        Duration::from_millis(1200),
        move || {
            let n = i.get();
            i.set(n + 1);
            if n % 2 == 1 {
                pill::with(|p| p.hide_now());
            } else {
                let name = pill::SCENARIOS[(n / 2) % pill::SCENARIOS.len()];
                pill::with(|p| p.scenario(name));
            }
        },
    );
    keep(t);
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Config::load();
    let start = Instant::now();
    if let Some(p) = &cfg.font_path {
        // SAFETY: still single-threaded; nothing else reads the environment yet.
        unsafe { std::env::set_var("SLINT_FONT_PATH", p) };
    }
    probe::now_ms();
    status::init(cfg.status.clone(), start);

    let selector = slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name(cfg.renderer.clone())
        .with_winit_window_attributes_hook(|a| match next_kind() {
            WindowKind::Pill => {
                use slint::winit_030::winit::dpi::PhysicalPosition;
                use slint::winit_030::winit::window::WindowLevel;
                let a = a
                    .with_active(false)
                    .with_resizable(false)
                    .with_decorations(false)
                    .with_transparent(true)
                    .with_window_level(WindowLevel::AlwaysOnTop)
                    .with_position(PhysicalPosition::new(-32000, -32000));
                #[cfg(windows)]
                let a = {
                    use slint::winit_030::winit::platform::windows::WindowAttributesExtWindows;
                    a.with_skip_taskbar(true)
                };
                a
            }
            // A measuring run opens the main window off every monitor, so it is
            // never on the owner's screen; the tray opens it normally.
            // `--main-onscreen=1` keeps the default place (frame timings need it).
            WindowKind::Main { activate: false }
                if std::env::args().any(|a| a == "--main-onscreen=1") =>
            {
                a.with_active(false)
            }
            WindowKind::Main { activate: false } => {
                use slint::winit_030::winit::dpi::PhysicalPosition;
                a.with_active(false)
                    .with_position(PhysicalPosition::new(-20000, -20000))
            }
            WindowKind::Main { activate: true } => a.with_active(true),
        });
    selector.select()?;
    log(
        "start",
        &[
            ("renderer", js(&cfg.renderer)),
            ("pid", std::process::id().to_string()),
            ("version", js(env!("CARGO_PKG_VERSION"))),
        ],
    );

    let open = cfg.open;
    let scenario = cfg.pill.clone();
    let close_after = cfg.close_after;
    let exit_after = cfg.exit_after;
    let probe_name = cfg.probe.clone();
    let fake_click = cfg.fake_click;
    let snapshot = cfg.snapshot.clone();
    let snapshot_main = cfg.snapshot_main.clone();
    STATE.with(|s| {
        *s.borrow_mut() = Some(State {
            cfg,
            pill_enabled: true,
            #[cfg(windows)]
            tray: None,
        })
    });

    #[cfg(windows)]
    {
        let t = tray::build()?;
        with_state(|s| s.tray = Some(t));
    }

    if let Some(ms) = exit_after {
        after(ms, || {
            log("exit-after", &[]);
            let _ = slint::quit_event_loop();
        });
    }
    if let Some(page) = open {
        // Measuring runs never take the focus.
        open_main(page, false);
    }
    if let Some(ms) = close_after {
        after(ms, close_main);
    }
    match scenario.as_deref() {
        Some("cycle") => ensure_pill(pill_cycle),
        Some(name) => {
            let name = name.to_string();
            ensure_pill(move || {
                pill::with(|p| p.scenario(&name));
            });
        }
        None => {}
    }
    if let Some(ms) = fake_click {
        after(ms, fake_click_now);
    }
    if let Some(dir) = snapshot {
        snapshot_all(dir);
    }
    if let Some(file) = snapshot_main {
        probe::on_next_frame(move |_| {
            after(1500, move || {
                let saved = main_window(|m| m.window().take_snapshot());
                match saved {
                    Some(Ok(buf)) => {
                        let r = write_png(&file, buf.width(), buf.height(), buf.as_bytes());
                        log(
                            "snapshot-main",
                            &[
                                ("ok", r.is_ok().to_string()),
                                ("path", js(&file.to_string_lossy())),
                            ],
                        );
                    }
                    Some(Err(e)) => log("error", &[("what", js(&format!("snapshot-main: {e}")))]),
                    None => log("error", &[("what", js("snapshot-main: no window"))]),
                }
                let _ = slint::quit_event_loop();
            });
        });
    }
    match probe_name.as_deref() {
        Some("scroll") => probe_scroll(),
        Some("editor") => probe_editor(),
        Some("open") => probe_open(),
        Some("frames") => probe_frames(),
        _ => {}
    }

    slint::run_event_loop_until_quit()?;
    log("exit", &[]);
    close_main();
    #[cfg(windows)]
    with_state(|s| s.tray.take());
    // Leave without running the remaining destructors (as Pulse does).
    std::process::exit(0);
}

/// Posts a click to the middle of the pill (our own window only).
fn fake_click_now() {
    #[cfg(windows)]
    {
        let info = pill::with(|p| (p.hwnd(), p.status()));
        let Some((Some(h), status)) = info else {
            log("fake-click", &[("ok", "false".into())]);
            return;
        };
        let b = win::bounds(h);
        let fg = win::foreground();
        win::post_click(h, b.w / 2, b.h / 2);
        log(
            "fake-click",
            &[
                ("ok", "true".into()),
                ("statusBefore", js(&format!("{status:?}"))),
                ("foreground", fg.to_string()),
            ],
        );
        after(600, move || {
            let now = pill::with(|p| p.status()).unwrap_or(pill::Status::Idle);
            log(
                "fake-click-after",
                &[
                    ("status", js(&format!("{now:?}"))),
                    ("foreground", win::foreground().to_string()),
                    ("foregroundUnchanged", (win::foreground() == fg).to_string()),
                ],
            );
        });
    }
}

#[cfg(windows)]
mod tray {
    use super::*;
    use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{
        Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    };

    pub struct Tray {
        _icon: TrayIcon,
        pub check: CheckMenuItem,
    }

    /// A 32 px amber dot on a navy disc, drawn in code.
    fn icon() -> Icon {
        let n = 32u32;
        let mut rgba = Vec::with_capacity((n * n * 4) as usize);
        for y in 0..n {
            for x in 0..n {
                let (dx, dy) = (x as f32 - 15.5, y as f32 - 15.5);
                let d = (dx * dx + dy * dy).sqrt();
                let outer = (15.5 - d).clamp(0.0, 1.0);
                let inner = (6.5 - d).clamp(0.0, 1.0);
                let (r, g, b) = if inner > 0.0 {
                    (0xfb, 0xbf, 0x24)
                } else {
                    (0x0f, 0x1e, 0x38)
                };
                rgba.extend_from_slice(&[r, g, b, (outer * 255.0) as u8]);
            }
        }
        Icon::from_rgba(rgba, n, n).expect("icon")
    }

    pub fn build() -> Result<Tray, Box<dyn std::error::Error>> {
        let menu = Menu::new();
        let open = MenuItem::new("Open Scribe trial", true, None);
        let show = MenuItem::new("Show the pill (recording)", true, None);
        let check = CheckMenuItem::new("Show floating pill", true, true, None);
        let quit = MenuItem::new("Quit", true, None);
        menu.append_items(&[
            &open,
            &show,
            &PredefinedMenuItem::separator(),
            &check,
            &PredefinedMenuItem::separator(),
            &quit,
        ])?;
        let ids = [
            open.id().clone(),
            show.id().clone(),
            check.id().clone(),
            quit.id().clone(),
        ];
        MenuEvent::set_event_handler(Some(std::sync::Arc::new(move |e: MenuEvent| {
            let Some(which) = ids.iter().position(|id| *id == e.id) else {
                return;
            };
            let _ = slint::invoke_from_event_loop(move || match which {
                0 => open_main(0, true),
                1 => show_pill_recording(),
                2 => {
                    // The menu flips its own check mark; follow it.
                    let checked = with_state(|s| s.tray.as_ref().map(|t| t.check.is_checked()))
                        .flatten()
                        .unwrap_or(true);
                    set_pill_enabled(checked);
                }
                _ => {
                    let _ = slint::quit_event_loop();
                }
            });
        })));
        TrayIconEvent::set_event_handler(Some(|e: TrayIconEvent| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = e
            {
                let _ = slint::invoke_from_event_loop(|| open_main(0, true));
            }
        }));
        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip("Scribe Slint trial")
            .with_icon(icon())
            .build()?;
        Ok(Tray { _icon: icon, check })
    }
}
