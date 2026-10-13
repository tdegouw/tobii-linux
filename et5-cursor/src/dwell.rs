// SPDX-License-Identifier: GPL-3.0-or-later
//! Focus-mode gaze point and dwell decision.
//!
//! Focus mode only needs to know *which window* the gaze is in, not a smooth point,
//! so it uses a light exponential filter (reset on saccades) instead of the cursor
//! mode one-euro filter, and a dwell that tolerates brief outliers/dropouts: a window
//! wins once it has been the candidate for `dwell_ms` and got at least `majority` of
//! the samples since, with no gap longer than `max_gap_ms` without a hit.
use std::time::{Duration, Instant};

/// Light EMA on the raw gaze point; jumps larger than `saccade_px` snap immediately.
pub struct FocusPoint {
    alpha: f64,
    saccade_px: f64,
    p: Option<(f64, f64)>,
}
impl FocusPoint {
    pub fn new(alpha: f64, saccade_px: f64) -> Self {
        FocusPoint {
            alpha,
            saccade_px,
            p: None,
        }
    }
    pub fn reset(&mut self) {
        self.p = None;
    }
    pub fn update(&mut self, x: f64, y: f64) -> (f64, f64) {
        let n = match self.p {
            Some((px, py)) if ((x - px).powi(2) + (y - py).powi(2)).sqrt() <= self.saccade_px => {
                (px + self.alpha * (x - px), py + self.alpha * (y - py))
            }
            _ => (x, y),
        };
        self.p = Some(n);
        n
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DwellCfg {
    pub dwell: Duration,
    pub majority: f64,
    pub max_gap: Duration,
}

struct Cand {
    addr: String,
    start: Instant,
    last_hit: Instant,
    hits: u32,
    total: u32,
}

#[derive(Default)]
pub struct Dwell {
    cand: Option<Cand>,
}
impl Dwell {
    pub fn reset(&mut self) {
        self.cand = None;
    }
    /// Time the current candidate was first seen (for latency logging).
    pub fn since(&self) -> Option<Instant> {
        self.cand.as_ref().map(|c| c.start)
    }
    /// Feed the window under the gaze (None = no window / inside margin / active window).
    /// Returns the address to focus when the dwell is satisfied.
    pub fn feed(&mut self, hit: Option<&str>, now: Instant, cfg: &DwellCfg) -> Option<String> {
        match (&mut self.cand, hit) {
            (Some(c), Some(a)) if c.addr == a => {
                c.hits += 1;
                c.total += 1;
                c.last_hit = now;
            }
            (Some(c), _) => {
                c.total += 1;
                if now.duration_since(c.last_hit) > cfg.max_gap {
                    self.cand = None;
                }
            }
            (None, _) => {}
        }
        if self.cand.is_none() {
            let a = hit?;
            self.cand = Some(Cand {
                addr: a.to_string(),
                start: now,
                last_hit: now,
                hits: 1,
                total: 1,
            });
        }
        let c = self.cand.as_ref()?;
        if now.duration_since(c.start) >= cfg.dwell
            && c.last_hit == now
            && c.hits as f64 >= cfg.majority * c.total as f64
        {
            return self.cand.take().map(|c| c.addr);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::{FilterConfig, GazeFilter};

    const DT: f64 = 1.0 / 33.0;
    fn cfg(ms: u64) -> DwellCfg {
        DwellCfg {
            dwell: Duration::from_millis(ms),
            majority: 0.7,
            max_gap: Duration::from_millis(100),
        }
    }

    /// Tiny deterministic noise (+-amp px).
    fn noise(i: u32, amp: f64) -> f64 {
        let v = (i.wrapping_mul(2654435761) >> 16) % 1000;
        (v as f64 / 500.0 - 1.0) * amp
    }

    /// Gaze moves from window A (centre x=1280) to window B; returns ms from the
    /// first raw sample inside B (past the margin) to the focus decision.
    /// Window B spans x 2560..5120. `jump` is the saccade distance.
    fn sim(new: bool, jump: f64, amp: f64, outlier_every: u32) -> Option<f64> {
        let (b_lo, b_hi, margin) = (2560.0, 5120.0, 80.0);
        let start = Instant::now();
        let mut old_f = GazeFilter::new(FilterConfig::default());
        let mut fp = FocusPoint::new(0.7, 600.0);
        let mut dw = Dwell::default();
        let mut old_cand: Option<Instant> = None;
        let a_x = 2560.0 - 100.0 - jump / 2.0; // gaze in A, `jump` px away from target
        let target = a_x + jump;
        let mut first_in: Option<f64> = None;
        for i in 0..200u32 {
            let t = i as f64 * DT;
            let now = start + Duration::from_secs_f64(t);
            let mut x = if i < 60 { a_x } else { target } + noise(i, amp);
            if outlier_every > 0 && i > 60 && i % outlier_every == 0 {
                x = a_x; // blink/outlier back in A
            }
            if i >= 60 && first_in.is_none() && x >= b_lo + margin {
                first_in = Some(t);
            }
            let inside = |p: f64| p >= b_lo + margin && p <= b_hi - margin;
            if new {
                let (px, _) = fp.update(x, 700.0);
                let hit = inside(px).then_some("b");
                if dw.feed(hit, now, &cfg(200)).is_some() {
                    return Some((t - first_in?) * 1000.0);
                }
            } else {
                let (px, _) = old_f.smooth(x, 700.0, DT);
                if inside(px) {
                    let s = *old_cand.get_or_insert(now);
                    if now.duration_since(s) >= Duration::from_millis(400) {
                        return Some((t - first_in?) * 1000.0);
                    }
                } else {
                    old_cand = None;
                }
            }
        }
        None
    }

    #[test]
    fn latency_old_vs_new() {
        for (jump, amp, out) in [
            (2500.0, 0.0, 0),
            (1500.0, 40.0, 0),
            (500.0, 40.0, 0),
            (400.0, 40.0, 0),
            (1500.0, 40.0, 7),
        ] {
            let o = sim(false, jump, amp, out);
            let n = sim(true, jump, amp, out).expect("new must focus");
            println!("jump {jump} noise {amp} outlier/{out}: old {o:?} ms, new {n:.0} ms");
            assert!(n < 280.0, "new too slow: {n}");
            if let Some(o) = o {
                assert!(n < o);
            }
        }
    }

    #[test]
    fn brief_glance_does_not_switch() {
        let t0 = Instant::now();
        let mut dw = Dwell::default();
        let c = cfg(200);
        // 4 samples (~120 ms) on b, then back to the active window.
        for i in 0..20u32 {
            let now = t0 + Duration::from_secs_f64(i as f64 * DT);
            let hit = (i < 4).then_some("b");
            assert!(dw.feed(hit, now, &c).is_none(), "switched at {i}");
        }
    }

    #[test]
    fn alternating_does_not_switch() {
        let t0 = Instant::now();
        let mut dw = Dwell::default();
        let c = cfg(200);
        for i in 0..60u32 {
            let now = t0 + Duration::from_secs_f64(i as f64 * DT);
            let hit = (i % 2 == 0).then_some("b"); // 50 % < majority
            assert!(dw.feed(hit, now, &c).is_none(), "switched at {i}");
        }
    }

    #[test]
    fn point_snaps_on_saccade() {
        let mut p = FocusPoint::new(0.6, 600.0);
        p.update(100.0, 100.0);
        assert_eq!(p.update(3000.0, 100.0), (3000.0, 100.0));
        let (x, _) = p.update(3100.0, 100.0);
        assert!((x - 3060.0).abs() < 1e-9);
    }
}
