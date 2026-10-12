// SPDX-License-Identifier: GPL-3.0-or-later
//! Layer-shell overlay, software-rendered into a wl_shm buffer.
use crate::{Phase, Shared, DOT_TIME};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_registry,
    delegate_seat, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers},
        Capability, SeatHandler, SeatState,
    },
    shell::{wlr_layer::*, WaylandSurface},
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use std::{
    sync::{atomic::Ordering, Arc},
    time::Instant,
};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_seat, wl_shm, wl_surface},
    Connection, QueueHandle,
};

// Tokyo Night
const BG: u32 = 0xff1a1b26;
const BLUE: u32 = 0xff7aa2f7;
const VIOLET: u32 = 0xffbb9af7;
const DIM: u32 = 0xff414868;
const GREEN: u32 = 0xff9ece6a;
const RED: u32 = 0xfff7768e;

pub fn run(shared: Arc<Shared>) -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| e.to_string())?;
    let (globals, mut q) = registry_queue_init(&conn).map_err(|e| e.to_string())?;
    let qh = q.handle();
    let compositor = CompositorState::bind(&globals, &qh).map_err(|e| e.to_string())?;
    let layer_shell = LayerShell::bind(&globals, &qh).map_err(|e| format!("layer-shell: {e}"))?;
    let shm = Shm::bind(&globals, &qh).map_err(|e| e.to_string())?;
    let surface = compositor.create_surface(&qh);
    let layer =
        layer_shell.create_layer_surface(&qh, surface, Layer::Overlay, Some("et5-calibrate"), None);
    layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
    layer.set_exclusive_zone(-1);
    layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
    layer.commit();
    let pool = SlotPool::new(4096 * 4, &shm).map_err(|e| e.to_string())?;
    let mut app = App {
        registry: RegistryState::new(&globals),
        seat: SeatState::new(&globals, &qh),
        output: OutputState::new(&globals, &qh),
        shm,
        pool,
        layer,
        width: 0,
        height: 0,
        configured: false,
        exit: false,
        keyboard: None,
        shared,
        started: Instant::now(),
    };
    while !app.exit {
        q.blocking_dispatch(&mut app).map_err(|e| e.to_string())?;
        if app.shared.finished.load(Ordering::Relaxed) {
            app.exit = true;
        }
    }
    Ok(())
}

struct App {
    registry: RegistryState,
    seat: SeatState,
    output: OutputState,
    shm: Shm,
    pool: SlotPool,
    layer: LayerSurface,
    width: u32,
    height: u32,
    configured: bool,
    exit: bool,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    shared: Arc<Shared>,
    started: Instant,
}

/// Software canvas over the shm buffer.
struct Canvas<'a> {
    buf: &'a mut [u32],
    w: i32,
    h: i32,
}

fn disc(cv: &mut Canvas, cx: i32, cy: i32, r: i32, c: u32) {
    let (buf, w, h) = (&mut *cv.buf, cv.w, cv.h);
    for y in (cy - r).max(0)..(cy + r + 1).min(h) {
        let dy = y - cy;
        let dx = ((r * r - dy * dy).max(0) as f64).sqrt() as i32;
        let (x0, x1) = ((cx - dx).max(0), (cx + dx + 1).min(w));
        if x0 < x1 {
            buf[(y * w + x0) as usize..(y * w + x1) as usize].fill(c);
        }
    }
}
fn ring(cv: &mut Canvas, cx: i32, cy: i32, r: i32, t: i32, c: u32) {
    let (buf, w, h) = (&mut *cv.buf, cv.w, cv.h);
    for y in (cy - r).max(0)..(cy + r + 1).min(h) {
        for x in (cx - r).max(0)..(cx + r + 1).min(w) {
            let d2 = (x - cx).pow(2) + (y - cy).pow(2);
            if d2 <= r * r && d2 >= (r - t).pow(2) {
                buf[(y * w + x) as usize] = c;
            }
        }
    }
}

impl App {
    fn draw(&mut self, qh: &QueueHandle<Self>) {
        if self.exit || !self.configured {
            return;
        }
        let (w, h) = (self.width as i32, self.height as i32);
        let (buffer, canvas) = match self
            .pool
            .create_buffer(w, h, w * 4, wl_shm::Format::Argb8888)
        {
            Ok(b) => b,
            Err(e) => {
                eprintln!("overlay: cannot allocate buffer: {e}");
                self.shared.abort.store(true, Ordering::Relaxed);
                self.exit = true;
                return;
            }
        };
        // The shm slot is page-aligned and sized w*h*4, so viewing it as u32 is sound.
        let (pre, px, post) = unsafe { canvas.align_to_mut::<u32>() };
        if !pre.is_empty() || !post.is_empty() {
            eprintln!("overlay: unaligned shm buffer");
            self.exit = true;
            return;
        }
        px.fill(BG);
        let mut cv = Canvas { buf: px, w, h };
        let now = Instant::now();
        let (cx, cy) = (w / 2, h / 2);
        let phase = self
            .shared
            .phase
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        match phase {
            Phase::Connecting => {
                let t = (now.duration_since(self.started).as_secs_f64() * 3.0).sin() * 0.5 + 0.5;
                disc(&mut cv, cx, cy, 10 + (t * 8.0) as i32, DIM);
            }
            Phase::Countdown { until } => {
                // one small dot per remaining second + big ring that closes in on the centre
                let left = until.saturating_duration_since(now).as_secs_f64();
                for i in 0..left.ceil() as i32 {
                    disc(&mut cv, cx - 40 + i * 40, cy + 120, 8, VIOLET);
                }
                let r = 30 + ((left % 1.0) * 90.0) as i32;
                ring(&mut cv, cx, cy, r, 4, BLUE);
                disc(&mut cv, cx, cy, 8, VIOLET);
            }
            Phase::Point { x, y, start } => {
                let p = (now.duration_since(start).as_secs_f64() / DOT_TIME.as_secs_f64()).min(1.0);
                let (dx, dy) = ((x * w as f64) as i32, (y * h as f64) as i32);
                disc(&mut cv, dx, dy, 8 + ((1.0 - p) * 42.0) as i32, BLUE);
                disc(&mut cv, dx, dy, 5, VIOLET);
            }
            Phase::Computing => disc(&mut cv, cx, cy, 6, DIM),
            Phase::Done { ok, at } => {
                let p = (now.duration_since(at).as_secs_f64() * 2.0).min(1.0);
                ring(
                    &mut cv,
                    cx,
                    cy,
                    30 + (p * 60.0) as i32,
                    6,
                    if ok { GREEN } else { RED },
                );
            }
        }
        let s = self.layer.wl_surface();
        s.damage_buffer(0, 0, w, h);
        s.frame(qh, s.clone());
        if let Err(e) = buffer.attach_to(s) {
            eprintln!("overlay: attach failed: {e}");
            return;
        }
        self.layer.commit();
    }
}

impl CompositorHandler for App {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(&mut self, _: &Connection, qh: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {
        self.draw(qh);
    }
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}
impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}
impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.shared.abort.store(true, Ordering::Relaxed);
        self.exit = true;
    }
    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &LayerSurface,
        c: LayerSurfaceConfigure,
        _: u32,
    ) {
        let (w, h) = c.new_size;
        self.width = if w == 0 { 5120 } else { w };
        self.height = if h == 0 { 1440 } else { h };
        eprintln!("overlay configured {}x{}", self.width, self.height);
        let first = !self.configured;
        self.configured = true;
        if first {
            self.draw(qh);
        }
    }
}
impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        c: Capability,
    ) {
        if c == Capability::Keyboard && self.keyboard.is_none() {
            self.keyboard = self.seat.get_keyboard(qh, &seat, None).ok();
        }
    }
    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        c: Capability,
    ) {
        if c == Capability::Keyboard {
            if let Some(k) = self.keyboard.take() {
                k.release();
            }
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}
impl KeyboardHandler for App {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
    }
    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
    ) {
    }
    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        e: KeyEvent,
    ) {
        if e.keysym == Keysym::Escape || e.keysym == Keysym::q {
            eprintln!("Esc: aborting");
            self.shared.abort.store(true, Ordering::Relaxed);
            self.exit = true;
        }
    }
    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: KeyEvent,
    ) {
    }
    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: Modifiers,
        _: u32,
    ) {
    }
}
impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}
impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState, SeatState];
}
delegate_compositor!(App);
delegate_output!(App);
delegate_shm!(App);
delegate_seat!(App);
delegate_keyboard!(App);
delegate_layer!(App);
delegate_registry!(App);
