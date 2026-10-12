// SPDX-License-Identifier: GPL-3.0-or-later
//! TLV encoding and message framing. Wire format:
//! `msg_id(LE32) len(LE32) | type(BE32) seq(BE32) prop(BE64) 0(BE32) inner_len(BE32) [00 00 TLV...]`
use crate::protocol::*;

pub const T_INT32: u8 = 0x02;
pub const T_Q16_16: u8 = 0x03;
pub const T_Q32_32: u8 = 0x04;
pub const T_SUB_ID: u8 = 0x05;
pub const T_U64: u8 = 0x06;
pub const T_STRING: u8 = 0x14;
pub const T_BINARY: u8 = 0x15;
pub const T_ARRAY_INT32: u8 = 0x17;

fn tlv(t: u8, payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(5 + payload.len());
    v.push(t);
    v.extend((payload.len() as u32).to_be_bytes());
    v.extend(payload);
    v
}
pub fn int32(v: u32) -> Vec<u8> {
    tlv(T_INT32, &v.to_be_bytes())
}
pub fn sub_id(v: u32) -> Vec<u8> {
    tlv(T_SUB_ID, &v.to_be_bytes())
}
pub fn binary(d: &[u8]) -> Vec<u8> {
    tlv(T_BINARY, d)
}
pub fn q32_32(f: f64) -> Vec<u8> {
    tlv(T_Q32_32, &((f * 4294967296.0) as i64).to_be_bytes())
}
pub fn array_int32(xs: &[u32]) -> Vec<u8> {
    let mut p = (xs.len() as u32).to_be_bytes().to_vec();
    for x in xs {
        p.extend(x.to_be_bytes());
    }
    tlv(T_ARRAY_INT32, &p)
}
pub fn point3d(x: f64, y: f64, z: f64) -> Vec<u8> {
    [sub_id(SUB_ID_POINT3D), q32_32(x), q32_32(y), q32_32(z)].concat()
}

/// Message content (without the 8-byte LE transfer header).
pub fn content(prop: u64, seq: u32, args: &[u8]) -> Vec<u8> {
    let inner: Vec<u8> = if args.is_empty() {
        vec![]
    } else {
        [&[0u8, 0][..], args].concat()
    };
    let mut c = Vec::with_capacity(24 + inner.len());
    c.extend(TYPE_REQ.to_be_bytes());
    c.extend(seq.to_be_bytes());
    c.extend(prop.to_be_bytes());
    c.extend(0u32.to_be_bytes());
    c.extend((inner.len() as u32).to_be_bytes());
    c.extend(inner);
    c
}

/// Split content into USB transfers, each with its own `0, len` LE header.
pub fn transfers(content: &[u8]) -> Vec<Vec<u8>> {
    content
        .chunks(TRANSFER_DATA_SIZE)
        .map(|ch| {
            let mut t = Vec::with_capacity(8 + ch.len());
            t.extend(0u32.to_le_bytes());
            t.extend((ch.len() as u32).to_le_bytes());
            t.extend(ch);
            t
        })
        .collect()
}

pub type Entry<'a> = (u8, &'a [u8]);

pub fn parse_entries(d: &[u8], mut pos: usize) -> Vec<Entry<'_>> {
    let mut out = Vec::new();
    while pos + 5 <= d.len() {
        let t = d[pos];
        let n = u32::from_be_bytes(d[pos + 1..pos + 5].try_into().unwrap()) as usize;
        pos += 5;
        if n > 1 << 20 || pos + n > d.len() {
            break;
        }
        out.push((t, &d[pos..pos + n]));
        pos += n;
    }
    out
}

pub fn be_u32(v: &[u8]) -> Option<u32> {
    v.try_into().ok().map(u32::from_be_bytes)
}
pub fn be_q32(v: &[u8]) -> Option<f64> {
    v.try_into()
        .ok()
        .map(|b| i64::from_be_bytes(b) as f64 / 4294967296.0)
}

/// A decoded device message (one per framed message; a USB transfer may hold several).
#[derive(Debug, Clone)]
pub struct Message {
    pub msg_type: u32,
    pub seq: u32,
    pub prop: u64,
    /// For responses: status word (0 = OK).
    pub status: u32,
    /// Raw content starting at the type field (offset 0 = old offset 8).
    pub raw: Vec<u8>,
}
impl Message {
    pub fn entries(&self) -> Vec<Entry<'_>> {
        parse_entries(&self.raw, 26)
    }
}

/// Splits a byte stream of `LE32 id, LE32 total_len, content` frames into messages.
/// Handles several messages per transfer and messages spanning transfers.
#[derive(Default)]
pub struct Deframer {
    buf: Vec<u8>,
    pending: Option<(usize, Vec<u8>)>,
}
impl Deframer {
    pub fn push(&mut self, data: &[u8], out: &mut Vec<Message>) {
        self.buf.extend_from_slice(data);
        let mut pos = 0;
        while self.buf.len() - pos >= 8 {
            // Inbound frames: LE32 id, LE32 total length *including* this 8-byte header.
            let total = u32::from_le_bytes(self.buf[pos + 4..pos + 8].try_into().unwrap()) as usize;
            if total < 8 {
                self.buf.clear();
                return;
            }
            if self.buf.len() - pos < total {
                break;
            }
            let chunk = &self.buf[pos + 8..pos + total];
            pos += total;
            // continuation of a message larger than one transfer
            if let Some((want, mut acc)) = self.pending.take() {
                acc.extend_from_slice(chunk);
                if acc.len() >= want {
                    out.extend(Self::decode(acc));
                } else {
                    self.pending = Some((want, acc));
                }
                continue;
            }
            if chunk.len() >= 24 {
                let want = 24 + u32::from_be_bytes(chunk[20..24].try_into().unwrap()) as usize;
                if chunk.len() < want {
                    self.pending = Some((want, chunk.to_vec()));
                    continue;
                }
            }
            out.extend(Self::decode(chunk.to_vec()));
        }
        self.buf.drain(..pos);
    }
    fn decode(raw: Vec<u8>) -> Option<Message> {
        if raw.len() < 24 {
            return None;
        }
        let g = |a: usize| u32::from_be_bytes(raw[a..a + 4].try_into().unwrap());
        Some(Message {
            msg_type: g(0),
            seq: g(4),
            // responses carry 0x00000001 in the upper word; the property id is the low 32 bits
            prop: g(12) as u64,
            status: g(16),
            raw,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_primitives() {
        assert_eq!(int32(0x1234), [2, 0, 0, 0, 4, 0, 0, 0x12, 0x34]);
        assert_eq!(
            array_int32(&[1, 2]),
            [0x17, 0, 0, 0, 12, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 2]
        );
        assert_eq!(binary(b"ab"), [0x15, 0, 0, 0, 2, b'a', b'b']);
        let q = q32_32(1.5);
        assert_eq!(q[0], T_Q32_32);
        assert_eq!(be_q32(&q[5..]), Some(1.5));
        assert_eq!(be_q32(&q32_32(-512.25)[5..]), Some(-512.25));
        assert_eq!(point3d(0.0, 0.0, 0.0).len(), 9 + 3 * 13);
    }

    #[test]
    fn content_layout() {
        let c = content(0x076C, 7, &int32(10002));
        assert_eq!(&c[0..4], &TYPE_REQ.to_be_bytes());
        assert_eq!(&c[4..8], &7u32.to_be_bytes());
        assert_eq!(&c[8..16], &0x076Cu64.to_be_bytes());
        assert_eq!(&c[20..24], &11u32.to_be_bytes()); // 2 pad + 9 TLV
        assert_eq!(&c[24..26], &[0, 0]);
        assert_eq!(content(0x05D2, 1, &[]).len(), 24); // no args: no padding
    }

    #[test]
    fn transfers_split() {
        let c = vec![0xAB; TRANSFER_DATA_SIZE * 2 + 10];
        let t = transfers(&c);
        assert_eq!(t.len(), 3);
        assert_eq!(&t[0][4..8], &(TRANSFER_DATA_SIZE as u32).to_le_bytes());
        assert_eq!(&t[2][4..8], &10u32.to_le_bytes());
        assert_eq!(t.iter().map(|x| x.len() - 8).sum::<usize>(), c.len());
    }

    /// Inbound frame: LE id, LE total length incl. header, content.
    pub(crate) fn frame_in(typ: u32, seq: u32, prop: u32, status: u32, tlvs: &[u8]) -> Vec<u8> {
        let inner: Vec<u8> = if tlvs.is_empty() {
            vec![]
        } else {
            [&[0u8, 0][..], tlvs].concat()
        };
        let mut c = Vec::new();
        for v in [typ, seq, 1, prop, status, inner.len() as u32] {
            c.extend(v.to_be_bytes());
        }
        c.extend(inner);
        let mut f = 1u32.to_le_bytes().to_vec();
        f.extend(((c.len() + 8) as u32).to_le_bytes());
        f.extend(c);
        f
    }

    #[test]
    fn deframe_multiple_and_split() {
        let a = frame_in(TYPE_RESP, 1, 0x3E8, 0, &int32(5));
        let b = frame_in(TYPE_RESP, 2, 0x5A0, 0x2000_0505, &[]);
        let mut all = a.clone();
        all.extend(&b);
        let mut d = Deframer::default();
        let mut out = vec![];
        // feed in awkward pieces
        d.push(&all[..5], &mut out);
        d.push(&all[5..a.len() + 3], &mut out);
        assert_eq!(out.len(), 1);
        d.push(&all[a.len() + 3..], &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!((out[0].seq, out[0].prop, out[0].status), (1, 0x3E8, 0));
        assert_eq!(be_u32(out[0].entries()[0].1), Some(5));
        assert_eq!(
            (out[1].seq, out[1].prop, out[1].status),
            (2, 0x5A0, 0x2000_0505)
        );
    }

    #[test]
    fn deframe_message_spanning_transfers() {
        // A big response (like a calibration blob) split into several inbound frames.
        let blob = vec![7u8; 9000];
        let whole = frame_in(TYPE_RESP, 3, 0x44C, 0, &binary(&blob));
        let content = &whole[8..];
        let mut d = Deframer::default();
        let mut out = vec![];
        for ch in content.chunks(4000) {
            let mut f = 1u32.to_le_bytes().to_vec();
            f.extend(((ch.len() + 8) as u32).to_le_bytes());
            f.extend(ch);
            d.push(&f, &mut out);
        }
        assert_eq!(out.len(), 1);
        let e = out[0].entries();
        assert_eq!(e[0].0, T_BINARY);
        assert_eq!(e[0].1.len(), 9000);
    }

    #[test]
    fn parse_entries_truncated() {
        let mut d = int32(1);
        d.extend([2, 0, 0, 0, 4, 0]); // truncated second entry
        assert_eq!(parse_entries(&d, 0).len(), 1);
        assert!(parse_entries(&[1, 2], 0).is_empty());
    }
}
