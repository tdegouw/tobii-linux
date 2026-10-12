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
        Config {
            display: Display {
                width_px: 5120,
                height_px: 1440,
                width_mm: 1193.0,
                height_mm: 336.0,
                bottom_offset_mm: 10.0,
                tracker_x_offset_mm: 0.0,
            },
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
            });
        base.join("tobii-et5").join("config.toml")
    }
    /// Load the config, writing the default if none exists.
    pub fn load_or_init() -> Result<Self> {
        let p = Self::path();
        match std::fs::read_to_string(&p) {
            Ok(s) => toml::from_str::<Config>(&s)
                .map_err(|e| Error::Config(format!("{}: {e}", p.display())))
                .and_then(|c| c.validate().map(|_| c)),
            Err(_) => {
                let c = Config::default();
                if let Some(d) = p.parent() {
                    let _ = std::fs::create_dir_all(d);
                }
                if let Ok(t) = toml::to_string_pretty(&c) {
                    let _ = std::fs::write(&p, t);
                }
                Ok(c)
            }
        }
    }
    /// Reject geometry that would produce a degenerate display plane.
    pub fn validate(&self) -> Result<()> {
        let d = &self.display;
        let pos = |v: f64| v.is_finite() && v > 0.0;
        if d.width_px == 0 || d.height_px == 0 || d.width_px > 32768 || d.height_px > 32768 {
            return Err(Error::Config(
                "display.width_px/height_px must be 1..32768".into(),
            ));
        }
        if !pos(d.width_mm)
            || !pos(d.height_mm)
            || !d.bottom_offset_mm.is_finite()
            || !d.tracker_x_offset_mm.is_finite()
        {
            return Err(Error::Config(
                "display sizes must be finite and width_mm/height_mm > 0".into(),
            ));
        }
        Ok(())
    }
    /// Map normalised gaze (0..1) to display pixels (clamped).
    pub fn to_pixels(&self, g: [f64; 2]) -> (i32, i32) {
        let d = &self.display;
        (
            (g[0].clamp(0.0, 1.0) * d.width_px as f64) as i32,
            (g[1].clamp(0.0, 1.0) * d.height_px as f64) as i32,
        )
    }
    /// Display corners TL, TR, BL in device units (µm).
    pub fn corners_um(&self) -> [[f64; 3]; 3] {
        let d = &self.display;
        let (l, r) = (
            -d.width_mm / 2.0 - d.tracker_x_offset_mm,
            d.width_mm / 2.0 - d.tracker_x_offset_mm,
        );
        let (b, t) = (d.bottom_offset_mm, d.bottom_offset_mm + d.height_mm);
        [[l, t, 0.0], [r, t, 0.0], [l, b, 0.0]].map(|p| p.map(|v| v * 1000.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_geometry() {
        let c = Config::default();
        c.validate().unwrap();
        let [tl, tr, bl] = c.corners_um();
        assert_eq!(tl, [-596_500.0, 346_000.0, 0.0]);
        assert_eq!(tr, [596_500.0, 346_000.0, 0.0]);
        assert_eq!(bl, [-596_500.0, 10_000.0, 0.0]);
        assert_eq!(c.to_pixels([0.5, 0.5]), (2560, 720));
        assert_eq!(c.to_pixels([-1.0, 2.0]), (0, 1440));
    }

    #[test]
    fn rejects_bad_geometry() {
        let mut c = Config::default();
        c.display.width_mm = 0.0;
        assert!(c.validate().is_err());
        let mut c = Config::default();
        c.display.height_px = 0;
        assert!(c.validate().is_err());
        assert!(toml::from_str::<Config>("[display]\nwidth_px = 1").is_err());
    }
}
