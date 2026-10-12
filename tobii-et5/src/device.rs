// SPDX-License-Identifier: GPL-3.0-or-later
//! High-level device: init/auth/subscribe, request/response, gaze stream.
//! Init sequence follows nottobii's `run_init_sequence`, minus calibration blob
//! uploads when no blob is present (the device rejects an empty blob, 0x20000503).
use crate::{
    config::Config,
    gaze,
    protocol::*,
    tlv::{self, Message},
    usb, Error, GazeFrame, Result,
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};

/// One entry from FREQUENCIES_GET / FREQUENCIES_LIST (struct 0x22af8).
/// Observed on IS50F fw: a single mode `{132, 33}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frequency {
    pub a: u32,
    pub b: u32,
}

pub struct Device {
    h: usb::Handle,
    seq: u32,
    resp_rx: mpsc::Receiver<Message>,
    gaze_rx: mpsc::Receiver<(Instant, GazeFrame)>,
    stop: Arc<AtomicBool>,
    reader: Option<thread::JoinHandle<()>>,
    config: Config,
    calib_blob: Option<Vec<u8>>,
    pending: Option<(Instant, GazeFrame)>,
    serial: String,
    pub verbose: bool,
}

impl Device {
    pub fn open(config: Config) -> Result<Self> {
        let (h, serial) = usb::open()?;
        let (resp_tx, resp_rx) = mpsc::channel();
        let (gaze_tx, gaze_rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let (hr, st) = (h.clone(), stop.clone());
        // Dedicated reader: big buffer, every framed message per transfer decoded.
        let debug = std::env::var_os("ET5_DEBUG").is_some();
        let reader = thread::spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            let mut df = tlv::Deframer::default();
            let mut msgs = Vec::new();
            while !st.load(Ordering::Relaxed) {
                match hr.read_bulk(EP_IN, &mut buf, Duration::from_millis(200)) {
                    Ok(n) => {
                        let now = Instant::now();
                        df.push(&buf[..n], &mut msgs);
                        if debug {
                            eprintln!(
                                "  rx {n} B -> {:?}",
                                msgs.iter()
                                    .map(|m| (m.msg_type, m.prop))
                                    .collect::<Vec<_>>()
                            );
                        }
                        for m in msgs.drain(..) {
                            match m.msg_type {
                                TYPE_RESP => {
                                    let _ = resp_tx.send(m);
                                }
                                TYPE_SUB => {
                                    if let Some(g) = gaze::parse(&m) {
                                        let _ = gaze_tx.send((now, g));
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    Err(rusb::Error::Timeout) => {}
                    Err(e) => {
                        if debug {
                            eprintln!("  rx error {e}");
                        }
                        break;
                    }
                }
            }
        });
        let calib_blob = calib_path(&serial)
            .and_then(|p| std::fs::read(p).ok())
            .filter(|b| !b.is_empty());
        Ok(Device {
            h,
            seq: 0,
            resp_rx,
            gaze_rx,
            stop,
            reader: Some(reader),
            config,
            calib_blob,
            pending: None,
            serial,
            verbose: false,
        })
    }

    fn send(&mut self, prop: u64, args: &[u8]) -> Result<u32> {
        self.seq += 1;
        for t in tlv::transfers(&tlv::content(prop, self.seq, args)) {
            usb::write(&self.h, &t)?;
        }
        Ok(self.seq)
    }

    /// Send a request and wait for its response (matched by seq).
    pub fn request(&mut self, prop: u64, args: &[u8], timeout: Duration) -> Result<Message> {
        let seq = self.send(prop, args)?;
        let end = Instant::now() + timeout;
        loop {
            let left = end.saturating_duration_since(Instant::now());
            match self.resp_rx.recv_timeout(left) {
                Ok(m) if m.seq == seq => {
                    if self.verbose {
                        eprintln!(
                            "  0x{prop:04x} -> status 0x{:08x} ({} B)",
                            m.status,
                            m.raw.len()
                        );
                    }
                    return Ok(m);
                }
                Ok(_) => continue,
                Err(mpsc::RecvTimeoutError::Timeout) => return Err(Error::Timeout("response")),
                Err(_) => return Err(Error::Disconnected),
            }
        }
    }
    fn req(&mut self, prop: u64, args: &[u8]) -> Result<Message> {
        self.request(prop, args, Duration::from_secs(3))
    }
    fn req_ok(&mut self, prop: u64, args: &[u8]) -> Result<Message> {
        let m = self.req(prop, args)?;
        if m.status != 0 {
            return Err(Error::Status {
                prop,
                status: m.status,
            });
        }
        Ok(m)
    }

    fn display_tlv(&self) -> Vec<u8> {
        let mut v: Vec<u8> = self
            .config
            .corners_um()
            .iter()
            .flat_map(|p| tlv::point3d(p[0], p[1], p[2]))
            .collect();
        v.extend(tlv::sub_id(SUB_ID_DISPLAY_TAG));
        v.extend(tlv::int32(12345));
        v
    }

    pub fn upload_calibration(&mut self, blob: &[u8]) -> Result<Message> {
        self.req(CALIBRATE_UPLOAD, &tlv::binary(blob))
    }

    /// Handshake, auth and display setup. Does not start streaming.
    pub fn init(&mut self) -> Result<()> {
        self.req(UPGRADE, &tlv::array_int32(UPGRADE_STREAMS))?;
        if let Some(b) = self.calib_blob.clone() {
            self.upload_calibration(&b)?;
        }
        let d = self.display_tlv();
        self.req(CONFIG_3D_SET, &d)?;
        let a: Vec<u8> = [tlv::int32(AUTH_TOKEN), tlv::array_int32(&[0])].concat();
        self.req(AUTH_CHALLENGE, &a)?;
        // AUTH_VERIFY answers 0x20000505 on this firmware but auth still succeeds.
        let a: Vec<u8> = [
            tlv::int32(AUTH_TOKEN),
            tlv::int32(0),
            tlv::binary(AUTH_HASH),
            tlv::binary(AUTH_SIG),
        ]
        .concat();
        self.req(AUTH_VERIFY, &a)?;
        self.req(MOUNTING_CONFIG, &[])?;
        self.req(AUTH_FINALIZE, &tlv::int32(AUTH_TOKEN))?;
        self.req(STATUS_GET, &[])?;
        self.req(PAUSE_SET, &tlv::int32(1))?;
        self.req(ENABLED_EYE_SET, &tlv::int32(3))?;
        if let Some(b) = self.calib_blob.clone() {
            self.upload_calibration(&b)?;
        }
        Ok(())
    }

    fn subscribe(&mut self) -> Result<()> {
        self.req_ok(
            STREAM_SUBSCRIBE,
            &[tlv::int32(STREAM_GAZE as u32), tlv::array_int32(&[])].concat(),
        )?;
        self.req(
            STREAM_SUBSCRIBE,
            &[tlv::int32(STREAM_PRESENCE as u32), tlv::array_int32(&[])].concat(),
        )?;
        self.req(PAUSE_SET, &tlv::int32(0))?;
        let d = self.display_tlv();
        self.req(CONFIG_3D_SET, &d)?;
        Ok(())
    }

    /// Subscribe to gaze and wait (up to 1.5 s) for the first packet.
    pub fn start_gaze(&mut self) -> Result<()> {
        self.subscribe()?;
        match self.gaze_rx.recv_timeout(Duration::from_millis(1500)) {
            Ok(g) => {
                self.pending = Some(g);
                Ok(())
            }
            Err(_) => Err(Error::Timeout("gaze stream")),
        }
    }

    /// Open and init, reopening the device on failure. On this firmware roughly
    /// every other session misbehaves (a request such as PAUSE_SET goes unanswered,
    /// or the gaze stream stays silent); a fresh open fixes it.
    pub fn open_ready(config: Config, verbose: bool) -> Result<Self> {
        Self::open_retry(config, verbose, false)
    }

    /// Open, init and start gaze streaming, with the same reopen-on-failure logic.
    pub fn open_streaming(config: Config, verbose: bool) -> Result<Self> {
        Self::open_retry(config, verbose, true)
    }

    fn open_retry(config: Config, verbose: bool, stream: bool) -> Result<Self> {
        let mut last = Error::NoDevice;
        for attempt in 0..4 {
            let r = Device::open(config.clone()).and_then(|mut d| {
                d.verbose = verbose;
                d.init()?;
                if stream {
                    d.start_gaze()?;
                }
                Ok(d)
            });
            match r {
                Ok(d) => return Ok(d),
                Err(Error::NoDevice) => return Err(Error::NoDevice),
                Err(e) => {
                    if verbose {
                        eprintln!("  open attempt {attempt} failed: {e}; reopening");
                    }
                    last = e;
                }
            }
        }
        Err(last)
    }

    pub fn recv_gaze(&mut self, timeout: Duration) -> Result<Option<(Instant, GazeFrame)>> {
        if let Some(g) = self.pending.take() {
            return Ok(Some(g));
        }
        match self.gaze_rx.recv_timeout(timeout) {
            Ok(g) => Ok(Some(g)),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(_) => Err(Error::Disconnected),
        }
    }

    fn parse_freqs(m: &Message) -> Vec<Frequency> {
        let mut out = Vec::new();
        let e = m.entries();
        for (i, (t, v)) in e.iter().enumerate() {
            if *t == tlv::T_SUB_ID && tlv::be_u32(v) == Some(SUB_ID_FREQUENCY) {
                if let (Some(a), Some(b)) = (
                    e.get(i + 1).and_then(|x| tlv::be_u32(x.1)),
                    e.get(i + 2).and_then(|x| tlv::be_u32(x.1)),
                ) {
                    out.push(Frequency { a, b });
                }
            }
        }
        out
    }
    pub fn frequency(&mut self) -> Result<Option<Frequency>> {
        Ok(Self::parse_freqs(&self.req_ok(FREQUENCIES_GET, &[])?)
            .first()
            .copied())
    }
    pub fn frequencies(&mut self) -> Result<Vec<Frequency>> {
        Ok(Self::parse_freqs(&self.req_ok(FREQUENCIES_LIST, &[])?))
    }

    pub fn unit_info(&mut self) -> Result<Vec<String>> {
        let m = self.req_ok(UNIT_INFO_GET, &[])?;
        Ok(m.entries()
            .iter()
            .filter(|(t, v)| *t == tlv::T_STRING && v.len() >= 4)
            .map(|(_, v)| String::from_utf8_lossy(&v[4..]).into_owned())
            .collect())
    }

    pub(crate) fn raw_request(&mut self, prop: u64, args: &[u8]) -> Result<Message> {
        self.req(prop, args)
    }
    pub(crate) fn raw_request_t(&mut self, prop: u64, args: &[u8], t: Duration) -> Result<Message> {
        self.request(prop, args, t)
    }
    /// USB serial number, e.g. `IS50F-100200242231`.
    pub fn serial(&self) -> &str {
        &self.serial
    }
    pub fn config(&self) -> &Config {
        &self.config
    }
    /// Whether a stored calibration blob was found (and is uploaded by `init`).
    pub fn has_calibration(&self) -> bool {
        self.calib_blob.is_some()
    }
}

/// `~/.config/tobii-et5/<serial>/calibration.bin`
pub fn calib_path(serial: &str) -> Option<std::path::PathBuf> {
    Config::path()
        .parent()
        .map(|d| d.join(serial).join("calibration.bin"))
}

impl Drop for Device {
    fn drop(&mut self) {
        // Pause + unsubscribe so the next session starts from a clean state
        // (otherwise every other session got no gaze packets).
        let _ = self.request(PAUSE_SET, &tlv::int32(1), Duration::from_millis(500));
        let _ = self.request(
            STREAM_UNSUBSCRIBE,
            &tlv::int32(STREAM_GAZE as u32),
            Duration::from_millis(500),
        );
        let _ = self.request(
            STREAM_UNSUBSCRIBE,
            &tlv::int32(STREAM_PRESENCE as u32),
            Duration::from_millis(500),
        );
        self.stop.store(true, Ordering::Relaxed);
        if let Some(r) = self.reader.take() {
            let _ = r.join();
        }
        let _ = self.h.release_interface(0);
    }
}
