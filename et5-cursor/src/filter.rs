// SPDX-License-Identifier: GPL-3.0-or-later
//! One-euro filter (Casiez et al. 2012) + fixation stabiliser.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FilterConfig {
    /// One-euro minimum cutoff (Hz). Lower = smoother/slower when the gaze is still.
    pub min_cutoff: f64,
    /// One-euro speed coefficient. Higher = less lag during fast eye movements.
    pub beta: f64,
    /// Cutoff for the derivative (Hz).
    pub d_cutoff: f64,
    /// Fixation dead-zone radius in pixels: the cursor stays put while the filtered
    /// gaze remains within this radius of it.
    pub deadzone_px: f64,
    /// Jumps larger than this (pixels) are treated as a saccade: the filter is reset
    /// and the cursor moves immediately.
    pub saccade_px: f64,
    /// Once outside the dead zone, move this fraction of the remaining distance per sample.
    pub follow: f64,
}
impl Default for FilterConfig {
    fn default() -> Self {
        FilterConfig { min_cutoff: 0.6, beta: 0.004, d_cutoff: 1.0, deadzone_px: 60.0, saccade_px: 600.0, follow: 0.5 }
    }
}

struct Lowpass { y: Option<f64> }
impl Lowpass {
    fn f(&mut self, x: f64, a: f64) -> f64 { let y = self.y.map_or(x, |p| p + a * (x - p)); self.y = Some(y); y }
}
fn alpha(cutoff: f64, dt: f64) -> f64 { let tau = 1.0 / (2.0 * std::f64::consts::PI * cutoff); 1.0 / (1.0 + tau / dt) }

struct OneEuro { x: Lowpass, dx: Lowpass, prev: Option<f64> }
impl OneEuro {
    fn new() -> Self { OneEuro { x: Lowpass { y: None }, dx: Lowpass { y: None }, prev: None } }
    fn f(&mut self, v: f64, dt: f64, c: &FilterConfig) -> f64 {
        let d = self.prev.map_or(0.0, |p| (v - p) / dt);
        self.prev = Some(v);
        let ed = self.dx.f(d, alpha(c.d_cutoff, dt));
        let cutoff = c.min_cutoff + c.beta * ed.abs();
        self.x.f(v, alpha(cutoff, dt))
    }
}

/// Pixel-space gaze filter: one-euro per axis, then dead-zone stabilisation.
pub struct GazeFilter { cfg: FilterConfig, fx: OneEuro, fy: OneEuro, cursor: Option<(f64, f64)>, last_raw: Option<(f64, f64)> }
impl GazeFilter {
    pub fn new(cfg: FilterConfig) -> Self { GazeFilter { cfg, fx: OneEuro::new(), fy: OneEuro::new(), cursor: None, last_raw: None } }
    pub fn reset(&mut self) { self.fx = OneEuro::new(); self.fy = OneEuro::new(); self.last_raw = None; }
    /// Feed a raw pixel sample; returns a new cursor position if it should move.
    pub fn update(&mut self, x: f64, y: f64, dt: f64) -> Option<(i32, i32)> {
        let dt = dt.clamp(0.005, 0.2);
        if let Some((lx, ly)) = self.last_raw {
            if ((x - lx).powi(2) + (y - ly).powi(2)).sqrt() > self.cfg.saccade_px { self.reset(); }
        }
        self.last_raw = Some((x, y));
        let (sx, sy) = (self.fx.f(x, dt, &self.cfg), self.fy.f(y, dt, &self.cfg));
        let (cx, cy) = match self.cursor {
            None => { self.cursor = Some((sx, sy)); return Some((sx as i32, sy as i32)); }
            Some(c) => c,
        };
        let dist = ((sx - cx).powi(2) + (sy - cy).powi(2)).sqrt();
        if dist <= self.cfg.deadzone_px { return None; }
        let k = if dist > self.cfg.saccade_px { 1.0 } else { self.cfg.follow };
        let n = (cx + (sx - cx) * k, cy + (sy - cy) * k);
        self.cursor = Some(n);
        Some((n.0.round() as i32, n.1.round() as i32))
    }
}
