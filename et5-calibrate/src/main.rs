// SPDX-License-Identifier: GPL-3.0-or-later
//! Fullscreen layer-shell calibration overlay for the Tobii Eye Tracker 5.
//!
//! Flow: connect -> 3 s countdown -> 7 points in rounds 1/3/3 (each dot shrinks over
//! 1.5 s, SHOW_POINT after 0.7 s) -> COMPUTE per round -> GET_STATUS (blob) -> APPLY ->
//! STOP -> save to ~/.config/tobii-et5/<serial>/calibration.bin. Esc aborts.
//! Layer-shell handling follows smithay-client-toolkit's `simple_layer` example (MIT).
mod ui;

use std::{
    sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use tobii_et5::{calibration::{POINTS, ROUNDS}, Config, Device};

#[derive(Clone, Debug)]
pub enum Phase {
    Connecting,
    Countdown { until: Instant },
    Point { x: f64, y: f64, start: Instant },
    Computing,
    Done { ok: bool, at: Instant },
}

pub struct Shared {
    pub phase: Mutex<Phase>,
    pub abort: AtomicBool,
    pub finished: AtomicBool,
}

pub const DOT_TIME: Duration = Duration::from_millis(1500);
const FIXATION: Duration = Duration::from_millis(700);
const COUNTDOWN: Duration = Duration::from_secs(3);

fn set(s: &Shared, p: Phase) { *s.phase.lock().unwrap() = p; }

fn sleep_until(s: &Shared, t: Instant) -> bool {
    while Instant::now() < t {
        if s.abort.load(Ordering::Relaxed) { return false; }
        thread::sleep(Duration::from_millis(10));
    }
    !s.abort.load(Ordering::Relaxed)
}

fn calibrate(s: &Shared, dev: &mut Device) -> Result<String, String> {
    let until = Instant::now() + COUNTDOWN;
    set(s, Phase::Countdown { until });
    if !sleep_until(s, until) { return Err("afgebroken".into()); }
    let e = |x: tobii_et5::Error| x.to_string();
    dev.calibration_start().map_err(e)?;
    dev.calibration_clear().map_err(e)?;
    for round in ROUNDS {
        for &i in round {
            let (x, y) = POINTS[i];
            let start = Instant::now();
            set(s, Phase::Point { x, y, start });
            if !sleep_until(s, start + FIXATION) { return Err("afgebroken".into()); }
            let st = dev.calibration_show_point(x, y).map_err(e)?;
            eprintln!("point {i} ({x:.2},{y:.2}): status 0x{st:08x}");
            if !sleep_until(s, start + DOT_TIME) { return Err("afgebroken".into()); }
        }
        set(s, Phase::Computing);
        let st = dev.calibration_compute().map_err(e)?;
        eprintln!("compute: status 0x{st:08x}");
    }
    let blob = dev.calibration_retrieve().map_err(e)?;
    let st = dev.calibration_apply().map_err(e)?;
    eprintln!("apply: status 0x{st:08x}");
    dev.calibration_stop().map_err(e)?;
    if blob.is_empty() { return Err("device returned an empty calibration".into()); }
    let p = dev.save_calibration(&blob).map_err(|x| x.to_string())?;
    Ok(format!("saved {} bytes to {}", blob.len(), p.display()))
}

fn worker(s: Arc<Shared>, dry_run: bool) {
    let res = if dry_run {
        // UI test without touching the tracker.
        let until = Instant::now() + COUNTDOWN;
        set(&s, Phase::Countdown { until });
        let mut ok = sleep_until(&s, until);
        for round in ROUNDS {
            for &i in round {
                if !ok { break; }
                let start = Instant::now();
                set(&s, Phase::Point { x: POINTS[i].0, y: POINTS[i].1, start });
                ok = sleep_until(&s, start + DOT_TIME);
            }
        }
        if ok { Ok("dry run".to_string()) } else { Err("afgebroken".to_string()) }
    } else {
        match Config::load_or_init().map_err(|e| e.to_string())
            .and_then(|c| Device::open_streaming(c, false).map_err(|e| e.to_string()))
        {
            Err(e) => Err(format!("tracker: {e}")),
            Ok(mut dev) => {
                eprintln!("tracker {} connected", dev.serial());
                let r = calibrate(&s, &mut dev);
                if r.is_err() { let _ = dev.calibration_stop(); }
                r
            }
        }
    };
    match &res {
        Ok(m) => eprintln!("calibration done: {m}"),
        Err(m) => eprintln!("calibration failed: {m}"),
    }
    set(&s, Phase::Done { ok: res.is_ok(), at: Instant::now() });
    if !s.abort.load(Ordering::Relaxed) { thread::sleep(Duration::from_millis(1500)); }
    s.finished.store(true, Ordering::Relaxed);
}

fn main() {
    let dry_run = std::env::args().any(|a| a == "--dry-run");
    let shared = Arc::new(Shared { phase: Mutex::new(Phase::Connecting), abort: AtomicBool::new(false), finished: AtomicBool::new(false) });
    let s2 = shared.clone();
    let w = thread::spawn(move || worker(s2, dry_run));
    if let Err(e) = ui::run(shared.clone()) {
        eprintln!("overlay: {e}");
        shared.abort.store(true, Ordering::Relaxed);
    }
    let _ = w.join();
    let ok = matches!(*shared.phase.lock().unwrap(), Phase::Done { ok: true, .. });
    std::process::exit(if ok { 0 } else { 1 });
}
