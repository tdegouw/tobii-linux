// SPDX-License-Identifier: GPL-3.0-or-later
//! Minimal Hyprland IPC client (request socket `.socket.sock`).
use serde_json::Value;
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    time::Duration,
};

pub fn socket() -> Option<PathBuf> {
    let rt = std::env::var_os("XDG_RUNTIME_DIR")?;
    let base = PathBuf::from(rt).join("hypr");
    // Prefer the env signature; fall back to the only/newest instance (systemd units may lack it).
    if let Some(sig) = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE") {
        let p = base.join(sig).join(".socket.sock");
        if p.exists() {
            return Some(p);
        }
    }
    std::fs::read_dir(&base)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path().join(".socket.sock"))
        .filter(|p| p.exists())
        .max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok())
}

pub fn request(cmd: &str) -> std::io::Result<String> {
    let p = socket()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no Hyprland socket"))?;
    let mut s = UnixStream::connect(p)?;
    s.set_read_timeout(Some(Duration::from_millis(500)))?;
    s.write_all(cmd.as_bytes())?;
    let mut out = String::new();
    s.read_to_string(&mut out)?;
    Ok(out)
}
pub fn json(cmd: &str) -> Option<Value> {
    serde_json::from_str(&request(&format!("j/{cmd}")).ok()?).ok()
}

#[derive(Debug, Clone)]
pub struct Win {
    pub addr: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub floating: bool,
}

pub struct Snapshot {
    pub windows: Vec<Win>, // floating first (they are on top)
    pub active: Option<String>,
    /// Focus switching should be suspended (fullscreen, floating popup focused, launcher/overlay layer open).
    pub blocked: bool,
}

fn layer_blocks(v: &Value) -> bool {
    // Any layer on the top/overlay levels other than the bar = launcher, menu, OSD, calibration...
    let ok = [
        "omarchy-bar",
        "omarchy-background",
        "notifications",
        "mako",
        "swaync",
        "omarchy-notif",
    ];
    v.as_object()
        .into_iter()
        .flat_map(|m| m.values())
        .any(|mon| {
            ["2", "3"].iter().any(|lvl| {
                mon["levels"][lvl].as_array().is_some_and(|a| {
                    a.iter().any(|l| {
                        let ns = l["namespace"].as_str().unwrap_or("");
                        !ok.iter().any(|o| ns.starts_with(o))
                    })
                })
            })
        })
}

pub fn snapshot() -> Option<Snapshot> {
    let ws = json("activeworkspace")?;
    let ws_id = ws["id"].as_i64()?;
    let aw = json("activewindow");
    let active = aw
        .as_ref()
        .and_then(|a| a["address"].as_str())
        .map(String::from);
    let active_floating = aw
        .as_ref()
        .and_then(|a| a["floating"].as_bool())
        .unwrap_or(false);
    let mut windows: Vec<Win> = json("clients")?
        .as_array()?
        .iter()
        .filter(|c| {
            c["workspace"]["id"].as_i64() == Some(ws_id)
                && c["mapped"].as_bool() == Some(true)
                && c["hidden"].as_bool() != Some(true)
        })
        .filter_map(|c| {
            Some(Win {
                addr: c["address"].as_str()?.to_string(),
                x: c["at"][0].as_f64()?,
                y: c["at"][1].as_f64()?,
                w: c["size"][0].as_f64()?,
                h: c["size"][1].as_f64()?,
                floating: c["floating"].as_bool().unwrap_or(false),
            })
        })
        .collect();
    windows.sort_by_key(|w| !w.floating);
    let blocked = ws["hasfullscreen"].as_bool().unwrap_or(false)
        || active_floating
        || json("layers").is_some_and(|l| layer_blocks(&l));
    Some(Snapshot {
        windows,
        active,
        blocked,
    })
}

pub fn cursor_pos() -> Option<(i64, i64)> {
    let v = json("cursorpos")?;
    Some((v["x"].as_i64()?, v["y"].as_i64()?))
}

/// Window under (x, y) at least `margin` px inside its edges. Windows are ordered
/// floating-first, so a floating window on top wins over the tiled one below it.
pub fn hit_test(windows: &[Win], x: f64, y: f64, margin: f64) -> Option<&Win> {
    windows.iter().find(|w| {
        x >= w.x + margin && x <= w.x + w.w - margin && y >= w.y + margin && y <= w.y + w.h - margin
    })
}

/// Current value of `cursor:no_warps`, if Hyprland reports it.
pub fn no_warps() -> Option<bool> {
    json("getoption cursor:no_warps")?["bool"].as_bool()
}

fn is_hex_addr(a: &str) -> bool {
    a.strip_prefix("0x")
        .is_some_and(|h| !h.is_empty() && h.chars().all(|c| c.is_ascii_hexdigit()))
}

/// Build the IPC batch that focuses `addr` without warping the pointer.
/// If `no_warps` is already on in the user's config, the batch is just the dispatch.
pub fn focus_batch(addr: &str, already_no_warps: bool) -> Option<String> {
    if !is_hex_addr(addr) {
        return None; // never interpolate anything but a window address into Lua
    }
    let d = format!("dispatch hl.dsp.focus({{ window = \"address:{addr}\" }})");
    Some(if already_no_warps {
        d
    } else {
        format!(
            "[[BATCH]]eval hl.config({{ cursor = {{ no_warps = true }} }});{d};eval hl.config({{ cursor = {{ no_warps = false }} }})"
        )
    })
}

/// Focus a window without warping the pointer: no_warps is enabled only for this
/// dispatch (in one IPC batch) and restored to `false` afterwards.
pub fn focus(addr: &str) -> std::io::Result<String> {
    let cmd = focus_batch(addr, no_warps().unwrap_or(false)).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "bad window address")
    })?;
    request(&cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(addr: &str, x: f64, w: f64, floating: bool) -> Win {
        Win {
            addr: addr.into(),
            x,
            y: 38.0,
            w,
            h: 1390.0,
            floating,
        }
    }

    #[test]
    fn hit_test_margin_and_order() {
        let ws = vec![
            win("0xf", 1000.0, 600.0, true),
            win("0xa", 12.0, 1266.0, false),
            win("0xb", 1292.0, 1261.0, false),
        ];
        // inside tiled window a
        assert_eq!(
            hit_test(&ws, 500.0, 700.0, 80.0).map(|w| w.addr.as_str()),
            Some("0xa")
        );
        // within the margin of a's right edge: nothing (hysteresis)
        assert!(hit_test(&ws[1..], 1250.0, 700.0, 80.0).is_none());
        // floating window on top of a and b wins
        assert_eq!(
            hit_test(&ws, 1300.0, 700.0, 80.0).map(|w| w.addr.as_str()),
            Some("0xf")
        );
        // outside everything
        assert!(hit_test(&ws, 6000.0, 700.0, 80.0).is_none());
    }

    #[test]
    fn layers() {
        let bar: Value = serde_json::from_str(r#"{"DP-1":{"levels":{"0":[{"namespace":"omarchy-background"}],"2":[{"namespace":"omarchy-bar"}],"3":[]}}}"#).unwrap();
        assert!(!layer_blocks(&bar));
        let launcher: Value = serde_json::from_str(r#"{"DP-1":{"levels":{"2":[{"namespace":"omarchy-bar"}],"3":[{"namespace":"walker"}]}}}"#).unwrap();
        assert!(layer_blocks(&launcher));
    }

    #[test]
    fn focus_batch_restores_and_validates() {
        let b = focus_batch("0x5b14e2cea6e0", false).unwrap();
        assert!(b.starts_with("[[BATCH]]"));
        assert!(b.contains("no_warps = true") && b.ends_with("no_warps = false } })"));
        assert_eq!(
            focus_batch("0x1", true).unwrap(),
            "dispatch hl.dsp.focus({ window = \"address:0x1\" })"
        );
        assert!(focus_batch("0x1\"); os.exit()", false).is_none());
        assert!(focus_batch("", false).is_none());
    }
}
