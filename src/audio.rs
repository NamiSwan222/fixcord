//! Sound for Hearth: voice chat between home-network friends and the app's own interface sounds.
//!
//! Voice: the microphone is read only while you're in a voice channel (or testing it), turned
//! into 48 kHz mono, cut into 20 ms pieces and sent as plain 16-bit samples by UDP straight to
//! the others in the same Home Network voice channel. Incoming voices are buffered a little
//! (about 40 ms) to smooth out the network, mixed together with the interface sounds and played
//! on the chosen speakers. Interface sounds are short tones made here; no recordings are used.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Internal sample rate.
pub const RATE: u32 = 48_000;
/// Samples in one 20 ms voice piece.
pub const FRAME: usize = 960;
/// Wait for this much voice before playing someone (smooths the network), in samples (40 ms).
const START_BUFFER: usize = FRAME * 2;
/// Never keep more than this much of someone's voice waiting (300 ms); older audio is dropped.
const MAX_BUFFER: usize = RATE as usize * 3 / 10;
/// Keep the microphone open this long after you stop talking (voice activity mode).
const HANGOVER: Duration = Duration::from_millis(400);
const MAGIC: &[u8; 3] = b"HV1";
/// Samples the noise suppressor works on at a time (10 ms at 48 kHz); two per voice piece.
const DENOISE_FRAME: usize = nnnoiseless::FRAME_SIZE;

/// Removes background noise (fans, keyboards, hum) from one 20 ms piece, in place.
/// Uses nnnoiseless, a Rust version of the open-source RNNoise noise suppressor (BSD licence).
/// Returns how sure it is that someone is talking (0..1).
fn denoise(state: &mut nnnoiseless::DenoiseState<'static>, frame: &mut [f32]) -> f32 {
    let mut voice: f32 = 0.0;
    let mut input = [0.0f32; DENOISE_FRAME];
    let mut output = [0.0f32; DENOISE_FRAME];
    for chunk in frame.chunks_mut(DENOISE_FRAME) {
        if chunk.len() != DENOISE_FRAME {
            continue;
        }
        // RNNoise expects 16-bit-sized numbers.
        for (i, s) in chunk.iter().enumerate() {
            input[i] = s * 32767.0;
        }
        voice = voice.max(state.process_frame(&mut output, &input));
        for (i, s) in chunk.iter_mut().enumerate() {
            *s = (output[i] / 32767.0).clamp(-1.0, 1.0);
        }
    }
    voice
}

/// Interface sounds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sfx {
    Message,
    Mention,
    Join,
    Leave,
    PeerJoin,
    PeerLeave,
    Mute,
    Unmute,
    Deafen,
    Undeafen,
    LiveStart,
    LiveStop,
}

#[derive(Default)]
struct Jitter {
    buf: VecDeque<f32>,
    playing: bool,
    level_db: f32,
    last_heard: Option<Instant>,
}

struct Shared {
    in_gain: AtomicU32,
    out_gain: AtomicU32,
    ui_gain: AtomicU32,
    threshold_db: AtomicU32,
    ptt_mode: AtomicBool,
    ptt_held: AtomicBool,
    muted: AtomicBool,
    deafened: AtomicBool,
    echo_reduce: AtomicBool,
    noise_suppression: AtomicBool,
    /// 0 = RNNoise, 1 = DeepFilterNet.
    noise_model: std::sync::atomic::AtomicU8,
    /// DeepFilterNet: most it may turn noise down, in dB.
    df_limit_db: AtomicU32,
    /// DeepFilterNet is loaded and running (it takes a moment the first time).
    df_ready: AtomicBool,
    loopback: AtomicBool,
    transmitting: AtomicBool,
    mic_db: AtomicU32,
    others_db: AtomicU32,
    allowed: Mutex<HashSet<u64>>,
    gains: Mutex<HashMap<u64, f32>>,
    incoming: Mutex<HashMap<u64, Jitter>>,
    sfx: Mutex<Vec<(Arc<Vec<f32>>, usize)>>,
    error: Mutex<Option<String>>,
    /// Short sound glitches since start (missed moments of sound; not errors).
    glitches: AtomicU32,
}

fn f(a: &AtomicU32) -> f32 {
    f32::from_bits(a.load(Ordering::Relaxed))
}
fn set_f(a: &AtomicU32, v: f32) {
    a.store(v.to_bits(), Ordering::Relaxed);
}

fn db(rms: f32) -> f32 {
    if rms <= 1e-6 {
        -100.0
    } else {
        20.0 * rms.log10()
    }
}

fn rms(s: &[f32]) -> f32 {
    if s.is_empty() {
        return 0.0;
    }
    (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt()
}

/// Linear-interpolating rate converter.
struct Resampler {
    step: f64,
    pos: f64,
    buf: VecDeque<f32>,
}

impl Resampler {
    fn new(from_rate: u32, to_rate: u32) -> Self {
        Resampler { step: from_rate as f64 / to_rate as f64, pos: 0.0, buf: VecDeque::new() }
    }
    fn needs(&self) -> bool {
        (self.pos as usize) + 1 >= self.buf.len()
    }
    fn push(&mut self, s: f32) {
        self.buf.push_back(s);
    }
    fn pull(&mut self) -> Option<f32> {
        let i = self.pos as usize;
        if i + 1 >= self.buf.len() {
            return None;
        }
        let t = (self.pos - i as f64) as f32;
        let v = self.buf[i] * (1.0 - t) + self.buf[i + 1] * t;
        self.pos += self.step;
        let drop = (self.pos as usize).min(self.buf.len());
        self.buf.drain(..drop);
        self.pos -= drop as f64;
        Some(v)
    }
}

/// A short tone with a quick attack and a soft fade.
fn tone(out: &mut Vec<f32>, start_s: f32, freq: f32, len_s: f32, amp: f32) {
    let start = (start_s * RATE as f32) as usize;
    let len = (len_s * RATE as f32) as usize;
    if out.len() < start + len {
        out.resize(start + len, 0.0);
    }
    for i in 0..len {
        let t = i as f32 / RATE as f32;
        let attack = (t / 0.006).min(1.0);
        let fade = (-t * 6.0 / len_s).exp();
        let v = (2.0 * std::f32::consts::PI * freq * t).sin() + 0.25 * (4.0 * std::f32::consts::PI * freq * t).sin();
        out[start + i] += v * amp * attack * fade;
    }
}

pub fn synth(s: Sfx) -> Vec<f32> {
    let mut v = vec![];
    // (start, frequency Hz, length s)
    let notes: &[(f32, f32, f32)] = match s {
        Sfx::Message => &[(0.0, 880.0, 0.12), (0.07, 1318.5, 0.18)],
        Sfx::Mention => &[(0.0, 784.0, 0.12), (0.08, 1046.5, 0.12), (0.16, 1568.0, 0.22)],
        Sfx::Join => &[(0.0, 523.3, 0.16), (0.10, 784.0, 0.26)],
        Sfx::Leave => &[(0.0, 784.0, 0.16), (0.10, 523.3, 0.26)],
        Sfx::PeerJoin => &[(0.0, 659.3, 0.12), (0.07, 987.8, 0.18)],
        Sfx::PeerLeave => &[(0.0, 987.8, 0.12), (0.07, 659.3, 0.18)],
        Sfx::Mute => &[(0.0, 392.0, 0.09)],
        Sfx::Unmute => &[(0.0, 587.3, 0.09)],
        Sfx::Deafen => &[(0.0, 392.0, 0.08), (0.06, 293.7, 0.12)],
        Sfx::Undeafen => &[(0.0, 293.7, 0.08), (0.06, 392.0, 0.12)],
        Sfx::LiveStart => &[(0.0, 523.3, 0.1), (0.07, 659.3, 0.1), (0.14, 784.0, 0.22)],
        Sfx::LiveStop => &[(0.0, 784.0, 0.1), (0.07, 659.3, 0.1), (0.14, 523.3, 0.22)],
    };
    for (start, freq, len) in notes {
        tone(&mut v, *start, *freq, *len, 0.22);
    }
    v
}

/// Names of the microphones and speakers on this computer.
pub fn device_names(input: bool) -> Vec<String> {
    let host = cpal::default_host();
    let list = if input { host.input_devices() } else { host.output_devices() };
    let mut names: Vec<String> = match list {
        Ok(it) => it.filter_map(|d| d.description().ok().map(|d| d.name().to_string())).collect(),
        Err(_) => vec![],
    };
    names.dedup();
    names
}

fn find_device(input: bool, name: Option<&str>) -> Option<cpal::Device> {
    let host = cpal::default_host();
    if let Some(name) = name {
        let list = if input { host.input_devices() } else { host.output_devices() };
        if let Ok(mut it) = list {
            if let Some(d) = it.find(|d| d.description().map(|x| x.name() == name).unwrap_or(false)) {
                return Some(d);
            }
        }
    }
    if input {
        host.default_input_device()
    } else {
        host.default_output_device()
    }
}

pub struct Audio {
    shared: Arc<Shared>,
    output: Option<(cpal::Stream, Option<String>)>,
    input: Option<(cpal::Stream, Option<String>)>,
    socket: Option<Arc<UdpSocket>>,
    sender: Option<Sender<Vec<u8>>>,
    targets: Arc<Mutex<Vec<SocketAddr>>>,
    pub voice_port: u16,
    pub self_id: u64,
    enabled: bool,
}

impl Audio {
    /// `enabled` false makes a silent stand-in (used while taking screenshots).
    pub fn new(self_id: u64, enabled: bool) -> Audio {
        let shared = Arc::new(Shared {
            in_gain: AtomicU32::new(1.0f32.to_bits()),
            out_gain: AtomicU32::new(1.0f32.to_bits()),
            ui_gain: AtomicU32::new(0.6f32.to_bits()),
            threshold_db: AtomicU32::new((-45.0f32).to_bits()),
            ptt_mode: AtomicBool::new(false),
            ptt_held: AtomicBool::new(false),
            muted: AtomicBool::new(false),
            deafened: AtomicBool::new(false),
            echo_reduce: AtomicBool::new(true),
            noise_suppression: AtomicBool::new(true),
            noise_model: std::sync::atomic::AtomicU8::new(1),
            df_limit_db: AtomicU32::new(100.0f32.to_bits()),
            df_ready: AtomicBool::new(false),
            loopback: AtomicBool::new(false),
            transmitting: AtomicBool::new(false),
            mic_db: AtomicU32::new((-100.0f32).to_bits()),
            others_db: AtomicU32::new((-100.0f32).to_bits()),
            allowed: Mutex::new(HashSet::new()),
            gains: Mutex::new(HashMap::new()),
            incoming: Mutex::new(HashMap::new()),
            sfx: Mutex::new(vec![]),
            error: Mutex::new(None),
            glitches: AtomicU32::new(0),
        });
        let mut a = Audio {
            shared,
            output: None,
            input: None,
            socket: None,
            sender: None,
            targets: Arc::new(Mutex::new(vec![])),
            voice_port: 0,
            self_id,
            enabled,
        };
        if enabled {
            a.start_network();
        }
        a
    }

    fn start_network(&mut self) {
        let Ok(sock) = UdpSocket::bind(("0.0.0.0", 0)) else {
            self.set_error("Couldn't open a network port for voice.".into());
            return;
        };
        self.voice_port = sock.local_addr().map(|a| a.port()).unwrap_or(0);
        let sock = Arc::new(sock);
        // Receiving voices
        {
            let sock = sock.clone();
            let shared = self.shared.clone();
            let _ = sock.set_read_timeout(Some(Duration::from_millis(500)));
            std::thread::spawn(move || {
                let mut buf = vec![0u8; 4096];
                loop {
                    let Ok((n, _)) = sock.recv_from(&mut buf) else { continue };
                    if n < 15 || &buf[..3] != MAGIC {
                        continue;
                    }
                    let from = u64::from_le_bytes(buf[3..11].try_into().unwrap());
                    if !shared.allowed.lock().unwrap().contains(&from) || shared.deafened.load(Ordering::Relaxed) {
                        continue;
                    }
                    let samples: Vec<f32> =
                        buf[15..n].chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect();
                    push_voice(&shared, from, &samples);
                }
            });
        }
        // Sending our voice to everyone in the channel
        let (tx, rx) = channel::<Vec<u8>>();
        {
            let sock = sock.clone();
            let targets = self.targets.clone();
            std::thread::spawn(move || {
                while let Ok(packet) = rx.recv() {
                    let list = targets.lock().unwrap().clone();
                    for t in list {
                        let _ = sock.send_to(&packet, t);
                    }
                }
            });
        }
        self.sender = Some(tx);
        self.socket = Some(sock);
    }

    fn set_error(&self, e: String) {
        *self.shared.error.lock().unwrap() = Some(e);
    }

    pub fn error(&self) -> Option<String> {
        self.shared.error.lock().unwrap().clone()
    }

    /// Start (or switch) the speakers. Cheap to call every frame.
    pub fn ensure_output(&mut self, device: Option<&str>) {
        if !self.enabled {
            return;
        }
        let wanted = device.map(|s| s.to_string());
        if let Some((_, cur)) = &self.output {
            if *cur == wanted {
                return;
            }
        }
        self.output = None;
        let Some(dev) = find_device(false, device) else {
            self.set_error("No speakers or headphones found.".into());
            return;
        };
        let Ok(cfg) = dev.default_output_config() else {
            self.set_error("The speakers can't be used right now.".into());
            return;
        };
        let format = cfg.sample_format();
        let config = cfg.config();
        let shared = self.shared.clone();
        let stream = match format {
            SampleFormat::F32 => build_output::<f32>(&dev, config, shared),
            SampleFormat::I16 => build_output::<i16>(&dev, config, shared),
            SampleFormat::U16 => build_output::<u16>(&dev, config, shared),
            SampleFormat::I32 => build_output::<i32>(&dev, config, shared),
            _ => Err(format!("The speakers use a sound format Hearth doesn't support ({format:?}).")),
        };
        match stream {
            Ok(s) => {
                let _ = s.play();
                self.output = Some((s, wanted));
                *self.shared.error.lock().unwrap() = None;
            }
            Err(e) => self.set_error(e),
        }
    }

    /// Open (or switch) the microphone.
    pub fn start_mic(&mut self, device: Option<&str>) {
        if !self.enabled {
            return;
        }
        let wanted = device.map(|s| s.to_string());
        if let Some((_, cur)) = &self.input {
            if *cur == wanted {
                return;
            }
        }
        self.input = None;
        let Some(dev) = find_device(true, device) else {
            self.set_error("No microphone found.".into());
            return;
        };
        let Ok(cfg) = dev.default_input_config() else {
            self.set_error("The microphone can't be used right now.".into());
            return;
        };
        let format = cfg.sample_format();
        let config = cfg.config();
        let (shared, sender, id) = (self.shared.clone(), self.sender.clone(), self.self_id);
        let stream = match format {
            SampleFormat::F32 => build_input::<f32>(&dev, config, shared, sender, id),
            SampleFormat::I16 => build_input::<i16>(&dev, config, shared, sender, id),
            SampleFormat::U16 => build_input::<u16>(&dev, config, shared, sender, id),
            SampleFormat::I32 => build_input::<i32>(&dev, config, shared, sender, id),
            _ => Err(format!("The microphone uses a sound format Hearth doesn't support ({format:?}).")),
        };
        match stream {
            Ok(s) => {
                let _ = s.play();
                self.input = Some((s, wanted));
                *self.shared.error.lock().unwrap() = None;
            }
            Err(e) => self.set_error(e),
        }
    }

    pub fn stop_mic(&mut self) {
        self.input = None;
        self.shared.transmitting.store(false, Ordering::Relaxed);
        set_f(&self.shared.mic_db, -100.0);
    }

    pub fn mic_open(&self) -> bool {
        self.input.is_some()
    }

    pub fn play(&mut self, s: Sfx, output_device: Option<&str>) {
        if !self.enabled {
            return;
        }
        self.ensure_output(output_device);
        let mut list = self.shared.sfx.lock().unwrap();
        if list.len() < 8 {
            list.push((Arc::new(synth(s)), 0));
        }
    }

    /// Who we send to and accept voices from: (their id, their voice address).
    pub fn set_peers(&self, peers: &[(u64, SocketAddr)]) {
        *self.targets.lock().unwrap() = peers.iter().map(|(_, a)| *a).collect();
        let ids: HashSet<u64> = peers.iter().map(|(id, _)| *id).collect();
        let mut incoming = self.shared.incoming.lock().unwrap();
        incoming.retain(|id, _| ids.contains(id) || *id == self.self_id);
        *self.shared.allowed.lock().unwrap() = ids;
    }

    pub fn set_peer_gain(&self, id: u64, gain: f32) {
        self.shared.gains.lock().unwrap().insert(id, gain);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_controls(
        &self,
        in_gain: f32,
        out_gain: f32,
        ui_gain: f32,
        threshold_db: f32,
        ptt_mode: bool,
        ptt_held: bool,
        muted: bool,
        deafened: bool,
        echo_reduce: bool,
    ) {
        let s = &self.shared;
        set_f(&s.in_gain, in_gain);
        set_f(&s.out_gain, out_gain);
        set_f(&s.ui_gain, ui_gain);
        set_f(&s.threshold_db, threshold_db);
        s.ptt_mode.store(ptt_mode, Ordering::Relaxed);
        s.ptt_held.store(ptt_held, Ordering::Relaxed);
        s.muted.store(muted, Ordering::Relaxed);
        s.deafened.store(deafened, Ordering::Relaxed);
        s.echo_reduce.store(echo_reduce, Ordering::Relaxed);
    }

    pub fn set_noise_suppression(&self, on: bool) {
        self.shared.noise_suppression.store(on, Ordering::Relaxed);
    }

    /// `model`: 0 = RNNoise, 1 = DeepFilterNet. `limit_db`: most DeepFilterNet may remove.
    pub fn set_noise_model(&self, model: u8, limit_db: f32) {
        self.shared.noise_model.store(model.min(1), Ordering::Relaxed);
        set_f(&self.shared.df_limit_db, limit_db.clamp(0.0, 100.0));
    }

    /// DeepFilterNet has finished loading and is cleaning the microphone.
    pub fn deepfilter_ready(&self) -> bool {
        self.shared.df_ready.load(Ordering::Relaxed)
    }

    /// Hear your own microphone (to test it).
    pub fn set_loopback(&self, on: bool) {
        self.shared.loopback.store(on, Ordering::Relaxed);
        if !on {
            self.shared.incoming.lock().unwrap().remove(&self.self_id);
        }
    }

    /// How many short sound glitches happened since Hearth started.
    pub fn glitches(&self) -> u32 {
        self.shared.glitches.load(Ordering::Relaxed)
    }

    pub fn mic_db(&self) -> f32 {
        f(&self.shared.mic_db)
    }
    pub fn transmitting(&self) -> bool {
        self.shared.transmitting.load(Ordering::Relaxed)
    }
    /// Is this person talking right now?
    pub fn peer_speaking(&self, id: u64) -> bool {
        let incoming = self.shared.incoming.lock().unwrap();
        incoming
            .get(&id)
            .map(|j| j.level_db > -50.0 && j.last_heard.map(|t| t.elapsed() < Duration::from_millis(300)).unwrap_or(false))
            .unwrap_or(false)
    }
    pub fn peer_heard_recently(&self, id: u64) -> bool {
        let incoming = self.shared.incoming.lock().unwrap();
        incoming.get(&id).and_then(|j| j.last_heard).map(|t| t.elapsed() < Duration::from_secs(5)).unwrap_or(false)
    }
}

fn push_voice(shared: &Shared, from: u64, samples: &[f32]) {
    let mut incoming = shared.incoming.lock().unwrap();
    let j = incoming.entry(from).or_default();
    j.buf.extend(samples.iter().copied());
    if j.buf.len() > MAX_BUFFER {
        let extra = j.buf.len() - MAX_BUFFER / 2;
        j.buf.drain(..extra);
    }
    j.level_db = db(rms(samples));
    j.last_heard = Some(Instant::now());
}

/// Mix `n` samples at 48 kHz: everyone's voice plus interface sounds.
fn mix(shared: &Shared, n: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; n];
    let deaf = shared.deafened.load(Ordering::Relaxed);
    let out_gain = f(&shared.out_gain);
    {
        let gains = shared.gains.lock().unwrap();
        let mut incoming = shared.incoming.lock().unwrap();
        for (id, j) in incoming.iter_mut() {
            if !j.playing && j.buf.len() >= START_BUFFER {
                j.playing = true;
            }
            if !j.playing {
                continue;
            }
            let g = gains.get(id).copied().unwrap_or(1.0) * out_gain;
            for o in out.iter_mut() {
                match j.buf.pop_front() {
                    Some(s) => {
                        if !deaf {
                            *o += s * g;
                        }
                    }
                    None => {
                        j.playing = false; // ran dry: wait for the buffer to fill again
                        break;
                    }
                }
            }
        }
    }
    set_f(&shared.others_db, if deaf { -100.0 } else { db(rms(&out)) });
    let ui_gain = f(&shared.ui_gain);
    let mut sfx = shared.sfx.lock().unwrap();
    for (sound, pos) in sfx.iter_mut() {
        for o in out.iter_mut() {
            if *pos >= sound.len() {
                break;
            }
            *o += sound[*pos] * ui_gain;
            *pos += 1;
        }
    }
    sfx.retain(|(s, p)| *p < s.len());
    for o in out.iter_mut() {
        *o = o.clamp(-1.0, 1.0);
    }
    out
}

fn build_output<T>(dev: &cpal::Device, config: cpal::StreamConfig, shared: Arc<Shared>) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels.max(1) as usize;
    let mut rs = Resampler::new(RATE, config.sample_rate);
    let err_shared = shared.clone();
    dev.build_output_stream::<T, _, _>(
        config,
        move |data: &mut [T], _| {
            for frame in data.chunks_mut(channels) {
                while rs.needs() {
                    for s in mix(&shared, 240) {
                        rs.push(s);
                    }
                }
                let v = rs.pull().unwrap_or(0.0);
                for c in frame.iter_mut() {
                    *c = T::from_sample(v);
                }
            }
        },
        move |e| note_stream_error(&err_shared, "Speaker", e),
        None,
    )
    .map_err(|e| format!("Couldn't start the speakers: {e}"))
}

/// Everything done to the microphone sound after it leaves the sound card: change it to
/// 48 kHz, clean it, measure it, decide whether to send it, and send it. Runs on its own
/// thread so the sound card's time-critical callback stays tiny (a slow callback makes Windows
/// drop microphone sound, which shows up as "buffer underrun or overrun").
struct MicProcessor {
    shared: Arc<Shared>,
    sender: Option<Sender<Vec<u8>>>,
    self_id: u64,
    rs: Resampler,
    frame: Vec<f32>,
    last_voice: Instant,
    seq: u32,
    denoiser: Box<nnnoiseless::DenoiseState<'static>>,
    deep: DeepSlot,
}

/// DeepFilterNet3 (a neural-network noise suppressor) through deep-filter-rt, which runs the
/// model on the CPU with tract. Cleans 10 ms at a time, about 30 ms behind.
struct DeepFilter {
    df: deep_filter_rt::tract::DfTract,
    noisy: ndarray::Array2<f32>,
    enh: ndarray::Array2<f32>,
    limit_db: f32,
}

impl DeepFilter {
    fn load() -> Option<DeepFilter> {
        // Loading prepares the model; don't let a problem there take the microphone down.
        let df = std::panic::catch_unwind(deep_filter_rt::tract::DfTract::default).ok()?;
        let hop = df.hop_size;
        if df.sr != RATE as usize || hop == 0 || FRAME % hop != 0 {
            return None;
        }
        Some(DeepFilter { df, noisy: ndarray::Array2::zeros((1, hop)), enh: ndarray::Array2::zeros((1, hop)), limit_db: 100.0 })
    }

    /// Cleans one piece in place. False if the model failed (the piece is left as it was).
    fn process(&mut self, frame: &mut [f32], limit_db: f32) -> bool {
        if (limit_db - self.limit_db).abs() > 0.1 {
            self.df.set_atten_lim(limit_db);
            self.limit_db = limit_db;
        }
        let hop = self.df.hop_size;
        for chunk in frame.chunks_mut(hop) {
            if chunk.len() != hop {
                return false;
            }
            for (d, s) in self.noisy.iter_mut().zip(chunk.iter()) {
                *d = *s;
            }
            if self.df.process(self.noisy.view(), self.enh.view_mut()).is_err() {
                return false;
            }
            for (d, s) in chunk.iter_mut().zip(self.enh.iter()) {
                *d = s.clamp(-1.0, 1.0);
            }
        }
        true
    }
}

/// DeepFilterNet loads in the background the first time it's chosen; RNNoise cleans meanwhile.
enum DeepSlot {
    Off,
    Loading(std::sync::mpsc::Receiver<Option<DeepFilter>>),
    Ready(Box<DeepFilter>),
    Failed,
}

impl MicProcessor {
    fn new(shared: Arc<Shared>, sender: Option<Sender<Vec<u8>>>, self_id: u64, device_rate: u32) -> Self {
        MicProcessor {
            shared,
            sender,
            self_id,
            rs: Resampler::new(device_rate, RATE),
            frame: Vec::with_capacity(FRAME),
            last_voice: Instant::now() - Duration::from_secs(10),
            seq: 0,
            denoiser: nnnoiseless::DenoiseState::new(),
            deep: DeepSlot::Off,
        }
    }

    /// Clean the current piece with the chosen noise suppressor.
    fn clean(&mut self) {
        let shared = self.shared.clone();
        if !shared.noise_suppression.load(Ordering::Relaxed) {
            return;
        }
        if shared.noise_model.load(Ordering::Relaxed) == 1 {
            // Start loading DeepFilterNet if needed, and pick it up once it's ready.
            if matches!(self.deep, DeepSlot::Off) {
                let (tx, rx) = std::sync::mpsc::channel();
                std::thread::Builder::new()
                    .name("hearth-deepfilter-load".into())
                    .spawn(move || {
                        let _ = tx.send(DeepFilter::load());
                    })
                    .ok();
                self.deep = DeepSlot::Loading(rx);
            }
            if let DeepSlot::Loading(rx) = &self.deep {
                match rx.try_recv() {
                    Ok(Some(df)) => self.deep = DeepSlot::Ready(Box::new(df)),
                    Ok(None) | Err(std::sync::mpsc::TryRecvError::Disconnected) => self.deep = DeepSlot::Failed,
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                }
            }
            if let DeepSlot::Ready(df) = &mut self.deep {
                let limit = f(&shared.df_limit_db);
                if df.process(&mut self.frame, limit) {
                    shared.df_ready.store(true, Ordering::Relaxed);
                    return;
                }
                self.deep = DeepSlot::Failed;
            }
        }
        shared.df_ready.store(false, Ordering::Relaxed);
        denoise(&mut self.denoiser, &mut self.frame);
    }

    /// Mono samples at the device's rate (already volume-adjusted).
    fn feed(&mut self, mono: &[f32]) {
        for &s in mono {
            self.rs.push(s);
            while let Some(v) = self.rs.pull() {
                self.frame.push(v.clamp(-1.0, 1.0));
                if self.frame.len() == FRAME {
                    self.piece();
                    self.frame.clear();
                }
            }
        }
    }

    /// One 20 ms piece is ready. Clean it first, then decide whether to send it.
    fn piece(&mut self) {
        let shared = &self.shared;
        self.clean();
        let shared = &self.shared;
        let level = db(rms(&self.frame));
        set_f(&shared.mic_db, level);
        let open = if shared.ptt_mode.load(Ordering::Relaxed) {
            shared.ptt_held.load(Ordering::Relaxed)
        } else {
            if level > f(&shared.threshold_db) {
                self.last_voice = Instant::now();
            }
            self.last_voice.elapsed() < HANGOVER
        };
        // Echo reduction: while others are heard, don't send (avoids sending them back).
        let echo_block =
            shared.echo_reduce.load(Ordering::Relaxed) && f(&shared.others_db) > -45.0 && !shared.ptt_mode.load(Ordering::Relaxed);
        let send = open && !echo_block && !shared.muted.load(Ordering::Relaxed);
        shared.transmitting.store(send, Ordering::Relaxed);
        if open && shared.loopback.load(Ordering::Relaxed) {
            let mut incoming = shared.incoming.lock().unwrap();
            let j = incoming.entry(self.self_id).or_default();
            j.buf.extend(self.frame.iter().copied());
            if j.buf.len() > MAX_BUFFER {
                let extra = j.buf.len() - MAX_BUFFER / 2;
                j.buf.drain(..extra);
            }
        }
        if send {
            if let Some(tx) = &self.sender {
                let mut p = Vec::with_capacity(15 + FRAME * 2);
                p.extend_from_slice(MAGIC);
                p.extend_from_slice(&self.self_id.to_le_bytes());
                p.extend_from_slice(&self.seq.to_le_bytes());
                for s in &self.frame {
                    p.extend_from_slice(&((s * 32767.0) as i16).to_le_bytes());
                }
                let _ = tx.send(p);
                self.seq = self.seq.wrapping_add(1);
            }
        }
    }
}

/// Short glitches (Windows reporting a missed moment of sound) are counted, not shown as errors.
fn note_stream_error(shared: &Shared, what: &str, e: cpal::Error) {
    if e.kind() == cpal::ErrorKind::Xrun {
        shared.glitches.fetch_add(1, Ordering::Relaxed);
    } else {
        *shared.error.lock().unwrap() = Some(format!("{what} problem: {e}"));
    }
}

fn build_input<T>(
    dev: &cpal::Device,
    config: cpal::StreamConfig,
    shared: Arc<Shared>,
    sender: Option<Sender<Vec<u8>>>,
    self_id: u64,
) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = config.channels.max(1) as usize;
    // About 2 s of sound can wait for the processing thread; beyond that the oldest is dropped.
    let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(200);
    {
        let mut proc = MicProcessor::new(shared.clone(), sender, self_id, config.sample_rate);
        std::thread::Builder::new()
            .name("hearth-mic".into())
            .spawn(move || {
                // Ends by itself when the microphone stream is closed (the sender goes away).
                while let Ok(chunk) = rx.recv() {
                    proc.feed(&chunk);
                }
            })
            .map_err(|e| format!("Couldn't start the microphone: {e}"))?;
    }
    let cb_shared = shared.clone();
    let err_shared = shared.clone();
    dev.build_input_stream::<T, _, _>(
        config,
        move |data: &[T], _| {
            // Keep this quick: turn into mono with the volume applied and hand it over.
            let gain = f(&cb_shared.in_gain);
            let mono: Vec<f32> = data
                .chunks(channels)
                .map(|c| c.iter().map(|s| s.to_sample::<f32>()).sum::<f32>() / c.len() as f32 * gain)
                .collect();
            if tx.try_send(mono).is_err() {
                cb_shared.glitches.fetch_add(1, Ordering::Relaxed);
            }
        },
        move |e| note_stream_error(&err_shared, "Microphone", e),
        None,
    )
    .map_err(|e| format!("Couldn't start the microphone: {e}"))
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampler_keeps_length_ratio() {
        let mut rs = Resampler::new(44_100, RATE);
        let mut out = 0;
        for i in 0..44_100 {
            rs.push((i as f32 * 0.01).sin());
            while rs.pull().is_some() {
                out += 1;
            }
        }
        assert!((out as i32 - RATE as i32).abs() < 10, "got {out}");
    }

    /// The microphone processing keeps up easily: 10 s of sound (with noise suppression)
    /// must take far less than 10 s, or Windows would drop microphone sound.
    #[test]
    fn microphone_processing_keeps_up() {
        let audio = Audio::new(7, false);
        let shared = audio.shared.clone();
        shared.noise_suppression.store(true, Ordering::Relaxed);
        set_f(&shared.threshold_db, -60.0);
        let (tx, rx) = channel::<Vec<u8>>();
        let mut proc = MicProcessor::new(shared.clone(), Some(tx), 7, 44_100);
        let tone: Vec<f32> = (0..441).map(|i| (i as f32 * 0.07).sin() * 0.3).collect();
        let start = Instant::now();
        for _ in 0..1000 {
            proc.feed(&tone); // 1000 × 10 ms = 10 s at 44.1 kHz
        }
        let took = start.elapsed();
        assert!(took < Duration::from_secs(2), "10 s of sound took {took:?}");
        // About 500 pieces of 20 ms were sent (voice was loud enough), all well-formed.
        let packets: Vec<Vec<u8>> = rx.try_iter().collect();
        assert!(packets.len() > 450 && packets.len() <= 501, "sent {}", packets.len());
        assert!(packets.iter().all(|p| p.len() == 15 + FRAME * 2 && &p[..3] == MAGIC));
    }

    /// DeepFilterNet turns steady hiss down much further than RNNoise, and fast enough for
    /// live voice (here: 10 s of sound must take under 5 s, measured after loading).
    #[test]
    fn deepfilter_removes_hiss_and_keeps_up() {
        let mut df = DeepFilter::load().expect("DeepFilterNet loads");
        let mut seed: u32 = 777;
        let mut noise = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 0.1
        };
        let (mut before, mut after) = (0.0, 0.0);
        let start = Instant::now();
        for i in 0..500 {
            // 500 × 20 ms = 10 s
            let mut frame: Vec<f32> = (0..FRAME).map(|_| noise()).collect();
            let r_in = rms(&frame);
            assert!(df.process(&mut frame, 100.0));
            if i >= 50 {
                before += r_in;
                after += rms(&frame);
            }
        }
        let took = start.elapsed();
        eprintln!("DeepFilterNet: 10 s of sound in {took:?}; hiss {before:.2} -> {after:.2}");
        assert!(after < before * 0.2, "noise only went from {before} to {after}");
        assert!(took < Duration::from_secs(5), "10 s of sound took {took:?}");
    }

    /// While DeepFilterNet loads, the microphone keeps working (RNNoise cleans meanwhile),
    /// and once loaded it takes over.
    #[test]
    fn deepfilter_takes_over_after_loading() {
        let audio = Audio::new(9, false);
        let shared = audio.shared.clone();
        audio.set_noise_model(1, 100.0);
        let (tx, rx) = channel::<Vec<u8>>();
        let mut proc = MicProcessor::new(shared.clone(), Some(tx), 9, RATE);
        set_f(&shared.threshold_db, -100.0);
        let tone: Vec<f32> = (0..FRAME).map(|i| (i as f32 * 0.05).sin() * 0.2).collect();
        let t = Instant::now();
        while !audio.deepfilter_ready() && t.elapsed() < Duration::from_secs(60) {
            proc.feed(&tone);
        }
        assert!(audio.deepfilter_ready(), "DeepFilterNet never took over");
        assert!(rx.try_iter().count() > 0, "sound kept flowing while it loaded");
    }

    /// Steady hiss (like a fan) is turned down a lot by the noise suppressor.
    #[test]
    fn noise_suppression_removes_steady_hiss() {
        let mut state = nnnoiseless::DenoiseState::new();
        let mut seed: u32 = 12345;
        let mut noise = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 0.1
        };
        let mut before = 0.0;
        let mut after = 0.0;
        // Two seconds of hiss; measure the last second (the suppressor needs a moment to learn).
        for i in 0..100 {
            let mut frame: Vec<f32> = (0..FRAME).map(|_| noise()).collect();
            let r_in = rms(&frame);
            denoise(&mut state, &mut frame);
            if i >= 50 {
                before += r_in;
                after += rms(&frame);
            }
        }
        // White hiss is the hardest case for RNNoise; expect it at least clearly quieter.
        assert!(after < before * 0.6, "noise only went from {before} to {after}");
    }

    #[test]
    fn interface_sounds_are_short_and_not_too_loud() {
        for s in [Sfx::Message, Sfx::Mention, Sfx::Join, Sfx::Leave, Sfx::PeerJoin, Sfx::Mute, Sfx::Deafen, Sfx::LiveStart] {
            let v = synth(s);
            assert!(!v.is_empty() && v.len() < RATE as usize, "{s:?} length");
            assert!(v.iter().all(|x| x.abs() <= 0.6), "{s:?} too loud");
        }
    }

    /// Voice sent by one copy is heard by another: real UDP on this computer.
    #[test]
    fn voice_travels_between_two_copies() {
        let a = Audio::new(1, true);
        let b = Audio::new(2, true);
        // a only accepts from b, b sends to a
        a.set_peers(&[(2, SocketAddr::from(([127, 0, 0, 1], b.voice_port)))]);
        b.set_peers(&[(1, SocketAddr::from(([127, 0, 0, 1], a.voice_port)))]);
        let tone: Vec<f32> = (0..FRAME).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
        let mut p = Vec::new();
        p.extend_from_slice(MAGIC);
        p.extend_from_slice(&2u64.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes());
        for s in &tone {
            p.extend_from_slice(&((s * 32767.0) as i16).to_le_bytes());
        }
        for _ in 0..3 {
            b.sender.as_ref().unwrap().send(p.clone()).unwrap();
        }
        let t = Instant::now();
        while !a.peer_heard_recently(2) && t.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(a.peer_heard_recently(2));
        // Mixed output contains the voice.
        let out = mix(&a.shared, FRAME * 2);
        assert!(rms(&out) > 0.05);
    }

    #[test]
    fn strangers_are_ignored_and_deafen_silences() {
        let a = Audio::new(1, true);
        let b = Audio::new(2, true);
        b.set_peers(&[(1, SocketAddr::from(([127, 0, 0, 1], a.voice_port)))]);
        let mut p = Vec::new();
        p.extend_from_slice(MAGIC);
        p.extend_from_slice(&2u64.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes());
        p.extend(std::iter::repeat_n([0x00u8, 0x40], FRAME).flatten());
        b.sender.as_ref().unwrap().send(p).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert!(!a.peer_heard_recently(2), "a never allowed 2");

        // Deafened: voices are not mixed in.
        push_voice(&a.shared, 9, &vec![0.5; FRAME * 3]);
        a.shared.deafened.store(true, Ordering::Relaxed);
        assert!(rms(&mix(&a.shared, FRAME)) < 1e-6);
    }
}
