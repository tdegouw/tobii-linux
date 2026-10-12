// SPDX-License-Identifier: GPL-3.0-or-later
//! et5-cursor: gaze-controlled pointer.
//!
//!   et5-cursor daemon               run the daemon (systemd user service)
//!   et5-cursor toggle|on|off        control the daemon (shows a notification)
//!   et5-cursor mode focus|cursor    switch mode (focus = gaze focuses windows, default)
//!   et5-cursor status               print state and mode
mod filter;
mod hypr;

use evdev::{
    uinput::VirtualDeviceBuilder, AbsInfo, AbsoluteAxisType, AttributeSet, EventType, InputEvent,
    Key, UinputAbsSetup,
};
use filter::{FilterConfig, GazeFilter};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use tobii_et5::{Config, Device};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
enum Mode {
    #[default]
    Focus,
    Cursor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct FocusConfig {
    /// Gaze must stay on another window this long before it gets focus.
    dwell_ms: u64,
    /// Hysteresis: gaze must be at least this far inside a window (px).
    margin_px: f64,
    /// Ignore gaze for this long after the mouse moved.
    mouse_quiet_ms: u64,
}
impl Default for FocusConfig {
    fn default() -> Self {
        FocusConfig {
            dwell_ms: 400,
            margin_px: 80.0,
            mouse_quiet_ms: 1000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct CursorConfig {
    mode: Mode,
    focus: FocusConfig,
    filter: FilterConfig,
}

fn cfg_path() -> PathBuf {
    Config::path().with_file_name("cursor.toml")
}
impl CursorConfig {
    /// Clamp values to sane ranges, logging anything that was changed.
    fn validated(mut self) -> Self {
        fn fix(name: &str, v: &mut f64, lo: f64, hi: f64) {
            let c = if v.is_finite() { v.clamp(lo, hi) } else { lo };
            if c != *v {
                eprintln!("cursor.toml: {name} = {v} out of range, using {c}");
                *v = c;
            }
        }
        let f = &mut self.filter;
        fix("min_cutoff", &mut f.min_cutoff, 0.01, 30.0);
        fix("beta", &mut f.beta, 0.0, 1.0);
        fix("d_cutoff", &mut f.d_cutoff, 0.01, 30.0);
        fix("deadzone_px", &mut f.deadzone_px, 0.0, 1000.0);
        fix("saccade_px", &mut f.saccade_px, 1.0, 10000.0);
        fix("follow", &mut f.follow, 0.01, 1.0);
        fix("margin_px", &mut self.focus.margin_px, 0.0, 1000.0);
        self.focus.dwell_ms = self.focus.dwell_ms.clamp(50, 10_000);
        self
    }
}

fn load_cfg() -> CursorConfig {
    let p = cfg_path();
    let c = match std::fs::read_to_string(&p) {
        Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
            eprintln!("{}: {e}; using defaults", p.display());
            CursorConfig::default()
        }),
        Err(_) => {
            let c = CursorConfig::default();
            if let Ok(t) = toml::to_string_pretty(&c) {
                let _ = std::fs::write(&p, t);
            }
            c
        }
    };
    c.validated()
}
fn sock_path() -> PathBuf {
    PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").unwrap_or_else(|| "/tmp".into()))
        .join("et5-cursor.sock")
}

fn notify(msg: &str) {
    let _ = std::process::Command::new("notify-send")
        .args(["-a", "et5-cursor", "-t", "2000", "Oogbesturing", msg])
        .status();
}

fn client(cmd: &str) -> i32 {
    let r = UnixStream::connect(sock_path()).and_then(|mut s| {
        s.set_read_timeout(Some(Duration::from_secs(2)))?;
        writeln!(s, "{cmd}")?;
        let mut line = String::new();
        BufReader::new(s).read_line(&mut line)?;
        Ok(line.trim().to_string())
    });
    match r {
        Ok(state) => {
            println!("{state}");
            if cmd != "status" {
                let on = state.starts_with("on");
                let what = if state.ends_with("cursor") {
                    "Oogbesturing"
                } else {
                    "Oogfocus"
                };
                notify(&format!("{what} {}", if on { "aan" } else { "uit" }));
            }
            0
        }
        Err(e) => {
            eprintln!("et5-cursor daemon not reachable: {e}");
            if cmd != "status" {
                notify("Oogfocus: daemon draait niet (systemctl --user status et5-cursor)");
            }
            1
        }
    }
}

fn make_pointer(w: i32, h: i32) -> std::io::Result<evdev::uinput::VirtualDevice> {
    let mut keys = AttributeSet::<Key>::new();
    keys.insert(Key::BTN_LEFT); // needed for libinput to treat it as a pointer
    VirtualDeviceBuilder::new()?
        .name("et5-cursor gaze pointer")
        .with_keys(&keys)?
        .with_absolute_axis(&UinputAbsSetup::new(
            AbsoluteAxisType::ABS_X,
            AbsInfo::new(0, 0, w - 1, 0, 0, 1),
        ))?
        .with_absolute_axis(&UinputAbsSetup::new(
            AbsoluteAxisType::ABS_Y,
            AbsInfo::new(0, 0, h - 1, 0, 0, 1),
        ))?
        .build()
}

fn serve(enabled: Arc<AtomicBool>, cursor_mode: Arc<AtomicBool>) -> std::io::Result<()> {
    let p = sock_path();
    let _ = std::fs::remove_file(&p);
    let l = UnixListener::bind(&p)?;
    for s in l.incoming() {
        let Ok(mut s) = s else { continue };
        let mut line = String::new();
        // A client that connects but never writes must not block the server.
        let _ = s.set_read_timeout(Some(Duration::from_secs(1)));
        if BufReader::new(&s).read_line(&mut line).is_err() {
            continue;
        }
        match line.trim() {
            "toggle" => {
                enabled.fetch_xor(true, Ordering::Relaxed);
            }
            "on" => enabled.store(true, Ordering::Relaxed),
            "off" => enabled.store(false, Ordering::Relaxed),
            "mode focus" => cursor_mode.store(false, Ordering::Relaxed),
            "mode cursor" => cursor_mode.store(true, Ordering::Relaxed),
            _ => {}
        }
        let st = format!(
            "{} {}",
            if enabled.load(Ordering::Relaxed) {
                "on"
            } else {
                "off"
            },
            if cursor_mode.load(Ordering::Relaxed) {
                "cursor"
            } else {
                "focus"
            }
        );
        eprintln!("state: {st}");
        let _ = writeln!(s, "{st}");
    }
    Ok(())
}

fn daemon() -> i32 {
    let cfg = load_cfg();
    let enabled = Arc::new(AtomicBool::new(false)); // default OFF
    let cursor_mode = Arc::new(AtomicBool::new(cfg.mode == Mode::Cursor));
    let (e2, m2) = (enabled.clone(), cursor_mode.clone());
    thread::spawn(move || {
        if let Err(e) = serve(e2, m2) {
            eprintln!("socket: {e}");
            std::process::exit(1)
        }
    });

    let dcfg = match Config::load_or_init() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let (w, h) = (dcfg.display.width_px as i32, dcfg.display.height_px as i32);
    // The virtual pointer is only created when cursor mode is first used.
    let mut ptr: Option<evdev::uinput::VirtualDevice> = None;
    // SIGTERM/SIGINT (systemctl stop): leave the loop so Device::drop pauses and
    // unsubscribes the tracker. Being killed mid-stream used to leave it wedged.
    let term = Arc::new(AtomicBool::new(false));
    for sig in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT] {
        let _ = signal_hook::flag::register(sig, term.clone());
    }
    eprintln!(
        "et5-cursor: {w}x{h}, state off, mode {:?}, focus {:?}, filter {:?}",
        cfg.mode, cfg.focus, cfg.filter
    );
    let mut focus = FocusState::new(cfg.focus.clone());

    while !term.load(Ordering::Relaxed) {
        // (Re)connect; a missing or replugged tracker is retried every 3 s.
        let mut dev = match Device::open_streaming(dcfg.clone(), false) {
            Ok(d) => {
                eprintln!("tracker {} streaming", d.serial());
                d
            }
            Err(e) => {
                eprintln!("tracker: {e}; retrying");
                for _ in 0..30 {
                    if term.load(Ordering::Relaxed) {
                        break;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
                continue;
            }
        };
        let mut filt = GazeFilter::new(cfg.filter.clone());
        let mut last: Option<Instant> = None;
        let mut was_on = false;
        let mut was_cursor = cursor_mode.load(Ordering::Relaxed);
        while !term.load(Ordering::Relaxed) {
            let g = match dev.recv_gaze(Duration::from_secs(2)) {
                Ok(Some((at, g))) => (at, g),
                Ok(None) => {
                    eprintln!("tracker: no gaze for 2 s, reconnecting");
                    break;
                }
                Err(e) => {
                    eprintln!("tracker: {e}, reconnecting");
                    break;
                }
            };
            let on = enabled.load(Ordering::Relaxed);
            if on != was_on {
                filt.reset();
                focus.reset();
                was_on = on;
            }
            if !on {
                continue;
            }
            let cursor = cursor_mode.load(Ordering::Relaxed);
            if cursor != was_cursor {
                filt.reset();
                focus.reset();
                was_cursor = cursor;
            }
            let (at, g) = g;
            // No eyes: freeze the cursor (do nothing).
            let Some(c) = g.combined.filter(|_| g.left.valid || g.right.valid) else {
                last = None;
                continue;
            };
            let dt = last.map_or(1.0 / 33.0, |l| at.duration_since(l).as_secs_f64());
            last = Some(at);
            let (x, y) = (
                c[0].clamp(0.0, 1.0) * w as f64,
                c[1].clamp(0.0, 1.0) * h as f64,
            );
            if !cursor {
                let (sx, sy) = filt.smooth(x, y, dt);
                focus.update(sx, sy, at);
                continue;
            }
            if ptr.is_none() {
                match make_pointer(w, h) {
                    Ok(p) => ptr = Some(p),
                    Err(e) => {
                        eprintln!("uinput: {e}; cursor mode unavailable");
                        cursor_mode.store(false, Ordering::Relaxed);
                        continue;
                    }
                }
            }
            if let (Some((px, py)), Some(ptr)) = (filt.update(x, y, dt), ptr.as_mut()) {
                let ev = [
                    InputEvent::new(
                        EventType::ABSOLUTE,
                        AbsoluteAxisType::ABS_X.0,
                        px.clamp(0, w - 1),
                    ),
                    InputEvent::new(
                        EventType::ABSOLUTE,
                        AbsoluteAxisType::ABS_Y.0,
                        py.clamp(0, h - 1),
                    ),
                ];
                if let Err(e) = ptr.emit(&ev) {
                    eprintln!("uinput emit: {e}");
                }
            }
        }
        drop(dev);
        if !term.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_secs(1));
        }
    }
    let _ = std::fs::remove_file(sock_path());
    eprintln!("et5-cursor: stopped");
    0
}

/// Gaze-to-focus state machine: dwell + hysteresis, suspended while the mouse moves.
struct FocusState {
    cfg: FocusConfig,
    snap: Option<hypr::Snapshot>,
    snap_at: Option<Instant>,
    mouse: Option<(i64, i64)>,
    mouse_at: Option<Instant>,
    mouse_moved: Option<Instant>,
    candidate: Option<(String, Instant)>,
}
impl FocusState {
    fn new(cfg: FocusConfig) -> Self {
        FocusState {
            cfg,
            snap: None,
            snap_at: None,
            mouse: None,
            mouse_at: None,
            mouse_moved: None,
            candidate: None,
        }
    }
    fn reset(&mut self) {
        self.candidate = None;
        self.snap = None;
        self.mouse = None;
    }
    fn update(&mut self, x: f64, y: f64, now: Instant) {
        // Mouse activity (polled at 10 Hz)
        if self
            .mouse_at
            .is_none_or(|t| now.duration_since(t) > Duration::from_millis(100))
        {
            self.mouse_at = Some(now);
            let p = hypr::cursor_pos();
            if p.is_some() && self.mouse.is_some() && p != self.mouse {
                self.mouse_moved = Some(now);
                self.candidate = None;
            }
            self.mouse = p;
        }
        if self
            .mouse_moved
            .is_some_and(|t| now.duration_since(t) < Duration::from_millis(self.cfg.mouse_quiet_ms))
        {
            return;
        }
        // Window layout (polled at 4 Hz)
        if self
            .snap_at
            .is_none_or(|t| now.duration_since(t) > Duration::from_millis(250))
        {
            self.snap_at = Some(now);
            self.snap = hypr::snapshot();
        }
        let Some(snap) = &self.snap else { return };
        if snap.blocked {
            self.candidate = None;
            return;
        }
        let m = self.cfg.margin_px;
        let Some(win) = hypr::hit_test(&snap.windows, x, y, m).cloned() else {
            self.candidate = None;
            return;
        };
        if snap.active.as_deref() == Some(&win.addr) {
            self.candidate = None;
            return;
        }
        match &self.candidate {
            Some((a, since)) if *a == win.addr => {
                if now.duration_since(*since) >= Duration::from_millis(self.cfg.dwell_ms) {
                    match hypr::focus(&win.addr) {
                        Ok(_) => eprintln!("focus {}", win.addr),
                        Err(e) => eprintln!("focus {}: {e}", win.addr),
                    }
                    if let Some(s) = &mut self.snap {
                        s.active = Some(win.addr.clone());
                    }
                    self.candidate = None;
                }
            }
            _ => self.candidate = Some((win.addr.clone(), now)),
        }
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let code = match a.first().map(String::as_str) {
        Some("daemon") => daemon(),
        Some(c @ ("toggle" | "on" | "off" | "status")) => client(c),
        Some("hypr-debug") => {
            // Read-only: print what focus mode sees (no dispatch).
            match hypr::snapshot() {
                Some(sn) => {
                    println!("active {:?} blocked {}", sn.active, sn.blocked);
                    for w in sn.windows {
                        println!(
                            "{} at {},{} {}x{} floating {}",
                            w.addr, w.x, w.y, w.w, w.h, w.floating
                        );
                    }
                    println!("cursor {:?}", hypr::cursor_pos());
                    0
                }
                None => {
                    eprintln!("no Hyprland IPC");
                    1
                }
            }
        }
        Some("mode") => match a.get(1).map(String::as_str) {
            Some(m @ ("focus" | "cursor")) => client(&format!("mode {m}")),
            _ => {
                eprintln!("usage: et5-cursor mode focus|cursor");
                2
            }
        },
        _ => {
            eprintln!("usage: et5-cursor daemon | toggle | on | off | status | mode focus|cursor");
            2
        }
    };
    std::process::exit(code);
}
