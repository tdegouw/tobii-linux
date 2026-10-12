// SPDX-License-Identifier: GPL-3.0-or-later
//! Gaze stream parsing (structure after nottobii's gaze.rs).
use crate::{protocol::*, tlv::*};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, Default)]
pub struct EyeData {
    pub valid: bool,
    /// Normalised display coordinates (0..1, origin top-left).
    pub gaze: [f64; 2],
    /// Eye position in µm relative to the tracker.
    pub origin: [f64; 3],
}

#[derive(Debug, Clone, Copy, Default)]
pub struct GazeFrame {
    pub timestamp_us: u64,
    pub left: EyeData,
    pub right: EyeData,
    pub combined: Option<[f64; 2]>,
}

pub fn parse(m: &Message) -> Option<GazeFrame> {
    if m.msg_type != TYPE_SUB || m.prop != STREAM_GAZE { return None; }
    #[derive(PartialEq)]
    enum Cur { Sel, G2, P3, Other }
    let (mut ts, mut prop, mut cur, mut vals) = (0u64, None, Cur::Other, Vec::<f64>::new());
    let mut g2: HashMap<u32, [f64; 2]> = HashMap::new();
    let mut p3: HashMap<u32, [f64; 3]> = HashMap::new();
    for (t, v) in m.entries() {
        match t {
            T_SUB_ID => {
                cur = match be_u32(v) {
                    Some(SUB_ID_PROP_SELECTOR) => Cur::Sel,
                    Some(SUB_ID_GAZE_2D) => Cur::G2,
                    Some(SUB_ID_POINT3D) => Cur::P3,
                    _ => Cur::Other,
                };
                vals.clear();
            }
            T_INT32 if cur == Cur::Sel => prop = be_u32(v),
            T_U64 if v.len() == 8 => ts = u64::from_be_bytes(v.try_into().unwrap()),
            T_Q32_32 => {
                if let Some(q) = be_q32(v) { vals.push(q); }
                if let Some(p) = prop {
                    if cur == Cur::G2 && vals.len() == 2 { g2.insert(p, [vals[0] / GAZE_RANGE, vals[1] / GAZE_RANGE]); }
                    if cur == Cur::P3 && vals.len() == 3 { p3.insert(p, [vals[0], vals[1], vals[2]]); }
                }
            }
            _ => {}
        }
    }
    let eye = |g, o| match g2.get(&g) {
        Some(&gz) => EyeData { valid: gz[0] > 0.0 || gz[1] > 0.0, gaze: gz, origin: p3.get(&o).copied().unwrap_or_default() },
        None => EyeData::default(),
    };
    Some(GazeFrame {
        timestamp_us: ts,
        left: eye(PROP_LEFT_EYE_GAZE, PROP_LEFT_EYE_3D),
        right: eye(PROP_RIGHT_EYE_GAZE, PROP_RIGHT_EYE_3D),
        combined: g2.get(&PROP_COMBINED_GAZE).or(g2.get(&PROP_COMBINED_GAZE_ALT)).copied(),
    })
}
