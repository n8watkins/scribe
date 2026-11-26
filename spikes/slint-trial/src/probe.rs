//! Frame timing for the main window: when each frame starts (winit's
//! RedrawRequested, before Slint draws) and ends (the next turn of the event
//! loop, after the draw). Works the same for every renderer.

use slint::winit_030::winit::event::WindowEvent;
use slint::winit_030::{EventResult, WinitWindowAccessor};
use std::cell::RefCell;
use std::time::Instant;

#[derive(Default)]
struct Recorder {
    origin: Option<Instant>,
    recording: bool,
    frames: Vec<(f64, f64)>,
    waiters: Vec<Box<dyn FnOnce(f64)>>,
}

thread_local! {
    static REC: RefCell<Recorder> = RefCell::new(Recorder::default());
}

pub fn now_ms() -> f64 {
    REC.with(|r| {
        let mut r = r.borrow_mut();
        let o = *r.origin.get_or_insert_with(Instant::now);
        o.elapsed().as_secs_f64() * 1000.0
    })
}

/// Starts timing this window's frames.
pub fn watch(w: &slint::Window) {
    w.on_winit_window_event(|_, ev| {
        if matches!(ev, WindowEvent::RedrawRequested) {
            let start = now_ms();
            let _ = slint::invoke_from_event_loop(move || frame_done(start));
        }
        EventResult::Propagate
    });
}

fn frame_done(start: f64) {
    let end = now_ms();
    let waiters = REC.with(|r| {
        let mut r = r.borrow_mut();
        if r.recording {
            r.frames.push((start, end));
        }
        std::mem::take(&mut r.waiters)
    });
    for w in waiters {
        w(end);
    }
}

/// Runs `f` with the end time of the next frame drawn by any watched window.
pub fn on_next_frame(f: impl FnOnce(f64) + 'static) {
    REC.with(|r| r.borrow_mut().waiters.push(Box::new(f)));
}

pub fn record(on: bool) {
    REC.with(|r| {
        let mut r = r.borrow_mut();
        r.recording = on;
        if on {
            r.frames.clear();
        }
    });
}

pub struct Stats {
    pub frames: usize,
    pub max_draw: f64,
    pub p95_draw: f64,
    pub max_gap: f64,
    pub p95_gap: f64,
    pub draws_over_50: usize,
    pub gaps_over_50: usize,
}

pub fn stats() -> Stats {
    REC.with(|r| {
        let r = r.borrow();
        let mut draws: Vec<f64> = r.frames.iter().map(|(s, e)| e - s).collect();
        let mut gaps: Vec<f64> = r.frames.windows(2).map(|w| w[1].1 - w[0].1).collect();
        let pct = |v: &mut Vec<f64>, p: f64| {
            if v.is_empty() {
                return 0.0;
            }
            v.sort_by(|a, b| a.total_cmp(b));
            v[((v.len() as f64 - 1.0) * p).round() as usize]
        };
        Stats {
            frames: r.frames.len(),
            max_draw: draws.iter().copied().fold(0.0, f64::max),
            p95_draw: pct(&mut draws, 0.95),
            max_gap: gaps.iter().copied().fold(0.0, f64::max),
            p95_gap: pct(&mut gaps, 0.95),
            draws_over_50: draws.iter().filter(|d| **d > 50.0).count(),
            gaps_over_50: gaps.iter().filter(|d| **d > 50.0).count(),
        }
    })
}
