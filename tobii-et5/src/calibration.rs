// SPDX-License-Identifier: GPL-3.0-or-later
//! Calibration commands (phase 2; not yet exercised on hardware).
//!
//! Plan (sequence from nottobii's `Device::calibrate`):
//! 1. A layer-shell overlay (separate crate `et5-calibrate`, smithay-client-toolkit)
//!    covers the 5120x1440 output and draws one target at a time.
//! 2. `start()`, `clear()`, optionally upload the current blob.
//! 3. Rounds of points `[[0], [1,2,3], [4,5,6]]`; for each point show the target,
//!    let the eye settle ~0.5 s, `show_point(x, y)` (normalised 0..1), dwell ~1 s;
//!    `compute()` after each round.
//! 4. `retrieve()` the blob (CALIBRATE_GET_STATUS response, may span transfers —
//!    the Deframer reassembles it), `apply()`, `stop()`, save to
//!    `~/.config/tobii-et5/calibration.bin`; `Device::init` uploads it on start.
use crate::{device::Device, protocol::*, tlv, Result};

/// Default 7 calibration points (normalised), centre first.
pub const POINTS: [(f64, f64); 7] = [
    (0.5, 0.5), (0.1, 0.1), (0.9, 0.1), (0.5, 0.9), (0.1, 0.9), (0.9, 0.9), (0.5, 0.1),
];
pub const ROUNDS: [&[usize]; 3] = [&[0], &[1, 2, 3], &[4, 5, 6]];

impl Device {
    pub fn calibration_start(&mut self) -> Result<()> { self.raw_request(CALIBRATE_START, &[]).map(drop) }
    pub fn calibration_stop(&mut self) -> Result<()> { self.raw_request(CALIBRATE_STOP, &[]).map(drop) }
    pub fn calibration_clear(&mut self) -> Result<()> { self.raw_request(CALIBRATE_CLEAR, &[]).map(drop) }
    pub fn calibration_compute(&mut self) -> Result<()> { self.raw_request(CALIBRATE_COMPUTE, &[]).map(drop) }
    pub fn calibration_apply(&mut self) -> Result<()> { self.raw_request(CALIBRATE_APPLY, &[]).map(drop) }
    pub fn calibration_show_point(&mut self, x: f64, y: f64) -> Result<()> {
        let a = [tlv::q32_32(x * GAZE_RANGE), tlv::q32_32(y * GAZE_RANGE), tlv::int32(3)].concat();
        self.raw_request(CALIBRATE_SHOW_POINT, &a).map(drop)
    }
    /// Returns the calibration blob from the device.
    pub fn calibration_retrieve(&mut self) -> Result<Vec<u8>> {
        let m = self.raw_request(CALIBRATE_GET_STATUS, &[])?;
        Ok(m.raw.get(26..).unwrap_or_default().to_vec())
    }
}
