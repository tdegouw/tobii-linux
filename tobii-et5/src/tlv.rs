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
pub fn int32(v: u32) -> Vec<u8> { tlv(T_INT32, &v.to_be_bytes()) }
pub fn sub_id(v: u32) -> Vec<u8> { tlv(T_SUB_ID, &v.to_be_bytes()) }
pub fn binary(d: &[u8]) -> Vec<u8> { tlv(T_BINARY, d) }
pub fn q32_32(f: f64) -> Vec<u8> { tlv(T_Q32_32, &((f * 4294967296.0) as i64).to_be_bytes()) }
pub fn array_int32(xs: &[u32]) -> Vec<u8> {
    let mut p = (xs.len() as u32).to_be_bytes().to_vec();
    for x in xs { p.extend(x.to_be_bytes()); }
    tlv(T_ARRAY_INT32, &p)
}
pub fn point3d(x: f64, y: f64, z: f64) -> Vec<u8> {
    [sub_id(SUB_ID_POINT3D), q32_32(x), q32_32(y), q32_32(z)].concat()
}

/// Message content (without the 8-byte LE transfer header).
pub fn content(prop: u64, seq: u32, args: &[u8]) -> Vec<u8> {
    let inner: Vec<u8> = if args.is_empty() { vec![] } else { [&[0u8, 0][..], args].concat() };
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
        if n > 1 << 20 || pos + n > d.len() { break; }
        out.push((t, &d[pos..pos + n]));
        pos += n;
    }
    out
}

pub fn be_u32(v: &[u8]) -> Option<u32> { v.try_into().ok().map(u32::from_be_bytes) }
pub fn be_q32(v: &[u8]) -> Option<f64> {
    v.try_into().ok().map(|b| i64::from_be_bytes(b) as f64 / 4294967296.0)
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
    pub fn entries(&self) -> Vec<Entry<'_>> { parse_entries(&self.raw, 26) }
}

/// Splits a byte stream of `LE32 id, LE32 total_len, content` frames into messages.
/// Handles several messages per transfer and messages spanning transfers.
#[derive(Default)]
pub struct Deframer { buf: Vec<u8>, pending: Option<(usize, Vec<u8>)> }
impl Deframer {
    pub fn push(&mut self, data: &[u8], out: &mut Vec<Message>) {
        self.buf.extend_from_slice(data);
        let mut pos = 0;
        while self.buf.len() - pos >= 8 {
            // Inbound frames: LE32 id, LE32 total length *including* this 8-byte header.
            let total = u32::from_le_bytes(self.buf[pos + 4..pos + 8].try_into().unwrap()) as usize;
            if total < 8 { self.buf.clear(); return; }
            if self.buf.len() - pos < total { break; }
            let chunk = &self.buf[pos + 8..pos + total];
            pos += total;
            // continuation of a message larger than one transfer
            if let Some((want, mut acc)) = self.pending.take() {
                acc.extend_from_slice(chunk);
                if acc.len() >= want { out.extend(Self::decode(acc)); } else { self.pending = Some((want, acc)); }
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
        if raw.len() < 24 { return None; }
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
