// SPDX-License-Identifier: GPL-3.0-or-later
use crate::{protocol::*, Error, Result};
use rusb::{DeviceHandle, GlobalContext};
use std::{sync::Arc, time::Duration};

pub type Handle = Arc<DeviceHandle<GlobalContext>>;

/// Opens the tracker; returns the handle and the USB serial number.
pub fn open() -> Result<(Handle, String)> {
    let h = rusb::open_device_with_vid_pid(VENDOR_ID, PRODUCT_ID).ok_or(Error::NoDevice)?;
    let serial = h.device().device_descriptor().ok()
        .and_then(|d| h.read_serial_number_string_ascii(&d).ok())
        .unwrap_or_else(|| "unknown".into());
    // uvcvideo only binds ifaces 1/2; iface 0 is vendor-specific, detach just in case.
    if h.kernel_driver_active(0).unwrap_or(false) {
        h.detach_kernel_driver(0)?;
    }
    // Only set the configuration if needed: re-setting it while uvcvideo holds
    // ifaces 1/2 makes xhci complain ("add endpoint 0x82") and can wedge the device.
    if h.active_configuration().unwrap_or(0) != 1 { let _ = h.set_active_configuration(1); }
    h.claim_interface(0)?;
    // vendor request that enables the bulk endpoints.
    // NB: do not libusb-reset on failure: that made the tracker drop off the bus.
    h.write_control(0x41, 0x41, 0, 0, &[], Duration::from_secs(2))?;
    Ok((Arc::new(h), serial))
}

pub fn write(h: &Handle, data: &[u8]) -> Result<()> {
    h.write_bulk(EP_OUT, data, Duration::from_secs(5))?;
    Ok(())
}
