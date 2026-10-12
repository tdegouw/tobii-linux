// SPDX-License-Identifier: GPL-3.0-or-later
//! et5-cursor: gaze-controlled pointer.
//!
//!   et5-cursor daemon            run the daemon (systemd user service)
//!   et5-cursor toggle|on|off     control the daemon (shows a notification)
//!   et5-cursor status            print on/off
mod filter;

use evdev::{uinput::VirtualDeviceBuilder, AbsInfo, AbsoluteAxisType, AttributeSet, EventType, InputEvent, Key, UinputAbsSetup};
use filter::{FilterConfig, GazeFilter};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    sync::{atomic::{AtomicBool, Ordering}, Arc},
    thread,
    time::{Duration, Instant},
};
use tobii_et5::{Config, Device};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct CursorConfig { filter: FilterConfig }

fn cfg_path() -> PathBuf { Config::path().with_file_name("cursor.toml") }
fn load_cfg() -> CursorConfig {
    let p = cfg_path();
    match std::fs::read_to_string(&p) {
        Ok(s) => toml::from_str(&s).unwrap_or_else(|e| { eprintln!("{}: {e}; using defaults", p.display()); CursorConfig::default() }),
        Err(_) => { let c = CursorConfig::default(); let _ = std::fs::write(&p, toml::to_string_pretty(&c).unwrap()); c }
    }
}
fn sock_path() -> PathBuf {
    PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").unwrap_or_else(|| "/tmp".into())).join("et5-cursor.sock")
}

fn notify(msg: &str) {
    let _ = std::process::Command::new("notify-send").args(["-a", "et5-cursor", "-t", "2000", "Oogbesturing", msg]).status();
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
            if cmd != "status" { notify(if state == "on" { "Oogbesturing aan" } else { "Oogbesturing uit" }); }
            0
        }
        Err(e) => {
            eprintln!("et5-cursor daemon not reachable: {e}");
            if cmd != "status" { notify("Oogbesturing: daemon draait niet (systemctl --user status et5-cursor)"); }
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
        .with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisType::ABS_X, AbsInfo::new(0, 0, w - 1, 0, 0, 1)))?
        .with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisType::ABS_Y, AbsInfo::new(0, 0, h - 1, 0, 0, 1)))?
        .build()
}

fn serve(enabled: Arc<AtomicBool>) -> std::io::Result<()> {
    let p = sock_path();
    let _ = std::fs::remove_file(&p);
    let l = UnixListener::bind(&p)?;
    for s in l.incoming() {
        let Ok(mut s) = s else { continue };
        let mut line = String::new();
        if BufReader::new(&s).read_line(&mut line).is_err() { continue; }
        match line.trim() {
            "toggle" => { enabled.fetch_xor(true, Ordering::Relaxed); }
            "on" => enabled.store(true, Ordering::Relaxed),
            "off" => enabled.store(false, Ordering::Relaxed),
            _ => {}
        }
        let on = enabled.load(Ordering::Relaxed);
        eprintln!("state: {}", if on { "on" } else { "off" });
        let _ = writeln!(s, "{}", if on { "on" } else { "off" });
    }
    Ok(())
}

fn daemon() -> i32 {
    let cfg = load_cfg();
    let enabled = Arc::new(AtomicBool::new(false)); // default OFF
    let e2 = enabled.clone();
    thread::spawn(move || if let Err(e) = serve(e2) { eprintln!("socket: {e}"); std::process::exit(1) });

    let dcfg = match Config::load_or_init() { Ok(c) => c, Err(e) => { eprintln!("{e}"); return 1 } };
    let (w, h) = (dcfg.display.width_px as i32, dcfg.display.height_px as i32);
    let mut ptr = match make_pointer(w, h) { Ok(p) => p, Err(e) => { eprintln!("uinput: {e}"); return 1 } };
    eprintln!("et5-cursor: uinput pointer {w}x{h}, state off, filter {:?}", cfg.filter);

    loop {
        // (Re)connect; a missing or replugged tracker is retried every 3 s.
        let mut dev = match Device::open_streaming(dcfg.clone(), false) {
            Ok(d) => { eprintln!("tracker {} streaming", d.serial()); d }
            Err(e) => { eprintln!("tracker: {e}; retrying"); thread::sleep(Duration::from_secs(3)); continue; }
        };
        let mut filt = GazeFilter::new(cfg.filter.clone());
        let mut last: Option<Instant> = None;
        let mut was_on = false;
        loop {
            let g = match dev.recv_gaze(Duration::from_secs(2)) {
                Ok(Some((at, g))) => (at, g),
                Ok(None) => { eprintln!("tracker: no gaze for 2 s, reconnecting"); break; }
                Err(e) => { eprintln!("tracker: {e}, reconnecting"); break; }
            };
            let on = enabled.load(Ordering::Relaxed);
            if on != was_on { filt.reset(); was_on = on; }
            if !on { continue; }
            let (at, g) = g;
            // No eyes: freeze the cursor (do nothing).
            let Some(c) = g.combined.filter(|_| g.left.valid || g.right.valid) else { last = None; continue };
            let dt = last.map_or(1.0 / 33.0, |l| at.duration_since(l).as_secs_f64());
            last = Some(at);
            let (x, y) = (c[0].clamp(0.0, 1.0) * w as f64, c[1].clamp(0.0, 1.0) * h as f64);
            if let Some((px, py)) = filt.update(x, y, dt) {
                let ev = [
                    InputEvent::new(EventType::ABSOLUTE, AbsoluteAxisType::ABS_X.0, px.clamp(0, w - 1)),
                    InputEvent::new(EventType::ABSOLUTE, AbsoluteAxisType::ABS_Y.0, py.clamp(0, h - 1)),
                ];
                if let Err(e) = ptr.emit(&ev) { eprintln!("uinput emit: {e}"); }
            }
        }
        drop(dev);
        thread::sleep(Duration::from_secs(1));
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let code = match a.first().map(String::as_str) {
        Some("daemon") => daemon(),
        Some(c @ ("toggle" | "on" | "off" | "status")) => client(c),
        _ => { eprintln!("usage: et5-cursor daemon | toggle | on | off | status"); 2 }
    };
    std::process::exit(code);
}
