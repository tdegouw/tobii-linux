// SPDX-License-Identifier: GPL-3.0-or-later
//! Minimal Hyprland IPC client (request socket `.socket.sock`).
use serde_json::Value;
use std::{io::{Read, Write}, os::unix::net::UnixStream, path::PathBuf, time::Duration};

pub fn socket() -> Option<PathBuf> {
    let rt = std::env::var_os("XDG_RUNTIME_DIR")?;
    let base = PathBuf::from(rt).join("hypr");
    // Prefer the env signature; fall back to the only/newest instance (systemd units may lack it).
    if let Some(sig) = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE") {
        let p = base.join(sig).join(".socket.sock");
        if p.exists() { return Some(p); }
    }
    std::fs::read_dir(&base).ok()?
        .filter_map(|e| e.ok()).map(|e| e.path().join(".socket.sock")).filter(|p| p.exists())
        .max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok())
}

pub fn request(cmd: &str) -> std::io::Result<String> {
    let p = socket().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no Hyprland socket"))?;
    let mut s = UnixStream::connect(p)?;
    s.set_read_timeout(Some(Duration::from_millis(500)))?;
    s.write_all(cmd.as_bytes())?;
    let mut out = String::new();
    s.read_to_string(&mut out)?;
    Ok(out)
}
pub fn json(cmd: &str) -> Option<Value> { serde_json::from_str(&request(&format!("j/{cmd}")).ok()?).ok() }

#[derive(Debug, Clone)]
pub struct Win { pub addr: String, pub x: f64, pub y: f64, pub w: f64, pub h: f64, pub floating: bool }

pub struct Snapshot {
    pub windows: Vec<Win>, // floating first (they are on top)
    pub active: Option<String>,
    /// Focus switching should be suspended (fullscreen, floating popup focused, launcher/overlay layer open).
    pub blocked: bool,
}

fn layer_blocks(v: &Value) -> bool {
    // Any layer on the top/overlay levels other than the bar = launcher, menu, OSD, calibration...
    let ok = ["omarchy-bar", "omarchy-background", "notifications", "mako", "swaync", "omarchy-notif"];
    v.as_object().into_iter().flat_map(|m| m.values()).any(|mon| {
        ["2", "3"].iter().any(|lvl| mon["levels"][lvl].as_array().is_some_and(|a| a.iter().any(|l| {
            let ns = l["namespace"].as_str().unwrap_or("");
            !ok.iter().any(|o| ns.starts_with(o))
        })))
    })
}

pub fn snapshot() -> Option<Snapshot> {
    let ws = json("activeworkspace")?;
    let ws_id = ws["id"].as_i64()?;
    let aw = json("activewindow");
    let active = aw.as_ref().and_then(|a| a["address"].as_str()).map(String::from);
    let active_floating = aw.as_ref().and_then(|a| a["floating"].as_bool()).unwrap_or(false);
    let mut windows: Vec<Win> = json("clients")?.as_array()?.iter().filter(|c| {
        c["workspace"]["id"].as_i64() == Some(ws_id) && c["mapped"].as_bool() == Some(true) && c["hidden"].as_bool() != Some(true)
    }).filter_map(|c| Some(Win {
        addr: c["address"].as_str()?.to_string(),
        x: c["at"][0].as_f64()?, y: c["at"][1].as_f64()?, w: c["size"][0].as_f64()?, h: c["size"][1].as_f64()?,
        floating: c["floating"].as_bool().unwrap_or(false),
    })).collect();
    windows.sort_by_key(|w| !w.floating);
    let blocked = ws["hasfullscreen"].as_bool().unwrap_or(false) || active_floating
        || json("layers").is_some_and(|l| layer_blocks(&l));
    Some(Snapshot { windows, active, blocked })
}

pub fn cursor_pos() -> Option<(i64, i64)> {
    let v = json("cursorpos")?;
    Some((v["x"].as_i64()?, v["y"].as_i64()?))
}

/// Focus a window without warping the pointer: no_warps is enabled only for this
/// dispatch (in one IPC batch) and restored afterwards.
pub fn focus(addr: &str) -> std::io::Result<String> {
    request(&format!(
        "[[BATCH]]eval hl.config({{ cursor = {{ no_warps = true }} }});dispatch hl.dsp.focus({{ window = \"address:{addr}\" }});eval hl.config({{ cursor = {{ no_warps = false }} }})"
    ))
}
