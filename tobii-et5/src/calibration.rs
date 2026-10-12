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
use std::time::Duration;

/// Default 7 calibration points (normalised), centre first, then two rounds of three.
pub const POINTS: [(f64, f64); 7] = [
    (0.5, 0.5),
    (0.1, 0.1), (0.9, 0.1), (0.5, 0.9),
    (0.1, 0.9), (0.9, 0.9), (0.5, 0.1),
];
pub const ROUNDS: [&[usize]; 3] = [&[0], &[1, 2, 3], &[4, 5, 6]];
const LONG: Duration = Duration::from_secs(10);

impl Device {
    pub fn calibration_start(&mut self) -> Result<()> { self.raw_request(CALIBRATE_START, &[]).map(drop) }
    pub fn calibration_stop(&mut self) -> Result<()> { self.raw_request(CALIBRATE_STOP, &[]).map(drop) }
    pub fn calibration_clear(&mut self) -> Result<()> { self.raw_request(CALIBRATE_CLEAR, &[]).map(drop) }
    pub fn calibration_compute(&mut self) -> Result<u32> { self.raw_request_t(CALIBRATE_COMPUTE, &[], LONG).map(|m| m.status) }
    pub fn calibration_apply(&mut self) -> Result<u32> { self.raw_request_t(CALIBRATE_APPLY, &[], LONG).map(|m| m.status) }
    /// Tell the tracker the user is fixating normalised point (x, y). Returns device status.
    pub fn calibration_show_point(&mut self, x: f64, y: f64) -> Result<u32> {
        let a = [tlv::q32_32(x * GAZE_RANGE), tlv::q32_32(y * GAZE_RANGE), tlv::int32(3)].concat();
        self.raw_request_t(CALIBRATE_SHOW_POINT, &a, LONG).map(|m| m.status)
    }
    /// Fetch the computed calibration blob (CALIBRATE_GET_STATUS). The response may
    /// span several USB transfers; the Deframer reassembles it.
    pub fn calibration_retrieve(&mut self) -> Result<Vec<u8>> {
        let m = self.raw_request_t(CALIBRATE_GET_STATUS, &[], LONG)?;
        if m.status != 0 { return Err(crate::Error::Status { prop: CALIBRATE_GET_STATUS, status: m.status }); }
        // Unwrap a single BINARY TLV if that is what the device sent; otherwise keep raw.
        let e = m.entries();
        if let Some((tlv::T_BINARY, v)) = e.first() { return Ok(v.to_vec()); }
        Ok(m.raw.get(26..).unwrap_or_default().to_vec())
    }
    /// Save the blob to `~/.config/tobii-et5/<serial>/calibration.bin`.
    pub fn save_calibration(&self, blob: &[u8]) -> std::io::Result<std::path::PathBuf> {
        let p = crate::device::calib_path(self.serial()).ok_or(std::io::ErrorKind::NotFound)?;
        if let Some(d) = p.parent() { std::fs::create_dir_all(d)?; }
        std::fs::write(&p, blob)?;
        Ok(p)
    }
}
