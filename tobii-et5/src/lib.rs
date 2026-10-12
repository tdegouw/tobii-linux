// SPDX-License-Identifier: GPL-3.0-or-later
//! Tobii Eye Tracker 5 userspace driver.
//!
//! Protocol knowledge derived from nottobii (https://github.com/puffnfresh/nottobii, GPL-3)
//! and tobiifree (https://github.com/Aetherall/tobiifree, GPL-3).
pub mod calibration;
pub mod config;
pub mod device;
pub mod gaze;
pub mod protocol;
pub mod tlv;
pub mod usb;

pub use config::Config;
pub use device::{Device, Frequency};
pub use gaze::{EyeData, GazeFrame};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no Tobii ET5 (2104:0313) found")]
    NoDevice,
    #[error("usb: {0}")]
    Usb(#[from] rusb::Error),
    #[error("timeout waiting for {0}")]
    Timeout(&'static str),
    #[error("device returned status 0x{status:08x} for prop 0x{prop:04x}")]
    Status { prop: u64, status: u32 },
    #[error("config: {0}")]
    Config(String),
    #[error("reader thread stopped")]
    Disconnected,
}
pub type Result<T> = std::result::Result<T, Error>;
