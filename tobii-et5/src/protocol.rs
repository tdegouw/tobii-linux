// SPDX-License-Identifier: GPL-3.0-or-later
//! Constants (from nottobii's protocol.rs, verified on IS50F hardware).
pub const VENDOR_ID: u16 = 0x2104;
pub const PRODUCT_ID: u16 = 0x0313;
pub const EP_OUT: u8 = 0x05;
pub const EP_IN: u8 = 0x83;
pub const GAZE_RANGE: f64 = 1024.0;
pub const TRANSFER_DATA_SIZE: usize = 4087;

pub const TYPE_REQ: u32 = 0x51;
pub const TYPE_RESP: u32 = 0x52;
pub const TYPE_SUB: u32 = 0x53;
pub const TYPE_NOTIF: u32 = 0x4E;

pub const STREAM_GAZE: u64 = 0x0500;
pub const STREAM_PRESENCE: u64 = 0x0504;

pub const UPGRADE: u64 = 0x03E8;
pub const CALIBRATE_START: u64 = 0x03F2;
pub const CALIBRATE_APPLY: u64 = 0x03FC;
pub const CALIBRATE_SHOW_POINT: u64 = 0x0406;
pub const CALIBRATE_CLEAR: u64 = 0x0424;
pub const CALIBRATE_COMPUTE: u64 = 0x042E;
pub const CALIBRATE_GET_STATUS: u64 = 0x044C;
pub const CALIBRATE_UPLOAD: u64 = 0x0456;
pub const CALIBRATE_STOP: u64 = 0x0460;
pub const STREAM_SUBSCRIBE: u64 = 0x04C4;
pub const STREAM_UNSUBSCRIBE: u64 = 0x04CE;
pub const UNIT_INFO_GET: u64 = 0x058C;
pub const CONFIG_3D_SET: u64 = 0x05A0;
pub const STATUS_GET: u64 = 0x05D2;
pub const FREQUENCIES_GET: u64 = 0x0672;
pub const FREQUENCIES_LIST: u64 = 0x0686;
pub const AUTH_CHALLENGE: u64 = 0x076C;
pub const AUTH_VERIFY: u64 = 0x0777;
pub const AUTH_FINALIZE: u64 = 0x077B;
pub const MOUNTING_CONFIG: u64 = 0x0848;
pub const PAUSE_SET: u64 = 0x0C1C;
pub const ENABLED_EYE_SET: u64 = 0x0C58;

pub const PROP_RIGHT_EYE_GAZE: u32 = 5;
pub const PROP_LEFT_EYE_GAZE: u32 = 11;
pub const PROP_COMBINED_GAZE: u32 = 28;
pub const PROP_COMBINED_GAZE_ALT: u32 = 32;
pub const PROP_RIGHT_EYE_3D: u32 = 2;
pub const PROP_LEFT_EYE_3D: u32 = 8;

pub const AUTH_TOKEN: u32 = 10002;
pub const AUTH_HASH: &[u8] = &[
    0x00, 0x00, 0x00, 0x10, 0x0c, 0x48, 0x94, 0x83, 0x80, 0xfa, 0x71, 0xeb, 0xcc, 0x6c, 0xee, 0x6b,
    0xe7, 0xfb, 0x78, 0x28,
];
pub const AUTH_SIG: &[u8] = &[0x00, 0x00, 0x00, 0x04, 0x81, 0x6d, 0x4d, 0xf2];
pub const UPGRADE_STREAMS: &[u32] = &[
    0x10000, 0x10001, 0x10002, 0x10003, 0x10004, 0x10005, 0x10006, 0x10007, 0x10008,
];

pub const SUB_ID_POINT3D: u32 = 0x0003_1f41;
pub const SUB_ID_GAZE_2D: u32 = 0x0002_1f40;
pub const SUB_ID_PROP_SELECTOR: u32 = 0x0002_0bb9;
pub const SUB_ID_DISPLAY_TAG: u32 = 0x0001_0100;
pub const SUB_ID_FREQUENCY: u32 = 0x0002_2af8;
