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
    if m.msg_type != TYPE_SUB || m.prop != STREAM_GAZE {
        return None;
    }
    #[derive(PartialEq)]
    enum Cur {
        Sel,
        G2,
        P3,
        Other,
    }
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
                if let Some(q) = be_q32(v) {
                    vals.push(q);
                }
                if let Some(p) = prop {
                    if cur == Cur::G2 && vals.len() == 2 {
                        g2.insert(p, [vals[0] / GAZE_RANGE, vals[1] / GAZE_RANGE]);
                    }
                    if cur == Cur::P3 && vals.len() == 3 {
                        p3.insert(p, [vals[0], vals[1], vals[2]]);
                    }
                }
            }
            _ => {}
        }
    }
    let eye = |g, o| match g2.get(&g) {
        Some(&gz) => EyeData {
            valid: gz[0] > 0.0 || gz[1] > 0.0,
            gaze: gz,
            origin: p3.get(&o).copied().unwrap_or_default(),
        },
        None => EyeData::default(),
    };
    Some(GazeFrame {
        timestamp_us: ts,
        left: eye(PROP_LEFT_EYE_GAZE, PROP_LEFT_EYE_3D),
        right: eye(PROP_RIGHT_EYE_GAZE, PROP_RIGHT_EYE_3D),
        combined: g2
            .get(&PROP_COMBINED_GAZE)
            .or(g2.get(&PROP_COMBINED_GAZE_ALT))
            .copied(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tlv::{int32, q32_32, sub_id, Deframer};

    fn u64_tlv(v: u64) -> Vec<u8> {
        [vec![T_U64, 0, 0, 0, 8], v.to_be_bytes().to_vec()].concat()
    }
    fn gaze2d(prop: u32, x: f64, y: f64) -> Vec<u8> {
        [
            sub_id(SUB_ID_PROP_SELECTOR),
            int32(prop),
            sub_id(SUB_ID_GAZE_2D),
            q32_32(x),
            q32_32(y),
        ]
        .concat()
    }
    fn point(prop: u32, p: [f64; 3]) -> Vec<u8> {
        [
            sub_id(SUB_ID_PROP_SELECTOR),
            int32(prop),
            sub_id(SUB_ID_POINT3D),
            q32_32(p[0]),
            q32_32(p[1]),
            q32_32(p[2]),
        ]
        .concat()
    }
    fn msg(tlvs: &[u8]) -> Message {
        // SUB frames carry 0 in the upper property word
        let inner = [&[0u8, 0][..], tlvs].concat();
        let mut c = Vec::new();
        for v in [TYPE_SUB, 0, 0, STREAM_GAZE as u32, 0, inner.len() as u32] {
            c.extend(v.to_be_bytes());
        }
        c.extend(inner);
        let mut f = 1u32.to_le_bytes().to_vec();
        f.extend(((c.len() + 8) as u32).to_le_bytes());
        f.extend(c);
        let mut out = vec![];
        Deframer::default().push(&f, &mut out);
        out.remove(0)
    }

    #[test]
    fn parses_both_eyes_and_combined() {
        let t = [
            u64_tlv(123_456),
            gaze2d(PROP_LEFT_EYE_GAZE, 512.0, 256.0),
            point(PROP_LEFT_EYE_3D, [30_000.0, -20_000.0, 800_000.0]),
            gaze2d(PROP_RIGHT_EYE_GAZE, 768.0, 512.0),
            gaze2d(PROP_COMBINED_GAZE, 640.0, 384.0),
        ]
        .concat();
        let g = parse(&msg(&t)).unwrap();
        assert_eq!(g.timestamp_us, 123_456);
        assert!(g.left.valid && g.right.valid);
        assert_eq!(g.left.gaze, [0.5, 0.25]);
        assert_eq!(g.left.origin, [30_000.0, -20_000.0, 800_000.0]);
        assert_eq!(g.right.gaze, [0.75, 0.5]);
        assert_eq!(g.combined, Some([0.625, 0.375]));
    }

    #[test]
    fn no_eyes_is_invalid() {
        let g = parse(&msg(&[
            gaze2d(PROP_LEFT_EYE_GAZE, 0.0, 0.0),
            gaze2d(PROP_COMBINED_GAZE, -1024.0, -1024.0),
        ]
        .concat()))
        .unwrap();
        assert!(!g.left.valid && !g.right.valid);
    }

    #[test]
    fn ignores_other_streams() {
        let mut m = msg(&gaze2d(PROP_LEFT_EYE_GAZE, 1.0, 1.0));
        m.prop = STREAM_PRESENCE;
        assert!(parse(&m).is_none());
    }
}
