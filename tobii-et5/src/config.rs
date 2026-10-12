// SPDX-License-Identifier: GPL-3.0-or-later
//! `~/.config/tobii-et5/config.toml`
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub display: Display,
}

/// Physical display geometry, tracker at origin (x right, y up, mm).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Display {
    pub width_px: u32,
    pub height_px: u32,
    pub width_mm: f64,
    pub height_mm: f64,
    /// Gap between tracker and bottom edge of the visible image (mm).
    pub bottom_offset_mm: f64,
    /// Horizontal offset of the tracker from the screen centre (mm, + = tracker right of centre).
    #[serde(default)]
    pub tracker_x_offset_mm: f64,
}

impl Default for Config {
    fn default() -> Self {
        // 49" 32:9 (e.g. Samsung G9 / Odyssey 49"), tracker bottom-centre. Measure & adjust!
        Config { display: Display { width_px: 5120, height_px: 1440, width_mm: 1193.0, height_mm: 336.0, bottom_offset_mm: 10.0, tracker_x_offset_mm: 0.0 } }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"));
        base.join("tobii-et5").join("config.toml")
    }
    /// Load the config, writing the default if none exists.
    pub fn load_or_init() -> Result<Self> {
        let p = Self::path();
        match std::fs::read_to_string(&p) {
            Ok(s) => toml::from_str(&s).map_err(|e| Error::Config(format!("{}: {e}", p.display()))),
            Err(_) => {
                let c = Config::default();
                if let Some(d) = p.parent() { let _ = std::fs::create_dir_all(d); }
                let _ = std::fs::write(&p, toml::to_string_pretty(&c).unwrap());
                Ok(c)
            }
        }
    }
    /// Map normalised gaze (0..1) to display pixels (clamped).
    pub fn to_pixels(&self, g: [f64; 2]) -> (i32, i32) {
        let d = &self.display;
        ((g[0].clamp(0.0, 1.0) * d.width_px as f64) as i32, (g[1].clamp(0.0, 1.0) * d.height_px as f64) as i32)
    }
    /// Display corners TL, TR, BL in device units (µm).
    pub fn corners_um(&self) -> [[f64; 3]; 3] {
        let d = &self.display;
        let (l, r) = (-d.width_mm / 2.0 - d.tracker_x_offset_mm, d.width_mm / 2.0 - d.tracker_x_offset_mm);
        let (b, t) = (d.bottom_offset_mm, d.bottom_offset_mm + d.height_mm);
        [[l, t, 0.0], [r, t, 0.0], [l, b, 0.0]].map(|p| p.map(|v| v * 1000.0))
    }
}
