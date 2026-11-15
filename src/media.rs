//! Screen sharing and camera video for voice channels.
//!
//! A `Capture` grabs pictures on its own thread (a whole screen, one app window, or a camera),
//! keeps the newest one for the local preview and serves it as JPEG pictures over TCP to anyone
//! on the home network who asks. A `Viewer` connects to someone else's capture and keeps their
//! newest picture. Pictures are length-prefixed: 4 bytes (big-endian size) then the JPEG bytes.

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Largest picture a viewer accepts.
const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

/// Picture quality choices: (label, tallest picture in pixels or 0 for the original size, JPEG quality).
pub const QUALITIES: [(&str, u32, u8); 4] = [
    ("Low · 480p", 480, 45),
    ("Medium · 720p", 720, 60),
    ("High · 1080p", 1080, 75),
    ("Best · original size", 0, 85),
];

/// Frame-rate choices (pictures per second).
pub const FRAME_RATES: [u8; 4] = [5, 15, 30, 60];

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Source {
    Screen,
    Camera,
}

impl Source {
    pub fn tag(self) -> &'static str {
        match self {
            Source::Screen => "screen",
            Source::Camera => "camera",
        }
    }
    pub fn from_tag(s: &str) -> Option<Source> {
        match s {
            "screen" => Some(Source::Screen),
            "camera" => Some(Source::Camera),
            _ => None,
        }
    }
}

/// Exactly what to capture.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum What {
    Monitor(u32),
    Window(u32),
    Camera(u32),
}

impl What {
    pub fn source(&self) -> Source {
        match self {
            What::Camera(_) => Source::Camera,
            _ => Source::Screen,
        }
    }
}

/// One thing that can be shared, for the picker.
#[derive(Clone)]
pub struct Choice {
    pub what: What,
    pub label: String,
    pub detail: String,
}

/// How a capture runs; can be changed while it's live.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct StreamSettings {
    /// Index into QUALITIES.
    pub quality: u8,
    pub fps: u8,
}

impl StreamSettings {
    fn max_height(self) -> u32 {
        QUALITIES[(self.quality as usize).min(QUALITIES.len() - 1)].1
    }
    fn jpeg_quality(self) -> u8 {
        QUALITIES[(self.quality as usize).min(QUALITIES.len() - 1)].2
    }
    fn fps(self) -> f64 {
        (self.fps as f64).clamp(1.0, 60.0)
    }
}

/// Every screen on this computer.
pub fn list_screens() -> Vec<Choice> {
    let Ok(list) = xcap::Monitor::all() else { return vec![] };
    list.iter()
        .enumerate()
        .filter_map(|(i, m)| {
            let id = m.id().ok()?;
            let (w, h) = (m.width().unwrap_or(0), m.height().unwrap_or(0));
            let primary = m.is_primary().unwrap_or(false);
            let name = m.friendly_name().or_else(|_| m.name()).unwrap_or_default();
            Some(Choice {
                what: What::Monitor(id),
                label: format!("Screen {}{}", i + 1, if primary { " (main)" } else { "" }),
                detail: format!("{name} · {w}×{h}").trim_start_matches(" · ").to_string(),
            })
        })
        .collect()
}

/// Open app windows that can be shared (not minimised, not tiny, not Hearth itself).
pub fn list_windows() -> Vec<Choice> {
    let Ok(list) = xcap::Window::all() else { return vec![] };
    let me = std::process::id();
    let mut out: Vec<Choice> = list
        .iter()
        .filter_map(|w| {
            if w.pid().ok()? == me || w.is_minimized().unwrap_or(false) {
                return None;
            }
            let title = w.title().unwrap_or_default();
            let (width, height) = (w.width().unwrap_or(0), w.height().unwrap_or(0));
            if title.trim().is_empty() || width < 120 || height < 80 {
                return None;
            }
            let app = w.app_name().unwrap_or_default();
            Some(Choice {
                what: What::Window(w.id().ok()?),
                label: title.chars().take(70).collect(),
                detail: format!("{} · {width}×{height}", if app.is_empty() { "App" } else { &app }),
            })
        })
        .collect();
    out.sort_by_key(|c| c.label.to_lowercase());
    out
}

/// Cameras on this computer.
pub fn list_cameras() -> Vec<Choice> {
    let Ok(list) = nokhwa::query(nokhwa::utils::ApiBackend::Auto) else { return vec![] };
    list.iter()
        .filter_map(|c| {
            let idx = match c.index() {
                nokhwa::utils::CameraIndex::Index(i) => *i,
                _ => return None,
            };
            Some(Choice { what: What::Camera(idx), label: c.human_name(), detail: c.description().to_string() })
        })
        .collect()
}

/// A decoded picture ready to show.
#[derive(Clone)]
pub struct Picture {
    pub width: usize,
    pub height: usize,
    pub rgba: Arc<Vec<u8>>,
}

#[derive(Default)]
struct Shared {
    picture: Option<Picture>,
    jpeg: Option<Arc<Vec<u8>>>,
    error: Option<String>,
    /// Pictures per second actually reached (measured).
    fps: f32,
}

/// The newest picture waiting to be compressed. Older ones are simply replaced, so a slow
/// moment never builds up a delay.
type Mailbox = Arc<(Mutex<Option<(Picture, u8)>>, Condvar)>;

/// Counts pictures and turns them into "pictures per second" about twice a second.
struct FpsMeter {
    since: Instant,
    frames: u32,
}

impl FpsMeter {
    fn new() -> Self {
        FpsMeter { since: Instant::now(), frames: 0 }
    }
    fn tick(&mut self, shared: &Mutex<Shared>) {
        self.frames += 1;
        let dt = self.since.elapsed().as_secs_f32();
        if dt >= 0.5 {
            shared.lock().unwrap().fps = self.frames as f32 / dt;
            self.frames = 0;
            self.since = Instant::now();
        }
    }
}

/// Everything a capture's threads share.
#[derive(Clone)]
struct Pipe {
    shared: Arc<Mutex<Shared>>,
    /// New preview pictures (for the person sharing).
    counter: Arc<AtomicU64>,
    /// New compressed pictures (for viewers).
    jpeg_counter: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    settings: Arc<Mutex<StreamSettings>>,
    mailbox: Mailbox,
    repaint: Option<egui::Context>,
}

/// Which stream a viewer wants: one byte sent right after connecting.
pub fn kind_byte(s: Source) -> u8 {
    match s {
        Source::Screen => b'S',
        Source::Camera => b'C',
    }
}

type Slot = (Arc<Mutex<Shared>>, Arc<AtomicU64>, Arc<AtomicBool>);

/// One TCP port (the fixed home-network port) for every stream on this computer.
#[derive(Clone)]
pub struct StreamServer {
    pub port: u16,
    slots: Arc<Mutex<std::collections::HashMap<u8, Slot>>>,
}

impl StreamServer {
    pub fn start(port: u16) -> Option<StreamServer> {
        let listener = TcpListener::bind(("0.0.0.0", port)).ok()?;
        let server = StreamServer { port, slots: Arc::new(Mutex::new(std::collections::HashMap::new())) };
        let slots = server.slots.clone();
        std::thread::Builder::new()
            .name("hearth-streams".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    let slots = slots.clone();
                    std::thread::spawn(move || {
                        let mut stream = stream;
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                        let mut k = [0u8; 1];
                        let wanted = stream.read_exact(&mut k).ok().map(|_| k[0]);
                        let slot = {
                            let s = slots.lock().unwrap();
                            match wanted {
                                Some(b) => s.get(&b).cloned(),
                                // Older Hearth sends nothing: give it the only stream, if there is one.
                                None if s.len() == 1 => s.values().next().cloned(),
                                None => None,
                            }
                        };
                        if let Some((sh, ct, st)) = slot {
                            send_to_viewer(stream, sh, ct, st);
                        }
                    });
                }
            })
            .ok()?;
        Some(server)
    }

    fn register(&self, kind: u8, slot: Slot) {
        self.slots.lock().unwrap().insert(kind, slot);
    }

    fn unregister(&self, kind: u8) {
        self.slots.lock().unwrap().remove(&kind);
    }
}

pub struct Capture {
    pub source: Source,
    pub what: What,
    pub label: String,
    pub port: u16,
    pipe: Pipe,
    server: Option<StreamServer>,
}

impl Capture {
    /// `repaint` lets the window redraw as soon as a new picture is ready.
    pub fn start(
        what: What,
        label: String,
        settings: StreamSettings,
        repaint: Option<egui::Context>,
        server: Option<StreamServer>,
    ) -> Capture {
        let pipe = Pipe {
            shared: Arc::new(Mutex::new(Shared::default())),
            counter: Arc::new(AtomicU64::new(0)),
            jpeg_counter: Arc::new(AtomicU64::new(0)),
            stop: Arc::new(AtomicBool::new(false)),
            settings: Arc::new(Mutex::new(settings)),
            mailbox: Arc::new((Mutex::new(None), Condvar::new())),
            repaint,
        };

        // Viewers connect to the shared stream port; without it, to a port of this capture's own.
        let port = if let Some(srv) = &server {
            srv.register(kind_byte(what.source()), (pipe.shared.clone(), pipe.jpeg_counter.clone(), pipe.stop.clone()));
            srv.port
        } else {
            match TcpListener::bind(("0.0.0.0", 0)) {
            Ok(listener) => {
                let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
                let _ = listener.set_nonblocking(true);
                let (sh, ct, st) = (pipe.shared.clone(), pipe.jpeg_counter.clone(), pipe.stop.clone());
                std::thread::spawn(move || serve(listener, sh, ct, st));
                port
            }
            Err(e) => {
                pipe.shared.lock().unwrap().error = Some(format!("Couldn't open a port for viewers: {e}"));
                0
            }
            }
        };

        // Compressing runs beside grabbing, so the two don't wait for each other.
        {
            let p = pipe.clone();
            std::thread::Builder::new().name("hearth-encode".into()).spawn(move || encode_loop(p)).ok();
        }
        {
            let p = pipe.clone();
            let w = what.clone();
            std::thread::Builder::new()
                .name("hearth-capture".into())
                .spawn(move || match w {
                    What::Monitor(id) => monitor_loop(id, p),
                    What::Window(id) => window_loop(id, p),
                    What::Camera(i) => camera_loop(i, p),
                })
                .ok();
        }
        Capture { source: what.source(), what, label, port, pipe, server }
    }

    pub fn settings(&self) -> StreamSettings {
        *self.pipe.settings.lock().unwrap()
    }
    /// Change quality or frame rate without stopping the stream.
    pub fn set_settings(&self, s: StreamSettings) {
        *self.pipe.settings.lock().unwrap() = s;
    }
    pub fn frame_number(&self) -> u64 {
        self.pipe.counter.load(Ordering::Relaxed)
    }
    pub fn picture(&self) -> Option<Picture> {
        self.pipe.shared.lock().unwrap().picture.clone()
    }
    pub fn error(&self) -> Option<String> {
        self.pipe.shared.lock().unwrap().error.clone()
    }
    /// Pictures per second actually being sent.
    pub fn fps(&self) -> f32 {
        self.pipe.shared.lock().unwrap().fps
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.pipe.stop.store(true, Ordering::Relaxed);
        self.pipe.mailbox.1.notify_all();
        if let Some(srv) = &self.server {
            srv.unregister(kind_byte(self.source));
        }
    }
}

/// Resizes pictures quickly (SIMD), keeping its working memory between pictures.
struct Shrinker {
    resizer: fast_image_resize::Resizer,
}

impl Shrinker {
    fn new() -> Self {
        Shrinker { resizer: fast_image_resize::Resizer::new() }
    }

    /// RGBA pixels → at most `max_h` tall (0 = keep the size). Never enlarges.
    fn shrink(&mut self, w: u32, h: u32, rgba: Vec<u8>, max_h: u32) -> (u32, u32, Vec<u8>) {
        if max_h == 0 || h <= max_h || w == 0 || h == 0 {
            return (w, h, rgba);
        }
        let nw = (((w as f64) * (max_h as f64) / (h as f64)).round() as u32).max(1);
        use fast_image_resize::{images::Image, FilterType, PixelType, ResizeAlg, ResizeOptions};
        let Ok(src) = Image::from_vec_u8(w, h, rgba, PixelType::U8x4) else {
            return (w, h, vec![]);
        };
        let mut dst = Image::new(nw, max_h, PixelType::U8x4);
        // Screen pictures are opaque: skipping alpha handling is faster.
        let opts = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Bilinear)).use_alpha(false);
        if self.resizer.resize(&src, &mut dst, &opts).is_err() {
            return (w, h, src.into_vec());
        }
        (nw, max_h, dst.into_vec())
    }
}

/// A grabbed picture: show it at once, and hand it to the compressor (replacing any older one).
fn hand_over(p: &Pipe, w: u32, h: u32, rgba: Vec<u8>) {
    if rgba.len() < (w as usize) * (h as usize) * 4 || w == 0 || h == 0 {
        return;
    }
    let pic = Picture { width: w as usize, height: h as usize, rgba: Arc::new(rgba) };
    {
        let mut s = p.shared.lock().unwrap();
        s.picture = Some(pic.clone());
        s.error = None;
    }
    p.counter.fetch_add(1, Ordering::Relaxed);
    let quality = p.settings.lock().unwrap().jpeg_quality();
    *p.mailbox.0.lock().unwrap() = Some((pic, quality));
    p.mailbox.1.notify_one();
    if let Some(ctx) = &p.repaint {
        ctx.request_repaint();
    }
}

/// JPEG with fast (SIMD) compression and 4:2:0 colour, straight from RGBA.
fn encode_jpeg(pic: &Picture, quality: u8) -> Option<Vec<u8>> {
    if pic.width > u16::MAX as usize || pic.height > u16::MAX as usize {
        return None;
    }
    let mut out = Vec::with_capacity(pic.width * pic.height / 6);
    let mut enc = jpeg_encoder::Encoder::new(&mut out, quality);
    enc.set_sampling_factor(jpeg_encoder::SamplingFactor::R_4_2_0);
    enc.encode(&pic.rgba, pic.width as u16, pic.height as u16, jpeg_encoder::ColorType::Rgba).ok()?;
    Some(out)
}

fn store_jpeg(shared: &Mutex<Shared>, jpeg_counter: &AtomicU64, jpeg: Vec<u8>) {
    shared.lock().unwrap().jpeg = Some(Arc::new(jpeg));
    jpeg_counter.fetch_add(1, Ordering::Relaxed);
}

fn encode_loop(p: Pipe) {
    let mut meter = FpsMeter::new();
    while !p.stop.load(Ordering::Relaxed) {
        let job = {
            let (lock, cv) = &*p.mailbox;
            let mut slot = lock.lock().unwrap();
            if slot.is_none() {
                slot = cv.wait_timeout(slot, Duration::from_millis(200)).unwrap().0;
            }
            slot.take()
        };
        if let Some((pic, quality)) = job {
            if let Some(jpeg) = encode_jpeg(&pic, quality) {
                store_jpeg(&p.shared, &p.jpeg_counter, jpeg);
                meter.tick(&p.shared);
            }
        }
    }
}

fn set_error(shared: &Mutex<Shared>, e: String) {
    shared.lock().unwrap().error = Some(e);
}

fn pace(start: Instant, fps: f64) {
    let budget = Duration::from_secs_f64(1.0 / fps);
    let spent = start.elapsed();
    if spent < budget {
        std::thread::sleep(budget - spent);
    }
}

fn monitor_loop(id: u32, p: Pipe) {
    let monitor = xcap::Monitor::all().ok().and_then(|l| l.into_iter().find(|m| m.id().ok() == Some(id)));
    let Some(monitor) = monitor else {
        set_error(&p.shared, "That screen isn't connected any more.".into());
        return;
    };
    let mut shrinker = Shrinker::new();

    // Fast path: Windows' desktop duplication hands over the screen as the graphics card
    // draws it. Asked for one picture at a time, so it never copies more than needed.
    if let Ok((recorder, rx)) = monitor.video_recorder() {
        if recorder.start().is_ok() {
            if let Ok(first) = rx.recv_timeout(Duration::from_millis(1500)) {
                let _ = recorder.stop();
                let s = *p.settings.lock().unwrap();
                let (w, h, px) = shrinker.shrink(first.width, first.height, first.raw, s.max_height());
                hand_over(&p, w, h, px);
                while !p.stop.load(Ordering::Relaxed) {
                    let t = Instant::now();
                    let s = *p.settings.lock().unwrap();
                    let budget = Duration::from_secs_f64(1.0 / s.fps());
                    while rx.try_recv().is_ok() {} // drop anything older than now
                    if recorder.start().is_err() {
                        break;
                    }
                    let got = rx.recv_timeout(budget);
                    let _ = recorder.stop();
                    match got {
                        Ok(frame) => {
                            let (w, h, px) = shrinker.shrink(frame.width, frame.height, frame.raw, s.max_height());
                            hand_over(&p, w, h, px);
                        }
                        // Nothing changed on screen: viewers keep the last picture.
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                    pace(t, s.fps());
                }
                let _ = recorder.stop();
                if p.stop.load(Ordering::Relaxed) {
                    return;
                }
            } else {
                let _ = recorder.stop();
            }
        }
    }

    // Fallback: copy the screen the slower, older way.
    while !p.stop.load(Ordering::Relaxed) {
        let t = Instant::now();
        let s = *p.settings.lock().unwrap();
        match monitor.capture_image() {
            Ok(img) => {
                let (w, h) = (img.width(), img.height());
                let (w, h, px) = shrinker.shrink(w, h, img.into_raw(), s.max_height());
                hand_over(&p, w, h, px);
            }
            Err(e) => set_error(&p.shared, format!("Screen capture failed: {e}")),
        }
        pace(t, s.fps());
    }
}

fn window_loop(id: u32, p: Pipe) {
    let find = || xcap::Window::all().ok().and_then(|l| l.into_iter().find(|w| w.id().ok() == Some(id)));
    let Some(mut window) = find() else {
        set_error(&p.shared, "That window was closed.".into());
        return;
    };
    let mut shrinker = Shrinker::new();
    let mut failures = 0u32;
    while !p.stop.load(Ordering::Relaxed) {
        let t = Instant::now();
        let s = *p.settings.lock().unwrap();
        if window.is_minimized().unwrap_or(false) {
            set_error(&p.shared, "The shared window is minimised. Open it again to keep sharing.".into());
        } else {
            match window.capture_image() {
                Ok(img) => {
                    failures = 0;
                    let (w, h) = (img.width(), img.height());
                    let (w, h, px) = shrinker.shrink(w, h, img.into_raw(), s.max_height());
                    hand_over(&p, w, h, px);
                }
                Err(_) => {
                    failures += 1;
                    // The window may have been closed: look for it again.
                    match find() {
                        Some(w) if failures < 50 => window = w,
                        _ => {
                            set_error(&p.shared, "That window was closed.".into());
                            if failures >= 50 {
                                return;
                            }
                        }
                    }
                }
            }
        }
        pace(t, s.fps());
    }
}

fn camera_loop(index: u32, p: Pipe) {
    use nokhwa::pixel_format::RgbFormat;
    use nokhwa::utils::{CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType};

    let format = RequestedFormat::new::<RgbFormat>(RequestedFormatType::AbsoluteHighestFrameRate);
    let mut camera = match nokhwa::Camera::new(CameraIndex::Index(index), format) {
        Ok(c) => c,
        Err(e) => {
            set_error(&p.shared, format!("No camera found ({e})."));
            return;
        }
    };
    if let Err(e) = camera.open_stream() {
        set_error(&p.shared, format!("Couldn't start the camera: {e}"));
        return;
    }
    let mut shrinker = Shrinker::new();
    while !p.stop.load(Ordering::Relaxed) {
        let t = Instant::now();
        let s = *p.settings.lock().unwrap();
        match camera.frame() {
            Ok(buf) => {
                let rgb: Option<image::RgbImage> = if buf.source_frame_format() == FrameFormat::MJPEG {
                    image::load_from_memory_with_format(buf.buffer(), image::ImageFormat::Jpeg).ok().map(|d| d.to_rgb8())
                } else {
                    buf.decode_image::<RgbFormat>()
                        .ok()
                        .and_then(|b| image::RgbImage::from_raw(b.width(), b.height(), b.into_raw()))
                };
                match rgb {
                    Some(rgb) => {
                        let rgba = image::DynamicImage::ImageRgb8(rgb).to_rgba8();
                        let (w, h) = (rgba.width(), rgba.height());
                        let (w, h, px) = shrinker.shrink(w, h, rgba.into_raw(), s.max_height());
                        hand_over(&p, w, h, px);
                    }
                    None => set_error(&p.shared, "The camera sent a picture Hearth can't read.".into()),
                }
            }
            Err(e) => set_error(&p.shared, format!("Camera stopped: {e}")),
        }
        pace(t, s.fps());
    }
    let _ = camera.stop_stream();
}

fn serve(listener: TcpListener, shared: Arc<Mutex<Shared>>, counter: Arc<AtomicU64>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                let (sh, ct, st) = (shared.clone(), counter.clone(), stop.clone());
                std::thread::spawn(move || send_to_viewer(stream, sh, ct, st));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

/// Sends each viewer the newest picture. A viewer on slow Wi-Fi simply skips pictures
/// instead of falling behind.
fn send_to_viewer(mut stream: TcpStream, shared: Arc<Mutex<Shared>>, counter: Arc<AtomicU64>, stop: Arc<AtomicBool>) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_nodelay(true);
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let mut last = u64::MAX;
    while !stop.load(Ordering::Relaxed) {
        let n = counter.load(Ordering::Relaxed);
        if n != last {
            let jpeg = shared.lock().unwrap().jpeg.clone();
            if let Some(jpeg) = jpeg {
                let mut msg = Vec::with_capacity(4 + jpeg.len());
                msg.extend_from_slice(&(jpeg.len() as u32).to_be_bytes());
                msg.extend_from_slice(&jpeg);
                if stream.write_all(&msg).is_err() {
                    return; // viewer left
                }
                last = n;
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Watching someone else's screen, window or camera.
pub struct Viewer {
    shared: Arc<Mutex<Shared>>,
    counter: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
}

impl Viewer {
    /// `repaint` lets the window redraw the moment a picture arrives.
    /// `kind`: which stream to ask for on a shared stream port (see `kind_byte`).
    pub fn start(ip: IpAddr, port: u16, kind: Option<u8>, repaint: Option<egui::Context>) -> Viewer {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let counter = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (sh, ct, st) = (shared.clone(), counter.clone(), stop.clone());
        std::thread::spawn(move || {
            let addr = SocketAddr::new(ip, port);
            let mut stream = match TcpStream::connect_timeout(&addr, Duration::from_secs(4)) {
                Ok(s) => s,
                Err(e) => {
                    set_error(&sh, format!("Couldn't connect: {e}"));
                    return;
                }
            };
            let _ = stream.set_nodelay(true);
            if let Some(k) = kind {
                let _ = stream.write_all(&[k]);
            }
            let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
            let mut meter = FpsMeter::new();
            let mut len = [0u8; 4];
            let mut buf: Vec<u8> = Vec::new();
            while !st.load(Ordering::Relaxed) {
                if stream.read_exact(&mut len).is_err() {
                    set_error(&sh, "The stream ended.".into());
                    return;
                }
                let n = u32::from_be_bytes(len) as usize;
                if n == 0 || n > MAX_FRAME_BYTES {
                    set_error(&sh, "The stream sent something unexpected.".into());
                    return;
                }
                buf.resize(n, 0);
                if stream.read_exact(&mut buf).is_err() {
                    set_error(&sh, "The stream ended.".into());
                    return;
                }
                if let Ok(img) = image::load_from_memory_with_format(&buf, image::ImageFormat::Jpeg) {
                    let rgba = img.to_rgba8();
                    let (w, h) = (rgba.width() as usize, rgba.height() as usize);
                    {
                        let mut s = sh.lock().unwrap();
                        s.picture = Some(Picture { width: w, height: h, rgba: Arc::new(rgba.into_raw()) });
                        s.error = None;
                    }
                    ct.fetch_add(1, Ordering::Relaxed);
                    meter.tick(&sh);
                    if let Some(ctx) = &repaint {
                        ctx.request_repaint();
                    }
                }
            }
        });
        Viewer { shared, counter, stop }
    }

    pub fn frame_number(&self) -> u64 {
        self.counter.load(Ordering::Relaxed)
    }
    pub fn picture(&self) -> Option<Picture> {
        self.shared.lock().unwrap().picture.clone()
    }
    pub fn error(&self) -> Option<String> {
        self.shared.lock().unwrap().error.clone()
    }
    /// Pictures per second arriving.
    pub fn fps(&self) -> f32 {
        self.shared.lock().unwrap().fps
    }
}

impl Drop for Viewer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pattern(w: u32, h: u32) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                v.extend_from_slice(&[(x % 256) as u8, (y % 256) as u8, 128, 255]);
            }
        }
        v
    }

    /// A made-up picture goes through the same path as a shared screen: resize, JPEG, TCP, decode.
    #[test]
    fn picture_travels_from_sharer_to_viewer() {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let counter = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        {
            let (sh, ct, st) = (shared.clone(), counter.clone(), stop.clone());
            std::thread::spawn(move || serve(listener, sh, ct, st));
        }
        let medium = StreamSettings { quality: 1, fps: 15 };
        let (w, h, px) = Shrinker::new().shrink(1920, 1080, pattern(1920, 1080), medium.max_height());
        let pic = Picture { width: w as usize, height: h as usize, rgba: Arc::new(px) };
        store_jpeg(&shared, &counter, encode_jpeg(&pic, medium.jpeg_quality()).unwrap());

        let viewer = Viewer::start("127.0.0.1".parse().unwrap(), port, None, None);
        let t = Instant::now();
        while viewer.picture().is_none() && t.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(20));
        }
        let got = viewer.picture().expect("viewer got a picture");
        assert_eq!((got.width, got.height), (1280, 720));
        stop.store(true, Ordering::Relaxed);
    }

    #[test]
    fn quality_choices_set_picture_size() {
        let mut s = Shrinker::new();
        assert_eq!(s.shrink(1920, 1080, pattern(1920, 1080), QUALITIES[0].1).1, 480);
        assert_eq!(s.shrink(1920, 1080, pattern(1920, 1080), QUALITIES[2].1).1, 1080);
        assert_eq!(s.shrink(2560, 1440, pattern(2560, 1440), QUALITIES[3].1).1, 1440); // original size
        assert_eq!(s.shrink(640, 360, pattern(640, 360), QUALITIES[1].1).1, 360); // never enlarged
    }

    #[test]
    fn higher_quality_means_bigger_pictures() {
        let pic = Picture { width: 1280, height: 720, rgba: Arc::new(pattern(1280, 720)) };
        let low = encode_jpeg(&pic, QUALITIES[0].2).unwrap().len();
        let best = encode_jpeg(&pic, QUALITIES[3].2).unwrap().len();
        assert!(low < best);
    }

    /// Smoothness budget: shrinking a big screen to 720p and compressing it must take well
    /// under a 60 fps frame (16 ms) on average, so 30-60 fps is reachable.
    #[test]
    fn one_picture_is_fast_enough_for_smooth_video() {
        let mut s = Shrinker::new();
        let src = pattern(2560, 1440);
        let start = Instant::now();
        let rounds = 20;
        for _ in 0..rounds {
            let (w, h, px) = s.shrink(2560, 1440, src.clone(), 720);
            let pic = Picture { width: w as usize, height: h as usize, rgba: Arc::new(px) };
            assert!(encode_jpeg(&pic, 60).is_some());
        }
        let each = start.elapsed() / rounds;
        assert!(each < Duration::from_millis(16), "one picture took {each:?}");
    }

    /// The compressor always takes the newest picture; older ones waiting are dropped.
    #[test]
    fn mailbox_keeps_only_the_newest_picture() {
        let pipe = Pipe {
            shared: Arc::new(Mutex::new(Shared::default())),
            counter: Arc::new(AtomicU64::new(0)),
            jpeg_counter: Arc::new(AtomicU64::new(0)),
            stop: Arc::new(AtomicBool::new(false)),
            settings: Arc::new(Mutex::new(StreamSettings { quality: 1, fps: 30 })),
            mailbox: Arc::new((Mutex::new(None), Condvar::new())),
            repaint: None,
        };
        for w in [64u32, 96, 128] {
            hand_over(&pipe, w, 64, pattern(w, 64));
        }
        let waiting = pipe.mailbox.0.lock().unwrap().take().unwrap();
        assert_eq!(waiting.0.width, 128);
        assert_eq!(pipe.counter.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn stream_tags_round_trip() {
        for s in [Source::Screen, Source::Camera] {
            assert_eq!(Source::from_tag(s.tag()), Some(s));
        }
    }
}