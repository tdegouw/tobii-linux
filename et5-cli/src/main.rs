// SPDX-License-Identifier: GPL-3.0-or-later
use std::time::{Duration, Instant};
use tobii_et5::{Config, Device};

fn usage() -> ! {
    eprintln!("usage: et5-cli [-v] info | stream [secs] | freq list | freq get");
    std::process::exit(2)
}

fn main() {
    if let Err(e) = run() { eprintln!("error: {e}"); std::process::exit(1); }
}

fn run() -> tobii_et5::Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let verbose = args.first().is_some_and(|a| a == "-v");
    if verbose { args.remove(0); }
    let cfg = Config::load_or_init()?;
    let cmd: Vec<&str> = args.iter().map(String::as_str).collect();
    if let ["stream", rest @ ..] = cmd.as_slice() {
        let secs: f64 = rest.first().and_then(|s| s.parse().ok()).unwrap_or(10.0);
        return stream(Device::open_streaming(cfg, verbose)?, secs);
    }
    let mut dev = Device::open(cfg)?;
    dev.verbose = verbose;
    dev.init()?;
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["info"] => {
            for s in dev.unit_info()? { println!("{s}"); }
            println!("frequency: {:?}", dev.frequency()?);
            println!("config: {}", Config::path().display());
            println!("calibration loaded: {}", dev.has_calibration());
        }
        ["freq", "list"] => for f in dev.frequencies()? { println!("{} / {}", f.a, f.b) },
        ["freq", "get"] => println!("{:?}", dev.frequency()?),
        ["freq", "set", ..] => {
            eprintln!("freq set: the setter property is unknown; this firmware lists a single mode");
            eprintln!("(probing 0x067C made the tracker reset). Not implemented.");
            std::process::exit(1);
        }
        _ => usage(),
    }
    Ok(())
}

fn stream(mut dev: Device, secs: f64) -> tobii_et5::Result<()> {
            let t0 = Instant::now();
            let (mut n, mut valid, mut first, mut last) = (0u64, 0u64, None, None);
            let mut gaps: Vec<f64> = vec![];
            let mut prev: Option<Instant> = None;
            while t0.elapsed().as_secs_f64() < secs {
                let Some((at, g)) = dev.recv_gaze(Duration::from_millis(200))? else { continue };
                n += 1;
                if g.left.valid || g.right.valid { valid += 1; }
                first.get_or_insert(g.timestamp_us);
                last = Some(g.timestamp_us);
                if let Some(p) = prev { gaps.push(at.duration_since(p).as_secs_f64() * 1000.0); }
                prev = Some(at);
                if n % 33 == 1 {
                    let c = g.combined.unwrap_or([f64::NAN; 2]);
                    let px = if g.left.valid || g.right.valid { let (x, y) = dev.config().to_pixels(c); format!("({x:4},{y:4})px") } else { "  (no eyes)  ".into() };
                    println!("{:6.2}s  {px}  L=({:.3},{:.3}) R=({:.3},{:.3}) C=({:.3},{:.3}) eyeZ={:.0}mm",
                        t0.elapsed().as_secs_f64(), g.left.gaze[0], g.left.gaze[1], g.right.gaze[0], g.right.gaze[1],
                        c[0], c[1], g.left.origin[2] / 1000.0);
                }
            }
            gaps.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let med = gaps.get(gaps.len() / 2).copied().unwrap_or(0.0);
            let dev_span = match (first, last) { (Some(a), Some(b)) if b > a => (b - a) as f64 / 1e6, _ => 0.0 };
            println!("samples: {n} in {secs:.1}s = {:.1} Hz (host), valid {valid}; median gap {med:.1} ms; device ts span {dev_span:.2}s",
                n as f64 / secs);
        
    Ok(())
}
