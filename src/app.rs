//! Hearth: the window, its panels and all the interaction.

use crate::audio::{self, Audio, Sfx};
use crate::media::{self, Capture, Choice, Source, StreamSettings, Viewer, What};
use crate::model::*;
use crate::net::{Lan, Packet, StreamAd};
use crate::theme::{self, Palette};
use crate::update;
use chrono::{Datelike, Local, TimeZone};
use egui::{
    pos2, vec2, Align, Align2, Color32, CornerRadius, FontId, Frame, Id as EId, Key, Layout, Margin, Pos2, Rect,
    RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder,
};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

// Layout sizes (estimated: the original's chat screen is loaded from the web, see MECHANICS.md).
pub const RAIL_W: f32 = 70.0;
pub const SIDE_W: f32 = 236.0;
pub const MEMBERS_W: f32 = 228.0;
pub const HEADER_H: f32 = 48.0;
pub const ICON: f32 = 46.0;
pub const AVATAR: f32 = 38.0;

pub const EMOJI: &[&str] = &[
    "😀", "😂", "😊", "😍", "😎", "🤔", "😮", "😢", "😡", "😴", "👍", "👎", "👏", "🙌", "🙏", "💪", "🤝", "👀", "🎉", "🔥",
    "✨", "💯", "❤", "💜", "💚", "💙", "⭐", "☕", "🍕", "🎮", "📚", "🎨", "🌱", "🌙", "☀", "🎵", "🏃", "✅", "❌", "👋",
];

const SPLASH_TIPS: &[&str] = &[
    "Warming up the fireplace…",
    "Tip: press ↑ in an empty message box to edit your last message.",
    "Tip: Shift+Enter starts a new line.",
    "Tip: wrap words in **stars** to make them stand out.",
    "Tip: everyone on your home network can join the Home Network community.",
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Channel(Id, Id),
    Dm(Id),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FriendsTab {
    Online,
    All,
    Pending,
    Add,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsPage {
    Account,
    Appearance,
    Chat,
    Notifications,
    Voice,
    Accessibility,
    Privacy,
    Shortcuts,
    Share,
    Network,
    Updates,
    Data,
}

enum Dialog {
    NewCommunity { name: String, hue: u8 },
    NewChannel { community: Id, name: String, kind: ChannelKind },
    RenameCommunity { community: Id, name: String },
    EditTopic { community: Id, channel: Id, topic: String },
    LeaveCommunity { community: Id },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PickerFor {
    Composer,
    Reaction(Id),
}

struct PendingReply {
    due: f64,
    typing_from: f64,
    author: Id,
    target: Target,
    text: String,
    react_to: Option<Id>,
}

enum MsgAction {
    React(Id, String),
    Reply(Id),
    StartEdit(Id),
    SaveEdit(Id),
    CancelEdit,
    Delete(Id),
    Pin(Id),
    Profile(Id, Pos2),
    Picker(Id, Pos2),
    Jump(Id),
}

pub struct ScreenshotJob {
    pub path: std::path::PathBuf,
    pub frames: u32,
    pub requested: bool,
    pub slot: Arc<Mutex<Option<Arc<egui::ColorImage>>>>,
}

pub struct HearthApp {
    data: Data,
    community: Option<Id>, // None = home
    channel_for: HashMap<Id, Id>,
    dm: Option<Id>,
    friends_tab: FriendsTab,
    find_dm: String,
    friend_name: String,
    add_friend_result: Option<(bool, String)>,
    composer: String,
    reply_to: Option<Id>,
    editing: Option<(Id, String)>,
    search: String,
    search_query: String,
    show_pins: bool,
    picker: Option<(PickerFor, Pos2)>,
    profile: Option<(Id, Pos2)>,
    dialog: Option<Dialog>,
    settings_open: bool,
    settings_page: SettingsPage,
    voice: Option<(Id, Id)>,
    mic_muted: bool,
    deafened: bool,
    pending: Vec<PendingReply>,
    collapsed: HashSet<(Id, bool)>,
    lan: Option<Lan>,
    lan_error: Option<String>,
    lan_seen: HashMap<u64, f64>,
    last_hello: f64,
    instance: u64,
    rng: u64,
    splash_until: f64,
    splash_tip: usize,
    applied_theme: Option<(Theme, u8, u32, bool)>,
    dirty: bool,
    last_save: f64,
    jump_to: Option<Id>,
    focus_composer: bool,
    pub screenshot: Option<ScreenshotJob>,
    save_path: Option<std::path::PathBuf>,
    applied_on_top: Option<bool>,
    attention: bool,
    /// Lets streaming threads ask for a redraw the moment a picture arrives.
    egui_ctx: Option<egui::Context>,
    // Screen sharing and video
    share_screen: Option<Capture>,
    camera: Option<Capture>,
    viewers: HashMap<(Id, Source), Viewer>,
    textures: HashMap<String, (egui::TextureHandle, u64)>,
    peers: HashMap<Id, PeerInfo>,
    show_stage: bool,
    focused_tile: Option<String>,
    go_live: Option<GoLive>,
    // Sound
    audio: Audio,
    mic_test: bool,
    window_focused: bool,
    device_lists: Option<(Vec<String>, Vec<String>)>,
    recording_ptt: bool,
    prev_people: Vec<Id>,
    // Updates
    updater: update::Updater,
    last_update_check: f64,
    update_banner_hidden: bool,
    // Animations
    view_key: u64,
    view_changed_at: f64,
    msg_born: HashMap<Id, f64>,
    popup_opened_at: f64,
    popup_key: u64,
    animating: bool,
    now: f64,
    banner_since: Option<f64>,
    update_failed: Option<String>,
}

/// The "Go live" picker: what to share and how.
struct GoLive {
    source: Source,
    /// 0 = whole screens, 1 = app windows (screen sharing only).
    tab: u8,
    screens: Vec<Choice>,
    windows: Vec<Choice>,
    cameras: Vec<Choice>,
    selected: Option<What>,
    quality: u8,
    fps: u8,
    /// Already live: "Apply" changes the running stream.
    changing: bool,
}

/// What a home-network peer last told us about themselves.
struct PeerInfo {
    instance: u64,
    voice_port: u16,
    ip: std::net::IpAddr,
    voice: String,
    streams: Vec<(Source, u16)>,
}

/// One box on the voice stage.
struct Tile {
    key: String,
    user: Id,
    label: String,
    stream: Option<(Id, Source)>, // whose picture, and which one (ME = our own preview)
}

fn now_secs() -> i64 {
    Local::now().timestamp()
}

fn state_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("APPDATA").or_else(|| std::env::var_os("HOME"))?;
    Some(std::path::PathBuf::from(base).join("Hearth").join("state.json"))
}

impl HearthApp {
    pub fn new(screenshot: Option<ScreenshotJob>) -> Self {
        let demo = screenshot.is_some();
        let save_path = if demo { None } else { state_path() };
        let data = save_path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<Data>(&b).ok())
            .unwrap_or_else(|| sample(now_secs(), "You"));
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9)
            | 1;
        let mut app = HearthApp {
            data,
            community: Some(101),
            channel_for: HashMap::new(),
            dm: None,
            friends_tab: FriendsTab::Online,
            find_dm: String::new(),
            friend_name: String::new(),
            add_friend_result: None,
            composer: String::new(),
            reply_to: None,
            editing: None,
            search: String::new(),
            search_query: String::new(),
            show_pins: false,
            picker: None,
            profile: None,
            dialog: None,
            settings_open: false,
            settings_page: SettingsPage::Account,
            voice: None,
            mic_muted: false,
            deafened: false,
            pending: vec![],
            collapsed: HashSet::new(),
            lan: None,
            lan_error: None,
            lan_seen: HashMap::new(),
            last_hello: -100.0,
            instance: seed,
            rng: seed,
            splash_until: if demo { 0.0 } else { 1.6 },
            splash_tip: (seed % SPLASH_TIPS.len() as u64) as usize,
            applied_theme: None,
            dirty: false,
            last_save: 0.0,
            jump_to: None,
            focus_composer: true,
            screenshot,
            save_path,
            applied_on_top: None,
            attention: false,
            egui_ctx: None,
            share_screen: None,
            camera: None,
            viewers: HashMap::new(),
            textures: HashMap::new(),
            peers: HashMap::new(),
            show_stage: false,
            focused_tile: None,
            go_live: None,
            audio: Audio::new(seed, !demo),
            mic_test: false,
            window_focused: true,
            device_lists: None,
            recording_ptt: false,
            prev_people: vec![],
            updater: update::Updater::new(),
            last_update_check: f64::NEG_INFINITY,
            update_banner_hidden: false,
            view_key: 0,
            view_changed_at: 0.0,
            msg_born: HashMap::new(),
            popup_opened_at: 0.0,
            popup_key: 0,
            animating: false,
            now: 0.0,
            banner_since: None,
            update_failed: None,
        };
        if !demo {
            // A previous update leaves the old program behind; tidy it up.
            update::cleanup_after_update();
        }
        // Nobody is in a voice channel when the app starts.
        for c in app.data.communities.iter_mut() {
            for ch in c.channels.iter_mut() {
                ch.voice_people.retain(|id| *id != ME && *id < 1_000_000);
            }
        }
        // The Home Network community gets a voice channel for video and screen sharing.
        let lounge_id = app.data.next_id + 1;
        if let Some(c) = app.data.community_mut(HOME_NET_COMMUNITY) {
            if !c.channels.iter().any(|x| x.kind == ChannelKind::Voice) {
                c.channels.push(Channel {
                    id: lounge_id,
                    name: "Lounge".into(),
                    kind: ChannelKind::Voice,
                    topic: String::new(),
                    messages: vec![],
                    unread: 0,
                    mentions: 0,
                    voice_people: vec![],
                });
                app.data.next_id = lounge_id;
            }
        }
        // The three sample communities from earlier versions are gone: only Home Network
        // (and any community the person made themselves) stays.
        let before = app.data.communities.len();
        app.data.communities.retain(|c| !matches!(c.id, 101..=103));
        if app.data.communities.len() != before {
            app.dirty = true;
        }
        if app.data.community(HOME_NET_COMMUNITY).is_none() {
            // Saves from very early versions: bring Home Network back.
            let fresh = sample(now_secs(), "You");
            if let Some(home) = fresh.communities.into_iter().find(|c| c.id == HOME_NET_COMMUNITY) {
                app.data.next_id = app.data.next_id.max(fresh.next_id);
                app.data.communities.insert(0, home);
                app.dirty = true;
            }
        }
        // Open Home Network's lobby.
        app.community = Some(HOME_NET_COMMUNITY);
        if let Some(c) = app.data.community(HOME_NET_COMMUNITY) {
            if let Some(ch) = c.channels.iter().find(|c| c.name == "lobby") {
                app.channel_for.insert(HOME_NET_COMMUNITY, ch.id);
            }
        }
        // Peers from an earlier session aren't around any more.
        for u in app.data.users.iter_mut().filter(|u| u.id >= 1_000_000) {
            u.status = Status::Hidden;
        }
        if app.data.settings.skip_splash {
            app.splash_until = 0.0;
        }
        // Test aid: HEARTH_OPEN=chat|notifications|accessibility|privacy|shortcuts opens that settings page.
        if let Ok(page) = std::env::var("HEARTH_OPEN") {
            let pg = match page.as_str() {
                "account" => Some(SettingsPage::Account),
                "appearance" => Some(SettingsPage::Appearance),
                "chat" => Some(SettingsPage::Chat),
                "notifications" => Some(SettingsPage::Notifications),
                "accessibility" => Some(SettingsPage::Accessibility),
                "privacy" => Some(SettingsPage::Privacy),
                "shortcuts" => Some(SettingsPage::Shortcuts),
                "share" => Some(SettingsPage::Share),
                "voice" => Some(SettingsPage::Voice),
                "updates" => Some(SettingsPage::Updates),
                _ => None,
            };
            if let Some(pg) = pg {
                app.settings_open = true;
                app.settings_page = pg;
            }
            if page == "stage" || page == "golive" {
                let room = app
                    .data
                    .community(HOME_NET_COMMUNITY)
                    .and_then(|c| c.channels.iter().find(|x| x.kind == ChannelKind::Voice).map(|x| x.id));
                if let Some(room) = room {
                    app.join_voice(HOME_NET_COMMUNITY, room);
                }
                if page == "golive" {
                    // Opens on the "Whole screen" tab, which lists screens only (no window titles).
                    app.open_go_live(Source::Screen, false);
                }
            }
        }
        if !demo && app.data.settings.lan_enabled {
            app.start_lan();
        }
        app.mark_read();
        app
    }

    /// Window-level settings that are applied once when they change.
    fn apply_window_settings(&mut self, ctx: &egui::Context) {
        let s = &self.data.settings;
        CLOCK_24H.store(s.clock_24h, std::sync::atomic::Ordering::Relaxed);
        let on_top = s.always_on_top && self.screenshot.is_none();
        if self.applied_on_top != Some(on_top) {
            self.applied_on_top = Some(on_top);
            if self.screenshot.is_none() {
                ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(if on_top {
                    egui::WindowLevel::AlwaysOnTop
                } else {
                    egui::WindowLevel::Normal
                }));
            }
        }
    }

    fn rand(&mut self) -> u64 {
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn rand_f(&mut self) -> f64 {
        (self.rand() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn pick<'a>(&mut self, list: &[&'a str]) -> &'a str {
        let i = (self.rand() % list.len() as u64) as usize;
        list[i]
    }

    fn start_lan(&mut self) {
        match Lan::start(self.data.settings.lan_port, self.instance) {
            Ok(l) => {
                self.lan = Some(l);
                self.lan_error = None;
                self.last_hello = -100.0;
            }
            Err(e) => {
                self.lan = None;
                self.lan_error = Some(e);
            }
        }
    }

    // ------------------------------------------------------------ navigation

    fn current_target(&self) -> Option<Target> {
        match self.community {
            None => self.dm.map(Target::Dm),
            Some(c) => self.current_channel(c).map(|ch| Target::Channel(c, ch)),
        }
    }

    fn current_channel(&self, community: Id) -> Option<Id> {
        let c = self.data.community(community)?;
        if let Some(id) = self.channel_for.get(&community) {
            if c.channels.iter().any(|ch| ch.id == *id && ch.kind == ChannelKind::Text) {
                return Some(*id);
            }
        }
        c.channels.iter().find(|ch| ch.kind == ChannelKind::Text).map(|ch| ch.id)
    }

    fn messages(&self, t: Target) -> Option<&Vec<Message>> {
        match t {
            Target::Channel(c, ch) => self.data.community(c)?.channels.iter().find(|x| x.id == ch).map(|x| &x.messages),
            Target::Dm(u) => self.data.dms.iter().find(|d| d.with == u).map(|d| &d.messages),
        }
    }

    fn messages_mut(&mut self, t: Target) -> Option<&mut Vec<Message>> {
        match t {
            Target::Channel(c, ch) => {
                self.data.community_mut(c)?.channels.iter_mut().find(|x| x.id == ch).map(|x| &mut x.messages)
            }
            Target::Dm(u) => Some(&mut self.data.dm_mut(u).messages),
        }
    }

    fn mark_read(&mut self) {
        match self.current_target() {
            Some(Target::Channel(c, ch)) => {
                if let Some(x) = self.data.community_mut(c).and_then(|c| c.channels.iter_mut().find(|x| x.id == ch)) {
                    x.unread = 0;
                    x.mentions = 0;
                }
            }
            Some(Target::Dm(u)) => {
                if let Some(d) = self.data.dms.iter_mut().find(|d| d.with == u) {
                    d.unread = 0;
                }
            }
            None => {}
        }
    }

    fn open_channel(&mut self, community: Id, channel: Id) {
        self.community = Some(community);
        self.channel_for.insert(community, channel);
        self.after_switch();
    }

    fn open_community(&mut self, community: Id) {
        self.community = Some(community);
        self.after_switch();
    }

    fn open_dm(&mut self, user: Id) {
        self.community = None;
        self.data.dm_mut(user);
        self.dm = Some(user);
        self.after_switch();
    }

    fn open_home(&mut self) {
        self.community = None;
        self.dm = None;
        self.after_switch();
    }

    fn after_switch(&mut self) {
        self.reply_to = None;
        self.editing = None;
        self.show_pins = false;
        self.search.clear();
        self.search_query.clear();
        self.profile = None;
        self.picker = None;
        self.focus_composer = true;
        self.mark_read();
    }

    fn display_name(&self, m: &Message) -> String {
        if let Some(n) = &m.author_name {
            return n.clone();
        }
        self.data.user(m.author).map(|u| u.name.clone()).unwrap_or_else(|| "Someone".into())
    }

    fn author_hue(&self, m: &Message) -> u8 {
        self.data.user(m.author).map(|u| u.hue).unwrap_or((m.author % 8) as u8)
    }

    fn mentions_me(&self, text: &str) -> bool {
        let me = format!("@{}", self.data.me().name.to_lowercase());
        let t = text.to_lowercase();
        t.contains(&me) || t.contains("@everyone")
    }

    // ------------------------------------------------------------ sending

    fn send(&mut self, now: f64) {
        let text = self.composer.trim().to_string();
        if text.is_empty() {
            return;
        }
        let Some(target) = self.current_target() else { return };
        let text: String = text.chars().take(MAX_MESSAGE_CHARS).collect();
        let id = self.data.new_id();
        self.msg_born.insert(id, self.now);
        let m = Message {
            id,
            author: ME,
            author_name: None,
            text: text.clone(),
            ts: now_secs(),
            edited: false,
            reactions: vec![],
            reply_to: self.reply_to.take(),
            pinned: false,
        };
        if let Some(list) = self.messages_mut(target) {
            list.push(m);
        }
        self.composer.clear();
        self.dirty = true;
        self.jump_to = None;

        if let Target::Channel(c, ch) = target {
            if c == HOME_NET_COMMUNITY {
                let channel = self.channel_name(c, ch);
                let name = self.data.me().name.clone();
                if let Some(lan) = &self.lan {
                    lan.send(&Packet::Say { from: self.instance, name, channel, text, id });
                }
                return;
            }
        }
        if self.data.settings.sample_replies {
            self.schedule_reply(target, &text, id, now);
        }
    }

    fn channel_name(&self, c: Id, ch: Id) -> String {
        self.data
            .community(c)
            .and_then(|c| c.channels.iter().find(|x| x.id == ch))
            .map(|x| x.name.clone())
            .unwrap_or_default()
    }

    /// A sample person answers a little later, with a typing indicator first.
    fn schedule_reply(&mut self, target: Target, text: &str, msg_id: Id, now: f64) {
        let lower = text.to_lowercase();
        let candidates: Vec<Id> = match target {
            Target::Dm(u) => vec![u],
            Target::Channel(c, _) => {
                let members = self.data.community(c).map(|c| c.members.clone()).unwrap_or_default();
                let named: Vec<Id> = members
                    .iter()
                    .copied()
                    .filter(|id| *id != ME)
                    .filter(|id| {
                        self.data
                            .user(*id)
                            .map(|u| lower.contains(&format!("@{}", u.name.to_lowercase())))
                            .unwrap_or(false)
                    })
                    .collect();
                if !named.is_empty() {
                    named
                } else {
                    members.into_iter().filter(|id| *id != ME).collect()
                }
            }
        };
        let awake: Vec<Id> = candidates
            .into_iter()
            .filter(|id| {
                self.data.user(*id).map(|u| matches!(u.status, Status::Online | Status::Away)).unwrap_or(false)
            })
            .collect();
        if awake.is_empty() || self.pending.iter().any(|p| p.target == target) {
            return;
        }
        let author = awake[(self.rand() % awake.len() as u64) as usize];
        let greeting = ["hi", "hey", "hello", "yo", "morning", "evening"]
            .iter()
            .any(|g| lower.split(|c: char| !c.is_alphanumeric()).any(|w| w == *g));
        let reply = if greeting {
            self.pick(GREETINGS)
        } else if lower.trim_end().ends_with('?') {
            self.pick(ANSWERS)
        } else {
            self.pick(REPLIES)
        }
        .to_string();
        // Relaxed / normal / quick (normal: 1.6–4.0 s).
        let factor = match self.data.settings.reply_speed {
            0 => 2.0,
            2 => 0.45,
            _ => 1.0,
        };
        let delay = (1.6 + self.rand_f() * 2.4) * factor;
        let react_to = if self.rand() % 3 == 0 { Some(msg_id) } else { None };
        self.pending.push(PendingReply {
            due: now + delay,
            typing_from: now + 0.6 * factor.min(1.0),
            author,
            target,
            text: reply,
            react_to,
        });
    }

    fn deliver(&mut self, target: Target, m: Message) {
        self.msg_born.insert(m.id, self.now);
        if self.msg_born.len() > 500 {
            let now = self.now;
            self.msg_born.retain(|_, t| now - *t < 5.0);
        }
        let viewing = self.current_target() == Some(target) && !self.settings_open;
        let mention = m.author != ME && self.mentions_me(&m.text);
        let from_someone_else = m.author != ME;
        match target {
            Target::Channel(c, ch) => {
                if let Some(x) = self.data.community_mut(c).and_then(|c| c.channels.iter_mut().find(|x| x.id == ch)) {
                    x.messages.push(m);
                    if !viewing {
                        x.unread += 1;
                        if mention {
                            x.mentions += 1;
                        }
                    }
                }
                let muted = self.data.community(c).map(|c| c.muted).unwrap_or(false);
                let s = &self.data.settings;
                if s.flash_on_message && !muted && (!s.mentions_only || mention) {
                    self.attention = true;
                }
                // A sound when you aren't already looking at it.
                if from_someone_else && !muted && (!viewing || !self.window_focused) {
                    self.sfx(if mention { Sfx::Mention } else { Sfx::Message });
                }
            }
            Target::Dm(u) => {
                let d = self.data.dm_mut(u);
                d.messages.push(m);
                if !viewing {
                    d.unread += 1;
                }
                if self.data.settings.flash_on_message {
                    self.attention = true;
                }
                if from_someone_else && (!viewing || !self.window_focused) {
                    self.sfx(Sfx::Message);
                }
                if let Some(i) = self.data.dms.iter().position(|d| d.with == u) {
                    let t = self.data.dms.remove(i);
                    self.data.dms.insert(0, t);
                }
            }
        }
        self.dirty = true;
    }

    fn tick(&mut self, now: f64) {
        let mut due = vec![];
        self.pending.retain(|p| {
            if p.due <= now {
                due.push((p.author, p.target, p.text.clone(), p.react_to));
                false
            } else {
                true
            }
        });
        for (author, target, text, react_to) in due {
            if let Some(mid) = react_to {
                let e = if self.rand() % 2 == 0 { "👍" } else { "😄" };
                self.toggle_reaction(target, mid, e, author);
            }
            let id = self.data.new_id();
            self.deliver(
                target,
                Message {
                    id,
                    author,
                    author_name: None,
                    text,
                    ts: now_secs(),
                    edited: false,
                    reactions: vec![],
                    reply_to: None,
                    pinned: false,
                },
            );
        }

        if let Some(lan) = &self.lan {
            if now - self.last_hello > 4.0 {
                let me = self.data.me();
                let s = &self.data.settings;
                let status = if s.share_status_lan { me.status } else { Status::Hidden };
                let activity = if s.share_activity { me.activity.clone() } else { String::new() };
                // Voice channel and streams are only shared from the Home Network community.
                let (voice, streams) = match self.voice {
                    Some((c, ch)) if c == HOME_NET_COMMUNITY => {
                        let name = self
                            .data
                            .community(c)
                            .and_then(|c| c.channels.iter().find(|x| x.id == ch))
                            .map(|x| x.name.clone())
                            .unwrap_or_default();
                        let streams: Vec<StreamAd> = [&self.share_screen, &self.camera]
                            .into_iter()
                            .flatten()
                            .filter(|cap| cap.port != 0)
                            .map(|cap| StreamAd { kind: cap.source.tag().into(), port: cap.port })
                            .collect();
                        (name, streams)
                    }
                    _ => (String::new(), vec![]),
                };
                lan.send(&Packet::Hello {
                    from: self.instance,
                    name: me.name.clone(),
                    status: status.label().into(),
                    activity,
                    voice,
                    streams,
                    voice_port: self.audio.voice_port,
                });
                self.last_hello = now;
            }
            let packets = lan.poll();
            for (p, ip) in packets {
                self.on_packet(p, ip, now);
            }
        }
        // Peers not heard from in 12 s are shown offline (estimated timeout).
        let stale: Vec<u64> = self.lan_seen.iter().filter(|(_, t)| now - **t > 12.0).map(|(k, _)| *k).collect();
        for k in stale {
            self.lan_seen.remove(&k);
            let uid = peer_user_id(k);
            self.peers.remove(&uid);
            if let Some(u) = self.data.user_mut(uid) {
                u.status = Status::Hidden;
            }
        }
        self.sync_lan_voice();
    }

    /// Put home-network peers into the Home Network voice channel they say they're in,
    /// and watch the streams of everyone in our own voice channel.
    fn sync_lan_voice(&mut self) {
        let me_in = self.voice;
        let peers: Vec<(Id, String)> = self.peers.iter().map(|(id, p)| (*id, p.voice.clone())).collect();
        if let Some(c) = self.data.community_mut(HOME_NET_COMMUNITY) {
            for ch in c.channels.iter_mut().filter(|x| x.kind == ChannelKind::Voice) {
                let mut people: Vec<Id> = vec![];
                if me_in == Some((HOME_NET_COMMUNITY, ch.id)) {
                    people.push(ME);
                }
                let mut here: Vec<Id> = peers.iter().filter(|(_, v)| *v == ch.name).map(|(id, _)| *id).collect();
                here.sort();
                people.extend(here);
                ch.voice_people = people;
            }
        }
        // Which streams should be watched now?
        let mut wanted: Vec<((Id, Source), std::net::IpAddr, u16)> = vec![];
        if let Some((c, ch)) = self.voice {
            if c == HOME_NET_COMMUNITY {
                let name = self.channel_name(c, ch);
                for (id, info) in &self.peers {
                    if info.voice == name {
                        for (src, port) in &info.streams {
                            wanted.push(((*id, *src), info.ip, *port));
                        }
                    }
                }
            }
        }
        self.viewers.retain(|k, _| wanted.iter().any(|(w, _, _)| w == k));
        for (key, ip, port) in wanted {
            let repaint = self.egui_ctx.clone();
            self.viewers.entry(key).or_insert_with(|| Viewer::start(ip, port, repaint));
        }

        // Voice: talk with (and only accept voices from) the people in our Home Network voice channel.
        let mut voice_peers: Vec<(u64, std::net::SocketAddr)> = vec![];
        let mut my_people: Vec<Id> = vec![];
        if let Some((c, ch)) = self.voice {
            my_people = self
                .data
                .community(c)
                .and_then(|x| x.channels.iter().find(|x| x.id == ch))
                .map(|x| x.voice_people.clone())
                .unwrap_or_default();
            if c == HOME_NET_COMMUNITY {
                let name = self.channel_name(c, ch);
                for (uid, info) in &self.peers {
                    if info.voice == name && info.voice_port != 0 {
                        voice_peers.push((info.instance, std::net::SocketAddr::new(info.ip, info.voice_port)));
                        let user_name = self.data.user(*uid).map(|u| u.name.clone()).unwrap_or_default();
                        let gain = self.data.settings.user_volumes.get(&user_name).copied().unwrap_or(1.0);
                        self.audio.set_peer_gain(info.instance, gain);
                    }
                }
            }
        }
        self.audio.set_peers(&voice_peers);
        // Friends arriving in or leaving our voice channel get a little sound.
        if self.voice.is_some() {
            let joined = my_people.iter().any(|id| *id != ME && !self.prev_people.contains(id));
            let left = self.prev_people.iter().any(|id| *id != ME && !my_people.contains(id));
            if joined {
                self.sfx(Sfx::PeerJoin);
            }
            if left {
                self.sfx(Sfx::PeerLeave);
            }
        }
        self.prev_people = my_people;
    }

    /// Play an interface sound if the person allows that kind of sound.
    fn sfx(&mut self, s: Sfx) {
        let st = &self.data.settings;
        if !st.ui_sounds {
            return;
        }
        let allowed = match s {
            Sfx::Message => st.sound_messages,
            Sfx::Mention => st.sound_mentions,
            Sfx::Join | Sfx::Leave | Sfx::PeerJoin | Sfx::PeerLeave => st.sound_voice,
            _ => st.sound_controls,
        };
        if allowed {
            let dev = st.output_device.clone();
            self.audio.play(s, dev.as_deref());
        }
    }

    /// Is this person talking right now (real for you and home-network friends)?
    fn speaking(&self, id: Id, now: f64, k: usize) -> bool {
        if self.data.settings.reduced_motion {
            return false;
        }
        if id == ME {
            return self.voice.is_some() && self.audio.transmitting();
        }
        if id >= 1_000_000 {
            return self.peers.get(&id).map(|p| self.audio.peer_speaking(p.instance)).unwrap_or(false);
        }
        (now * 1.3 + k as f64 * 2.1 + id as f64).sin() > 0.5 // sample people
    }

    fn toggle_mute(&mut self) {
        self.mic_muted = !self.mic_muted;
        if !self.mic_muted {
            self.deafened = false;
        }
        self.sfx(if self.mic_muted { Sfx::Mute } else { Sfx::Unmute });
    }

    fn toggle_deafen(&mut self) {
        self.deafened = !self.deafened;
        if self.deafened {
            self.mic_muted = true;
        }
        self.sfx(if self.deafened { Sfx::Deafen } else { Sfx::Undeafen });
    }

    /// Animation progress (0 → 1, eased) for something that started at `since`.
    /// Instant when "Reduce motion" is on. Keeps the window redrawing until it finishes.
    fn ease(&mut self, since: f64, dur: f32) -> f32 {
        if self.data.settings.reduced_motion || self.screenshot.is_some() {
            return 1.0;
        }
        let t = (((self.now - since) as f32) / dur).clamp(0.0, 1.0);
        if t < 1.0 {
            self.animating = true;
        }
        egui::emath::easing::cubic_out(t)
    }

    /// Notice when the person moves to another screen, so the new one can fade in.
    fn track_view(&mut self) {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (self.community, self.current_channel(self.community.unwrap_or(0)), self.dm, self.settings_open, self.show_stage && self.voice.is_some())
            .hash(&mut h);
        if self.settings_open {
            (self.settings_page as u8).hash(&mut h);
        }
        let key = h.finish();
        if key != self.view_key {
            self.view_key = key;
            self.view_changed_at = self.now;
        }
        // Pop-ups (profile card, emoji picker, picker dialogs) fade and grow in when they open.
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (self.profile.map(|p| p.0), self.picker.map(|p| matches!(p.0, PickerFor::Composer)), self.picker.is_some(), self.dialog.is_some(), self.go_live.is_some())
            .hash(&mut h);
        let pk = h.finish();
        if pk != self.popup_key {
            self.popup_key = pk;
            self.popup_opened_at = self.now;
        }
    }

    /// Check for updates now and then (and download them if automatic install is on).
    fn schedule_updates(&mut self) {
        if self.screenshot.is_some() || !self.data.settings.auto_update_check {
            return;
        }
        // First check 5 s after start, then every 6 hours (estimated).
        let due = if self.last_update_check.is_infinite() { self.now > 5.0 } else { self.now - self.last_update_check > 6.0 * 3600.0 };
        if due {
            self.last_update_check = self.now;
            self.updater.check(self.data.settings.auto_update_install);
        }
    }

    fn restart_to_update(&mut self, ctx: &egui::Context) {
        self.save_now();
        match self.updater.install() {
            Ok(program) => match update::restart(&program) {
                Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Err(e) => self.update_error(e),
            },
            Err(e) => self.update_error(e),
        }
    }

    fn update_error(&mut self, e: String) {
        self.updater = update::Updater::new();
        self.update_banner_hidden = false;
        self.last_update_check = self.now; // try again in a while
        self.dirty = true;
        eprintln!("update: {e}");
        self.update_failed = Some(e);
    }

    /// The "new version ready" note that slides down from the top.
    fn update_banner(&mut self, ctx: &egui::Context, p: &Palette) {
        if self.update_banner_hidden {
            return;
        }
        let (text, button) = match self.updater.state() {
            update::State::Ready { version, .. } => (format!("Hearth {version} is ready."), Some("Restart to update")),
            update::State::Available { version, .. } if !self.data.settings.auto_update_install => {
                (format!("Hearth {version} is available."), Some("Download"))
            }
            update::State::Downloading { version, done, total } if total > 0 => {
                (format!("Downloading Hearth {version}… {}%", done * 100 / total.max(1)), None)
            }
            _ => {
                self.banner_since = None;
                return;
            }
        };
        let since = *self.banner_since.get_or_insert(self.now);
        let t = self.ease(since, 0.35);
        let screen = ctx.content_rect();
        let mut restart = false;
        let mut download = false;
        let mut hide = false;
        egui::Area::new(EId::new("update_banner"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2(screen.center().x - 230.0, 8.0 - (1.0 - t) * 60.0))
            .show(ctx, |ui| {
                ui.multiply_opacity(t);
                Frame::new()
                    .fill(p.raised)
                    .corner_radius(10.0)
                    .stroke(Stroke::new(1.0, p.accent))
                    .inner_margin(Margin::symmetric(14, 10))
                    .shadow(egui::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(90) })
                    .show(ui, |ui| {
                        ui.set_width(432.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("⬆").color(p.accent).size(16.0));
                            ui.label(RichText::new(&text).color(p.strong));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.add(egui::Button::new(RichText::new("Later").color(p.muted)).frame(false)).clicked() {
                                    hide = true;
                                }
                                if let Some(b) = button {
                                    if pill_button(ui, b, p.accent, p.on_accent).clicked() {
                                        if b == "Download" {
                                            download = true;
                                        } else {
                                            restart = true;
                                        }
                                    }
                                }
                            });
                        });
                    });
            });
        if hide {
            self.update_banner_hidden = true;
        }
        if download {
            self.updater.download();
        }
        if restart {
            self.restart_to_update(ctx);
        }
    }

    /// Keep the sound engine in step with the settings and what's happening.
    fn update_audio(&mut self, ctx: &egui::Context) {
        let s = self.data.settings.clone();
        let ptt_key = egui::Key::from_name(&s.ptt_key);
        let ptt_held = !self.recording_ptt && self.window_focused && ptt_key.map(|k| ctx.input(|i| i.key_down(k))).unwrap_or(false);
        self.audio.set_controls(
            s.input_volume,
            s.output_volume,
            s.ui_volume,
            s.mic_sensitivity_db,
            s.push_to_talk,
            ptt_held,
            self.mic_muted || self.deafened,
            self.deafened,
            s.echo_reduction,
        );
        self.audio.set_loopback(self.mic_test);
        self.audio.set_noise_suppression(s.noise_suppression);
        let want_mic = self.voice.is_some() || self.mic_test;
        if want_mic {
            self.audio.ensure_output(s.output_device.as_deref());
            self.audio.start_mic(s.input_device.as_deref());
        } else if self.audio.mic_open() {
            self.audio.stop_mic();
        }
    }

    fn on_packet(&mut self, p: Packet, ip: std::net::IpAddr, now: f64) {
        let (from, name) = match &p {
            Packet::Hello { from, name, .. } | Packet::Say { from, name, .. } => (*from, name.clone()),
        };
        let name: String = name.chars().take(32).collect();
        let uid = peer_user_id(from);
        self.lan_seen.insert(from, now);
        if let Packet::Hello { voice, streams, voice_port, .. } = &p {
            let mut list: Vec<(Source, u16)> = streams
                .iter()
                .filter_map(|s| Source::from_tag(&s.kind).map(|src| (src, s.port)))
                .filter(|(_, port)| *port != 0)
                .collect();
            list.truncate(2);
            // A stream whose port changed is a new stream: reconnect.
            if let Some(old) = self.peers.get(&uid) {
                for (src, port) in &old.streams {
                    if !list.contains(&(*src, *port)) {
                        self.viewers.remove(&(uid, *src));
                    }
                }
            }
            self.peers.insert(
                uid,
                PeerInfo { instance: from, voice_port: *voice_port, ip, voice: voice.chars().take(40).collect(), streams: list },
            );
        }
        match self.data.user_mut(uid) {
            Some(u) => {
                u.name = name.clone();
                if let Packet::Hello { status, activity, .. } = &p {
                    u.status = Status::ALL.iter().copied().find(|s| s.label() == status).unwrap_or(Status::Online);
                    u.activity = activity.chars().take(60).collect();
                } else if u.status == Status::Hidden {
                    u.status = Status::Online;
                }
            }
            None => {
                self.data.users.push(User {
                    id: uid,
                    name: name.clone(),
                    status: Status::Online,
                    hue: (from % 8) as u8,
                    about: "On your home network.".into(),
                    activity: "Home network".into(),
                    friend: false,
                    pending_in: false,
                    pending_out: false,
                });
            }
        }
        if let Some(c) = self.data.community_mut(HOME_NET_COMMUNITY) {
            if !c.members.contains(&uid) {
                c.members.push(uid);
            }
        }
        if let Packet::Say { channel, text, .. } = p {
            let channel: String = channel.chars().take(40).collect();
            let existing = self
                .data
                .community(HOME_NET_COMMUNITY)
                .and_then(|c| c.channels.iter().find(|x| x.name == channel && x.kind == ChannelKind::Text).map(|x| x.id));
            let ch = match existing {
                Some(id) => id,
                None => {
                    let id = self.data.new_id();
                    if let Some(c) = self.data.community_mut(HOME_NET_COMMUNITY) {
                        c.channels.push(Channel {
                            id,
                            name: channel.clone(),
                            kind: ChannelKind::Text,
                            topic: "Shared over the home network.".into(),
                            messages: vec![],
                            unread: 0,
                            mentions: 0,
                            voice_people: vec![],
                        });
                    }
                    id
                }
            };
            let id = self.data.new_id();
            let text: String = text.chars().take(MAX_MESSAGE_CHARS).collect();
            self.deliver(
                Target::Channel(HOME_NET_COMMUNITY, ch),
                Message {
                    id,
                    author: uid,
                    author_name: Some(name),
                    text,
                    ts: now_secs(),
                    edited: false,
                    reactions: vec![],
                    reply_to: None,
                    pinned: false,
                },
            );
        }
    }

    fn toggle_reaction(&mut self, target: Target, msg: Id, emoji: &str, who: Id) {
        if let Some(list) = self.messages_mut(target) {
            if let Some(m) = list.iter_mut().find(|m| m.id == msg) {
                if let Some(r) = m.reactions.iter_mut().find(|r| r.emoji == emoji) {
                    if let Some(i) = r.by.iter().position(|x| *x == who) {
                        r.by.remove(i);
                    } else {
                        r.by.push(who);
                    }
                } else {
                    m.reactions.push(Reaction { emoji: emoji.into(), by: vec![who] });
                }
                m.reactions.retain(|r| !r.by.is_empty());
            }
        }
        self.dirty = true;
    }

    fn save_now(&mut self) {
        if let Some(p) = &self.save_path {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(bytes) = serde_json::to_vec(&self.data) {
                let tmp = p.with_extension("tmp");
                if std::fs::write(&tmp, bytes).is_ok() {
                    let _ = std::fs::rename(&tmp, p);
                }
            }
        }
        self.dirty = false;
    }
}

fn peer_user_id(instance: u64) -> Id {
    // Keep home-network peers far away from sample ids.
    (instance % 1_000_000_000) + 1_000_000
}

// ---------------------------------------------------------------- drawing helpers

fn initials(name: &str) -> String {
    let mut s: String = name.split_whitespace().take(2).filter_map(|w| w.chars().next()).collect();
    if s.is_empty() {
        s.push('?');
    }
    s.to_uppercase()
}

fn paint_avatar(ui: &Ui, center: Pos2, size: f32, name: &str, hue: u8, status: Option<Status>, ring_bg: Color32) {
    let p = ui.painter();
    let r = size / 2.0;
    p.circle_filled(center, r, theme::hue(hue));
    p.circle_filled(center + vec2(-r * 0.25, -r * 0.3), r * 0.55, Color32::from_white_alpha(14));
    p.text(
        center,
        Align2::CENTER_CENTER,
        name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default(),
        FontId::proportional(size * 0.45),
        Color32::WHITE,
    );
    if let Some(s) = status {
        let dot = (size * 0.17).max(4.0);
        let c = center + vec2(r * 0.72, r * 0.72);
        p.circle_filled(c, dot + 2.5, ring_bg);
        p.circle_filled(c, dot, theme::status_color(s));
        match s {
            Status::Hidden => {
                p.circle_filled(c, dot * 0.5, ring_bg);
            }
            Status::Busy => {
                p.line_segment([c - vec2(dot * 0.55, 0.0), c + vec2(dot * 0.55, 0.0)], Stroke::new(1.6, ring_bg));
            }
            Status::Away => {
                p.circle_filled(c + vec2(-dot * 0.45, -dot * 0.45), dot * 0.62, ring_bg);
            }
            Status::Online => {}
        }
    }
}

/// Hearth's own mark: a rounded tile with a stylised flame.
fn paint_logo(ui: &Ui, rect: Rect, accent: Color32) {
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same((rect.width() * 0.3) as u8), accent);
    let c = rect.center();
    let s = rect.width() / 46.0;
    let flame = vec![
        c + vec2(0.0, -13.0 * s),
        c + vec2(7.0 * s, -3.0 * s),
        c + vec2(9.0 * s, 4.0 * s),
        c + vec2(7.5 * s, 9.5 * s),
        c + vec2(3.5 * s, 12.5 * s),
        c + vec2(-3.5 * s, 12.5 * s),
        c + vec2(-7.5 * s, 9.5 * s),
        c + vec2(-9.0 * s, 4.0 * s),
        c + vec2(-6.0 * s, -4.0 * s),
    ];
    p.add(egui::Shape::convex_polygon(flame, Color32::from_rgb(0xFF, 0xF4, 0xE8), Stroke::NONE));
    p.circle_filled(c + vec2(0.0, 7.0 * s), 4.0 * s, accent);
}

fn paint_house(ui: &Ui, c: Pos2, color: Color32) {
    let p = ui.painter();
    let roof = vec![c + vec2(0.0, -10.0), c + vec2(11.0, -1.0), c + vec2(-11.0, -1.0)];
    p.add(egui::Shape::convex_polygon(roof, color, Stroke::NONE));
    p.rect_filled(Rect::from_min_max(c + vec2(-7.5, -2.0), c + vec2(7.5, 10.0)), CornerRadius::same(2), color);
    p.rect_filled(
        Rect::from_min_max(c + vec2(-2.5, 3.0), c + vec2(2.5, 10.0)),
        CornerRadius::same(1),
        Color32::from_black_alpha(90),
    );
}

fn paint_speaker(ui: &Ui, c: Pos2, color: Color32) {
    let p = ui.painter();
    p.rect_filled(Rect::from_center_size(c + vec2(-4.0, 0.0), vec2(4.0, 6.0)), CornerRadius::same(1), color);
    p.add(egui::Shape::convex_polygon(
        vec![c + vec2(-3.0, -3.0), c + vec2(2.0, -7.0), c + vec2(2.0, 7.0), c + vec2(-3.0, 3.0)],
        color,
        Stroke::NONE,
    ));
    p.add(egui::Shape::line(vec![c + vec2(4.5, -4.0), c + vec2(6.5, 0.0), c + vec2(4.5, 4.0)], Stroke::new(1.5, color)));
}

/// Smoothly animated 0..1 for hover/selection on the community rail.
fn rail_anim(ui: &Ui, id: EId, on: bool, reduced: bool) -> f32 {
    if reduced {
        return if on { 1.0 } else { 0.0 };
    }
    ui.ctx().animate_bool_with_time(id, on, 0.16)
}

/// Hover highlight that fades in and out instead of switching.
fn hover_anim(ui: &Ui, id: EId, hovered: bool, reduced: bool) -> f32 {
    if reduced {
        return if hovered { 1.0 } else { 0.0 };
    }
    ui.ctx().animate_bool_with_time(id, hovered, 0.12)
}

/// Round when idle, a rounded square when hovered or selected.
fn rail_radius(k: f32, square: bool) -> CornerRadius {
    let idle = if square { 12.0 } else { 23.0 };
    CornerRadius::same((idle + (14.0 - idle) * k).round() as u8)
}

fn paint_chevron(ui: &Ui, c: Pos2, open: bool, color: Color32) {
    let pts = if open {
        vec![c + vec2(-3.5, -1.5), c + vec2(0.0, 2.0), c + vec2(3.5, -1.5)]
    } else {
        vec![c + vec2(-1.5, -3.5), c + vec2(2.0, 0.0), c + vec2(-1.5, 3.5)]
    };
    ui.painter().add(egui::Shape::line(pts, Stroke::new(1.6, color)));
}

fn paint_hash(ui: &Ui, c: Pos2, color: Color32) {
    let p = ui.painter();
    let st = Stroke::new(1.7, color);
    p.line_segment([c + vec2(-2.0, -7.0), c + vec2(-4.0, 7.0)], st);
    p.line_segment([c + vec2(4.0, -7.0), c + vec2(2.0, 7.0)], st);
    p.line_segment([c + vec2(-6.5, -2.5), c + vec2(6.5, -2.5)], st);
    p.line_segment([c + vec2(-7.0, 2.5), c + vec2(6.0, 2.5)], st);
}

fn badge(ui: &Ui, center: Pos2, n: usize, bg: Color32, ring: Color32) {
    let txt = if n > 99 { "99+".to_string() } else { n.to_string() };
    let w = 16.0f32.max(8.0 + txt.len() as f32 * 7.0);
    let r = Rect::from_center_size(center, vec2(w, 16.0));
    ui.painter().rect_filled(r.expand(2.5), CornerRadius::same(10), ring);
    ui.painter().rect_filled(r, CornerRadius::same(8), bg);
    ui.painter().text(r.center(), Align2::CENTER_CENTER, txt, FontId::proportional(11.0), Color32::WHITE);
}

fn day_label(ts: i64) -> String {
    match Local.timestamp_opt(ts, 0).single() {
        Some(t) => t.format("%B %-d, %Y").to_string(),
        None => String::new(),
    }
}

fn time_label(ts: i64) -> String {
    let Some(t) = Local.timestamp_opt(ts, 0).single() else { return String::new() };
    let today = Local::now().date_naive();
    let d = t.date_naive();
    let clock = t.format(clock_fmt()).to_string();
    if d == today {
        format!("Today at {clock}")
    } else if today.num_days_from_ce() - d.num_days_from_ce() == 1 {
        format!("Yesterday at {clock}")
    } else {
        format!("{} {clock}", t.format("%d %b %Y"))
    }
}

static CLOCK_24H: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

fn clock_fmt() -> &'static str {
    if CLOCK_24H.load(std::sync::atomic::Ordering::Relaxed) { "%H:%M" } else { "%-I:%M %p" }
}

fn short_time(ts: i64) -> String {
    Local.timestamp_opt(ts, 0).single().map(|t| t.format(clock_fmt()).to_string()).unwrap_or_default()
}

/// Simple formatting: **strong**, *italic*, `code`, @mentions.
fn rich_text(text: &str, p: &Palette, names: &[String], size: f32, width: f32) -> egui::text::LayoutJob {
    use egui::text::{LayoutJob, TextFormat};
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    let base = TextFormat { font_id: FontId::proportional(size), color: p.text, ..Default::default() };
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let mut plain = String::new();
    let flush = |job: &mut LayoutJob, s: &mut String| {
        if !s.is_empty() {
            job.append(s, 0.0, base.clone());
            s.clear();
        }
    };
    let find = |from: usize, pat: &[char]| -> Option<usize> { (from..chars.len()).find(|&k| chars[k..].starts_with(pat)) };
    let everyone = "everyone".to_string();
    while i < chars.len() {
        if chars[i..].starts_with(&['*', '*']) {
            if let Some(end) = find(i + 2, &['*', '*']) {
                if end > i + 2 {
                    flush(&mut job, &mut plain);
                    let s: String = chars[i + 2..end].iter().collect();
                    job.append(&s, 0.0, TextFormat { color: p.strong, font_id: FontId::proportional(size + 0.5), ..base.clone() });
                    i = end + 2;
                    continue;
                }
            }
        }
        if chars[i] == '*' {
            if let Some(end) = find(i + 1, &['*']) {
                if end > i + 1 {
                    flush(&mut job, &mut plain);
                    let s: String = chars[i + 1..end].iter().collect();
                    job.append(&s, 0.0, TextFormat { italics: true, ..base.clone() });
                    i = end + 1;
                    continue;
                }
            }
        }
        if chars[i] == '`' {
            if let Some(end) = find(i + 1, &['`']) {
                if end > i + 1 {
                    flush(&mut job, &mut plain);
                    let s: String = chars[i + 1..end].iter().collect();
                    job.append(
                        &s,
                        0.0,
                        TextFormat { font_id: FontId::monospace(size - 1.0), background: p.input, color: p.strong, ..base.clone() },
                    );
                    i = end + 1;
                    continue;
                }
            }
        }
        if chars[i] == '@' && (i == 0 || chars[i - 1].is_whitespace()) {
            let rest: String = chars[i + 1..].iter().collect();
            let lower = rest.to_lowercase();
            let hit = names
                .iter()
                .chain(std::iter::once(&everyone))
                .filter(|n| !n.is_empty() && lower.starts_with(&n.to_lowercase()))
                .max_by_key(|n| n.len())
                .cloned();
            if let Some(n) = hit {
                flush(&mut job, &mut plain);
                let len = n.chars().count();
                let s: String = chars[i..i + 1 + len].iter().collect();
                job.append(&s, 0.0, TextFormat { color: p.accent, background: p.mention_bg, ..base.clone() });
                i += 1 + len;
                continue;
            }
        }
        plain.push(chars[i]);
        i += 1;
    }
    flush(&mut job, &mut plain);
    job
}

fn icon_button(ui: &mut Ui, glyph: &str, tip: &str, active: bool, p: &Palette) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::click());
    if resp.hovered() || active {
        ui.painter().rect_filled(rect, CornerRadius::same(6), if active { p.selected } else { p.hover });
    }
    let col = if active {
        p.accent
    } else if resp.hovered() {
        p.strong
    } else {
        p.muted
    };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(17.0), col);
    resp.on_hover_text(tip)
}

fn pill_button(ui: &mut Ui, text: &str, fill: Color32, fg: Color32) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(14.0), fg);
    let size = vec2(galley.size().x + 28.0, 34.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let f = if resp.hovered() { fill.gamma_multiply(0.85) } else { fill };
    ui.painter().rect_filled(rect, CornerRadius::same(8), f);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
    resp
}

fn section_label(ui: &mut Ui, text: &str, p: &Palette) {
    ui.label(RichText::new(text.to_uppercase()).size(11.5).color(p.muted).strong());
}

fn row_rect(ui: &mut Ui, h: f32) -> (Rect, egui::Response) {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    (rect.shrink2(vec2(8.0, 0.0)), resp)
}

// ---------------------------------------------------------------- the app

impl eframe::App for HearthApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        self.now = now;
        if self.egui_ctx.is_none() {
            self.egui_ctx = Some(ctx.clone());
        }
        // Something was still moving last frame: draw the next one soon.
        if std::mem::take(&mut self.animating) {
            ctx.request_repaint();
        }
        let mut p = theme::palette(self.data.settings.theme, self.data.settings.accent);
        if self.data.settings.high_contrast {
            p.text = p.strong;
            p.muted = p.strong.lerp_to_gamma(p.muted, 0.35);
            p.faint = p.strong.lerp_to_gamma(p.faint, 0.45);
            p.divider = p.muted;
        }
        let key = (
            self.data.settings.theme,
            self.data.settings.accent,
            (self.data.settings.text_scale * 100.0) as u32,
            self.data.settings.high_contrast,
        );
        if self.applied_theme != Some(key) {
            theme::apply(&ctx, &p, self.data.settings.text_scale);
            self.applied_theme = Some(key);
        }
        self.apply_window_settings(&ctx);

        self.window_focused = ctx.input(|i| i.focused);
        self.tick(now);
        self.update_audio(&ctx);
        self.track_view();
        self.schedule_updates();
        if std::mem::take(&mut self.attention) && self.screenshot.is_none() && !ctx.input(|i| i.focused) {
            // Flash the taskbar button when a message arrives while the window is in the background.
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
        }
        self.handle_screenshot(&ctx);
        if self.dirty && now - self.last_save > 20.0 {
            self.save_now();
            self.last_save = now;
        }
        if self.screenshot.is_some() {
            ctx.request_repaint();
        } else if self.share_screen.is_some()
            || self.camera.is_some()
            || !self.viewers.is_empty()
            || self.voice.is_some()
            || self.mic_test
        {
            // Moving pictures or voice meters: redraw about 30 times a second.
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        } else {
            // Keep animating (typing dots, voice rings) and keep listening to the network.
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        }

        if now < self.splash_until {
            self.splash(ui, &p, now);
            self.view_changed_at = now; // the app fades in when the start-up screen ends
            return;
        }
        self.update_banner(&ctx, &p);

        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            if self.picker.is_some() {
                self.picker = None;
            } else if self.profile.is_some() {
                self.profile = None;
            } else if self.dialog.is_some() {
                self.dialog = None;
            } else if self.settings_open {
                self.settings_open = false;
            } else if self.editing.is_some() {
                self.editing = None;
            } else {
                self.reply_to = None;
            }
        }

        if self.settings_open {
            self.settings(ui, &p);
            self.dialogs(&ctx, &p);
            self.go_live_dialog(&ctx, &p);
            return;
        }

        self.rail(ui, &p);
        self.sidebar(ui, &p, now);
        if self.show_stage && self.voice.is_some() {
            self.stage(ui, &p, now);
            self.popups(&ctx, &p);
            self.dialogs(&ctx, &p);
            self.go_live_dialog(&ctx, &p);
            return;
        }
        let target = self.current_target();
        let show_members = self.data.settings.show_members && matches!(target, Some(Target::Channel(..)));
        if !self.search_query.is_empty() && target.is_some() {
            self.search_results(ui, &p);
        } else if self.show_pins && target.is_some() {
            self.pins_panel(ui, &p);
        } else if show_members {
            self.members(ui, &p);
        } else if let Some(Target::Dm(u)) = target {
            self.dm_profile_panel(ui, &p, u);
        }
        match target {
            Some(t) => self.chat(ui, &p, t, now),
            None => self.friends(ui, &p),
        }
        self.popups(&ctx, &p);
        self.dialogs(&ctx, &p);
            self.go_live_dialog(&ctx, &p);
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        if self.dirty {
            self.save_now();
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_now();
        // Automatic install: a downloaded update takes this program's place for next time.
        if self.data.settings.auto_update_install && matches!(self.updater.state(), update::State::Ready { .. }) {
            if let Err(e) = self.updater.install() {
                eprintln!("update: {e}");
            }
        }
    }
}

impl HearthApp {
    fn handle_screenshot(&mut self, ctx: &egui::Context) {
        let Some(job) = &mut self.screenshot else { return };
        job.frames += 1;
        if job.frames >= 120 && !job.requested {
            job.requested = true;
            let slot = job.slot.clone();
            ctx.request_screenshot(move |img| {
                *slot.lock().unwrap() = Some(img);
            });
        }
        if job.frames == 400 {
            job.requested = false; // ask again if the first request got lost
        }
        let img = job.slot.lock().unwrap().take();
        if let Some(img) = img {
            let [w, h] = img.size;
            let mut bytes = Vec::with_capacity(w * h * 4);
            for px in &img.pixels {
                let [r, g, b, _] = px.to_srgba_unmultiplied();
                bytes.extend_from_slice(&[r, g, b, 255]);
            }
            if let Some(dir) = job.path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            match image::RgbaImage::from_raw(w as u32, h as u32, bytes) {
                Some(buf) => {
                    if let Err(e) = buf.save(&job.path) {
                        eprintln!("couldn't save screenshot: {e}");
                    }
                }
                None => eprintln!("screenshot had an unexpected size"),
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if job.frames > 900 {
            eprintln!("screenshot never arrived");
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn splash(&mut self, ui: &mut Ui, p: &Palette, now: f64) {
        egui::CentralPanel::default().frame(Frame::new().fill(p.rail)).show(ui, |ui| {
            let c = ui.max_rect().center();
            let pulse = if self.data.settings.reduced_motion { 1.0 } else { 1.0 + ((now * 3.0).sin() as f32) * 0.04 };
            paint_logo(ui, Rect::from_center_size(c - vec2(0.0, 40.0), vec2(84.0, 84.0) * pulse), p.accent);
            ui.painter().text(c + vec2(0.0, 26.0), Align2::CENTER_CENTER, "Hearth", FontId::proportional(28.0), p.strong);
            ui.painter().text(c + vec2(0.0, 60.0), Align2::CENTER_CENTER, SPLASH_TIPS[self.splash_tip], FontId::proportional(14.0), p.muted);
            let bar = Rect::from_center_size(c + vec2(0.0, 92.0), vec2(180.0, 4.0));
            ui.painter().rect_filled(bar, CornerRadius::same(2), p.input);
            let f = (now / self.splash_until).clamp(0.0, 1.0) as f32;
            ui.painter().rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * f, bar.height())), CornerRadius::same(2), p.accent);
        });
    }

    // ------------------------------------------------------------ community rail

    fn rail(&mut self, ui: &mut Ui, p: &Palette) {
        let mut open: Option<Option<Id>> = None;
        let mut new_dialog = false;
        egui::Panel::left("rail")
            .exact_size(RAIL_W)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::new().fill(p.rail).inner_margin(Margin::symmetric(0, 12)))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("rail_scroll")
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.spacing_mut().item_spacing.y = 8.0;
                            let home_unread: usize = self.data.dms.iter().map(|d| d.unread).sum::<usize>()
                                + self.data.users.iter().filter(|u| u.pending_in).count();
                            let (rect, resp) = ui.allocate_exact_size(vec2(ICON, ICON), Sense::click());
                            let on = self.community.is_none() || resp.hovered();
                            let k = rail_anim(ui, EId::new("rail_home"), on, self.data.settings.reduced_motion);
                            ui.painter().rect_filled(rect, rail_radius(k, self.data.settings.square_icons), p.side.lerp_to_gamma(p.accent, k));
                            paint_house(ui, rect.center(), if on { p.on_accent } else { p.text });
                            if home_unread > 0 && self.data.settings.show_badges {
                                badge(ui, rect.right_bottom() - vec2(4.0, 4.0), home_unread, p.danger, p.rail);
                            }
                            if resp.on_hover_text("Home: friends and direct messages").clicked() {
                                open = Some(None);
                            }
                            let (d, _) = ui.allocate_exact_size(vec2(30.0, 2.0), Sense::hover());
                            ui.painter().rect_filled(d, CornerRadius::same(1), p.hover);

                            for c in &self.data.communities {
                                let (rect, resp) = ui.allocate_exact_size(vec2(ICON, ICON), Sense::click());
                                let sel = self.community == Some(c.id);
                                let on = sel || resp.hovered();
                                let k = rail_anim(ui, EId::new(("rail", c.id)), on, self.data.settings.reduced_motion);
                                ui.painter().rect_filled(rect, rail_radius(k, self.data.settings.square_icons), p.side.lerp_to_gamma(theme::hue(c.hue), k));
                                ui.painter().text(
                                    rect.center(),
                                    Align2::CENTER_CENTER,
                                    initials(&c.name),
                                    FontId::proportional(16.0),
                                    if on { Color32::WHITE } else { p.text },
                                );
                                if sel {
                                    ui.painter().rect_stroke(rect.expand(3.0), CornerRadius::same(17), Stroke::new(2.0, p.accent), StrokeKind::Outside);
                                }
                                let m = c.mentions();
                                if m > 0 && self.data.settings.show_badges {
                                    badge(ui, rect.right_bottom() - vec2(4.0, 4.0), m, p.danger, p.rail);
                                } else if c.unread() && !sel && !c.muted {
                                    ui.painter().circle_filled(rect.right_top() + vec2(-3.0, 3.0), 5.5, p.rail);
                                    ui.painter().circle_filled(rect.right_top() + vec2(-3.0, 3.0), 3.5, p.strong);
                                }
                                if c.id == HOME_NET_COMMUNITY {
                                    let o = rect.left_bottom() + vec2(6.0, -6.0);
                                    ui.painter().circle_filled(o, 6.0, p.rail);
                                    ui.painter().circle_filled(
                                        o,
                                        4.0,
                                        if self.lan.is_some() { theme::status_color(Status::Online) } else { p.faint },
                                    );
                                }
                                if resp.on_hover_text(&c.name).clicked() {
                                    open = Some(Some(c.id));
                                }
                            }
                            let (rect, resp) = ui.allocate_exact_size(vec2(ICON, ICON), Sense::click());
                            let hov = resp.hovered();
                            let k = rail_anim(ui, EId::new("rail_add"), hov, self.data.settings.reduced_motion);
                            ui.painter().rect_filled(rect, rail_radius(k, self.data.settings.square_icons), p.side.lerp_to_gamma(p.accent, k));
                            ui.painter().text(rect.center(), Align2::CENTER_CENTER, "+", FontId::proportional(24.0), if hov { p.on_accent } else { p.accent });
                            if resp.on_hover_text("Create a community").clicked() {
                                new_dialog = true;
                            }
                        });
                    });
            });
        if let Some(o) = open {
            match o {
                None => self.open_home(),
                Some(c) => self.open_community(c),
            }
        }
        if new_dialog {
            let hue = (self.rand() % 8) as u8;
            self.dialog = Some(Dialog::NewCommunity { name: String::new(), hue });
        }
    }

    // ------------------------------------------------------------ sidebar

    fn sidebar(&mut self, ui: &mut Ui, p: &Palette, now: f64) {
        egui::Panel::left("sidebar")
            .exact_size(SIDE_W)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::new().fill(p.side))
            .show(ui, |ui| {
                egui::Panel::bottom("userbar")
                    .exact_size(if self.voice.is_some() { 124.0 } else { 56.0 })
                    .show_separator_line(false)
                    .resizable(false)
                    .frame(Frame::new().fill(p.raised).inner_margin(Margin::symmetric(8, 8)))
                    .show(ui, |ui| self.user_bar(ui, p));
                match self.community {
                    None => self.home_sidebar(ui, p),
                    Some(c) => self.community_sidebar(ui, p, c, now),
                }
            });
    }

    fn user_bar(&mut self, ui: &mut Ui, p: &Palette) {
        if let Some((c, ch)) = self.voice {
            let cname = self.data.community(c).map(|c| c.name.clone()).unwrap_or_default();
            let chname = self.channel_name(c, ch);
            let mut leave = false;
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("• Voice connected").color(theme::status_color(Status::Online)).size(13.5).strong());
                    ui.add(egui::Label::new(RichText::new(format!("{chname} / {cname}")).color(p.muted).size(12.0)).truncate());
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if icon_button(ui, "✖", "Disconnect", false, p).clicked() {
                        leave = true;
                    }
                });
            });
            ui.add_space(4.0);
            let mut toggle: Option<Source> = None;
            ui.horizontal(|ui| {
                let w = (ui.available_width() - 8.0) / 2.0;
                let cam_on = self.camera.is_some();
                let scr_on = self.share_screen.is_some();
                let (cam_fill, cam_fg) = if cam_on { (p.accent, p.on_accent) } else { (p.input, p.text) };
                let (scr_fill, scr_fg) = if scr_on { (p.accent, p.on_accent) } else { (p.input, p.text) };
                let cam = ui.add_sized(
                    [w, 24.0],
                    egui::Button::new(RichText::new(if cam_on { "Video on" } else { "Video" }).size(12.0).color(cam_fg)).fill(cam_fill),
                );
                if cam.on_hover_text(if cam_on { "Turn your camera off" } else { "Turn your camera on" }).clicked() {
                    toggle = Some(Source::Camera);
                }
                let scr = ui.add_sized(
                    [w, 24.0],
                    egui::Button::new(RichText::new(if scr_on { "Sharing" } else { "Share screen" }).size(12.0).color(scr_fg))
                        .fill(scr_fill),
                );
                if scr.on_hover_text(if scr_on { "Stop sharing your screen" } else { "Share your main screen" }).clicked() {
                    toggle = Some(Source::Screen);
                }
            });
            ui.add_space(2.0);
            ui.separator();
            if let Some(s) = toggle {
                self.toggle_capture(s);
            }
            if leave {
                self.leave_voice();
            }
        }
        ui.horizontal_centered(|ui| {
            let me = self.data.me().clone();
            let (rect, resp) = ui.allocate_exact_size(vec2(AVATAR, AVATAR), Sense::click());
            paint_avatar(ui, rect.center(), 32.0, &me.name, me.hue, Some(me.status), p.raised);
            let pos = rect.left_top();
            ui.vertical(|ui| {
                ui.add_space(2.0);
                ui.label(RichText::new(&me.name).color(p.strong).size(13.5).strong());
                ui.label(RichText::new(me.status.label()).color(p.muted).size(11.5));
            });
            if resp.on_hover_text("Your profile and status").clicked() {
                self.profile = Some((ME, pos - vec2(0.0, 360.0)));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                if icon_button(ui, "⚙", "Settings", false, p).clicked() {
                    self.settings_open = true;
                }
                let d = icon_button(ui, "🎧", if self.deafened { "Undeafen" } else { "Deafen" }, self.deafened, p);
                if d.clicked() {
                    self.toggle_deafen();
                }
                let m = icon_button(ui, "🎤", if self.mic_muted { "Unmute" } else { "Mute" }, self.mic_muted, p);
                if m.clicked() {
                    self.toggle_mute();
                }
            });
        });
    }

    fn leave_voice(&mut self) {
        if let Some((c, ch)) = self.voice.take() {
            if let Some(x) = self.data.community_mut(c).and_then(|c| c.channels.iter_mut().find(|x| x.id == ch)) {
                x.voice_people.retain(|id| *id != ME);
            }
            self.sfx(Sfx::Leave);
        }
        self.prev_people.clear();
        // Leaving stops everything we share or watch.
        self.share_screen = None;
        self.camera = None;
        self.viewers.clear();
        self.show_stage = false;
        self.focused_tile = None;
        self.last_hello = -100.0;
    }

    fn join_voice(&mut self, c: Id, ch: Id) {
        self.leave_voice();
        if let Some(x) = self.data.community_mut(c).and_then(|c| c.channels.iter_mut().find(|x| x.id == ch)) {
            if !x.voice_people.contains(&ME) {
                x.voice_people.push(ME);
            }
        }
        self.voice = Some((c, ch));
        self.show_stage = true;
        self.last_hello = -100.0;
        // People already there don't count as arriving.
        self.prev_people = self
            .data
            .community(c)
            .and_then(|x| x.channels.iter().find(|x| x.id == ch))
            .map(|x| x.voice_people.clone())
            .unwrap_or_default();
        self.sfx(Sfx::Join);
    }

    /// Newest picture of a stream as a texture (uploaded only when a new picture arrived).
    fn stream_texture(&mut self, ctx: &egui::Context, key: &str, stream: (Id, Source)) -> Option<(egui::TextureId, egui::Vec2)> {
        let (frame, pic) = if stream.0 == ME {
            let cap = match stream.1 {
                Source::Screen => self.share_screen.as_ref()?,
                Source::Camera => self.camera.as_ref()?,
            };
            (cap.frame_number(), cap.picture())
        } else {
            let v = self.viewers.get(&stream)?;
            (v.frame_number(), v.picture())
        };
        let pic = pic?;
        let size = vec2(pic.width as f32, pic.height as f32);
        let stale = self.textures.get(key).map(|(_, f)| *f != frame).unwrap_or(true);
        if stale {
            let img = egui::ColorImage::from_rgba_unmultiplied([pic.width, pic.height], &pic.rgba);
            match self.textures.get_mut(key) {
                Some((tex, f)) => {
                    tex.set(img, egui::TextureOptions::LINEAR);
                    *f = frame;
                }
                None => {
                    let tex = ctx.load_texture(key, img, egui::TextureOptions::LINEAR);
                    self.textures.insert(key.to_string(), (tex, frame));
                }
            }
        }
        self.textures.get(key).map(|(t, _)| (t.id(), size))
    }

    fn stream_error(&self, stream: (Id, Source)) -> Option<String> {
        if stream.0 == ME {
            match stream.1 {
                Source::Screen => self.share_screen.as_ref()?.error(),
                Source::Camera => self.camera.as_ref()?.error(),
            }
        } else {
            self.viewers.get(&stream)?.error()
        }
    }

    fn stage_tiles(&self, c: Id, ch: Id) -> Vec<Tile> {
        let people = self
            .data
            .community(c)
            .and_then(|c| c.channels.iter().find(|x| x.id == ch))
            .map(|x| x.voice_people.clone())
            .unwrap_or_default();
        let name_of = |id: Id| self.data.user(id).map(|u| u.name.clone()).unwrap_or_else(|| "Someone".into());
        let mut screens = vec![];
        let mut tiles = vec![];
        for pid in people {
            let name = name_of(pid);
            let (has_cam, has_screen) = if pid == ME {
                (self.camera.is_some(), self.share_screen.is_some())
            } else if let Some(info) = self.peers.get(&pid) {
                (
                    info.streams.iter().any(|(s, _)| *s == Source::Camera),
                    info.streams.iter().any(|(s, _)| *s == Source::Screen),
                )
            } else {
                (false, false)
            };
            tiles.push(Tile {
                key: format!("{}-camera", if pid == ME { "me".to_string() } else { pid.to_string() }),
                user: pid,
                label: if pid == ME && name != "You" { format!("{name} (you)") } else { name.clone() },
                stream: if has_cam { Some((pid, Source::Camera)) } else { None },
            });
            if has_screen {
                screens.push(Tile {
                    key: format!("{}-screen", if pid == ME { "me".to_string() } else { pid.to_string() }),
                    user: pid,
                    label: if pid == ME { "Your screen".to_string() } else { format!("{name}'s screen") },
                    stream: Some((pid, Source::Screen)),
                });
            }
        }
        // Shared screens first: they're usually what people want to look at.
        screens.extend(tiles);
        screens
    }

    fn stage(&mut self, ui: &mut Ui, p: &Palette, now: f64) {
        let Some((c, ch)) = self.voice else { return };
        let ctx = ui.ctx().clone();
        let chname = self.channel_name(c, ch);
        let cname = self.data.community(c).map(|c| c.name.clone()).unwrap_or_default();
        let tiles = self.stage_tiles(c, ch);
        if let Some(f) = &self.focused_tile {
            if !tiles.iter().any(|t| &t.key == f) {
                self.focused_tile = None;
            }
        }
        // Forget pictures of streams that ended.
        self.textures.retain(|k, _| tiles.iter().any(|t| &t.key == k && t.stream.is_some()));

        let mut back = false;
        egui::Panel::top("stage_header")
            .exact_size(HEADER_H)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::new().fill(p.rail).inner_margin(Margin::symmetric(14, 0)))
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                    paint_speaker(ui, r.center(), p.muted);
                    ui.label(RichText::new(&chname).size(16.0).color(p.strong).strong());
                    ui.label(RichText::new(format!("in {cname}")).size(13.0).color(p.muted));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(egui::Button::new(RichText::new("Back to chat").color(p.text)).fill(p.input)).clicked() {
                            back = true;
                        }
                    });
                });
            });

        let mut action: Option<u8> = None;
        egui::Panel::bottom("stage_controls")
            .exact_size(76.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::new().fill(p.rail))
            .show(ui, |ui| {
                let full = ui.max_rect();
                let mut buttons: Vec<(&str, &str, bool, Color32, u8)> = vec![
                    ("🎤", if self.mic_muted { "Unmute" } else { "Mute" }, self.mic_muted, p.danger, 0),
                    ("🎧", if self.deafened { "Undeafen" } else { "Deafen (hear nobody)" }, self.deafened, p.danger, 5),
                    ("📷", if self.camera.is_some() { "Turn camera off" } else { "Turn camera on" }, self.camera.is_some(), p.accent, 1),
                    ("🖥", if self.share_screen.is_some() { "Stop sharing" } else { "Share your screen or an app" }, self.share_screen.is_some(), p.accent, 2),
                ];
                if self.share_screen.is_some() || self.camera.is_some() {
                    buttons.push(("⚙", "Stream options: what you share, quality and frame rate", false, p.accent, 4));
                }
                buttons.push(("✖", "Leave voice channel", true, p.danger, 3));
                let gap = 16.0;
                let total = buttons.len() as f32 * 52.0 + (buttons.len() as f32 - 1.0) * gap;
                let mut x = full.center().x - total / 2.0 + 26.0;
                for (glyph, tip, on, on_col, k) in buttons {
                    let c = pos2(x, full.center().y);
                    let r = Rect::from_center_size(c, vec2(52.0, 52.0));
                    let resp = ui.interact(r, EId::new(("stage_btn", k)), Sense::click());
                    let fill = if on { on_col } else if resp.hovered() { p.selected } else { p.input };
                    ui.painter().circle_filled(c, 26.0, fill);
                    let fg = if on { Color32::WHITE } else { p.strong };
                    ui.painter().text(c, Align2::CENTER_CENTER, glyph, FontId::proportional(20.0), fg);
                    if (k == 0 && self.mic_muted) || (k == 5 && self.deafened) {
                        ui.painter().line_segment([c + vec2(-11.0, -11.0), c + vec2(11.0, 11.0)], Stroke::new(2.5, Color32::WHITE));
                    }
                    if resp.on_hover_text(tip).clicked() {
                        action = Some(k);
                    }
                    x += 52.0 + gap;
                }
            });

        let mut clicked_tile: Option<String> = None;
        let view_t = self.ease(self.view_changed_at, 0.25);
        egui::CentralPanel::default().frame(Frame::new().fill(p.rail).inner_margin(16.0)).show(ui, |ui| {
            ui.multiply_opacity(view_t);
            let mut area = ui.max_rect();
            area.min.y += (1.0 - view_t) * 14.0;
            let mut notes: Vec<(String, Color32)> = vec![];
            if let Some(e) = self.audio.error() {
                notes.push((format!("Sound: {e} (Settings → Voice & sound)"), p.danger));
            }
            if c != HOME_NET_COMMUNITY {
                notes.push((
                    "Nobody can hear or see you in this community. Talk with friends in Home Network → Lounge.".into(),
                    p.mention_bg,
                ));
            } else if self.data.settings.push_to_talk {
                notes.push((format!("Push to talk: hold {} while Hearth is the active window.", self.data.settings.ptt_key), p.mention_bg));
            }
            if self.share_screen.is_some() && c == HOME_NET_COMMUNITY {
                notes.push(("You're live: everyone in this voice channel can see what you share.".into(), p.mention_bg));
            }
            for (note, fill) in notes {
                let bar = Rect::from_min_size(area.min, vec2(area.width(), 30.0));
                ui.painter().rect_filled(bar, CornerRadius::same(8), fill);
                ui.painter().text(bar.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, note, FontId::proportional(13.0), p.strong);
                area.min.y += 38.0;
            }
            if tiles.is_empty() {
                return;
            }
            let gap = 12.0;
            let focus = self.focused_tile.clone().and_then(|f| tiles.iter().position(|t| t.key == f));
            let mut placed: Vec<(usize, Rect)> = vec![];
            if let Some(fi) = focus.filter(|_| tiles.len() > 1) {
                let strip_h = 120.0f32.min(area.height() * 0.25);
                let big = Rect::from_min_max(area.min, pos2(area.max.x, area.max.y - strip_h - gap));
                placed.push((fi, big));
                let others: Vec<usize> = (0..tiles.len()).filter(|i| *i != fi).collect();
                let w = (strip_h * 16.0 / 9.0).min((area.width() - gap * (others.len() as f32 - 1.0)) / others.len() as f32);
                let total = others.len() as f32 * w + (others.len() as f32 - 1.0) * gap;
                let mut x = area.center().x - total / 2.0;
                for i in others {
                    placed.push((i, Rect::from_min_size(pos2(x, area.max.y - strip_h), vec2(w, strip_h))));
                    x += w + gap;
                }
            } else {
                // Grid of equal 16:9 boxes, as large as fits.
                let n = tiles.len();
                let (mut best, mut best_cols) = (0.0f32, 1usize);
                for cols in 1..=n {
                    let rows = n.div_ceil(cols);
                    let mut w = (area.width() - gap * (cols as f32 - 1.0)) / cols as f32;
                    let mut h = w * 9.0 / 16.0;
                    if rows as f32 * h + gap * (rows as f32 - 1.0) > area.height() {
                        h = (area.height() - gap * (rows as f32 - 1.0)) / rows as f32;
                        w = h * 16.0 / 9.0;
                    }
                    if w * h > best {
                        best = w * h;
                        best_cols = cols;
                    }
                }
                let cols = best_cols;
                let rows = n.div_ceil(cols);
                let mut w = (area.width() - gap * (cols as f32 - 1.0)) / cols as f32;
                let mut h = w * 9.0 / 16.0;
                if rows as f32 * h + gap * (rows as f32 - 1.0) > area.height() {
                    h = (area.height() - gap * (rows as f32 - 1.0)) / rows as f32;
                    w = h * 16.0 / 9.0;
                }
                let grid_h = rows as f32 * h + gap * (rows as f32 - 1.0);
                let top = area.center().y - grid_h / 2.0;
                for i in 0..n {
                    let (r, col) = (i / cols, i % cols);
                    let in_row = if r == rows - 1 { n - r * cols } else { cols };
                    let row_w = in_row as f32 * w + (in_row as f32 - 1.0) * gap;
                    let left = area.center().x - row_w / 2.0;
                    placed.push((i, Rect::from_min_size(pos2(left + col as f32 * (w + gap), top + r as f32 * (h + gap)), vec2(w, h))));
                }
            }
            for (i, rect) in placed {
                let t = &tiles[i];
                if self.draw_tile(ui, &ctx, p, t, rect, now) {
                    clicked_tile = Some(t.key.clone());
                }
            }
        });

        if let Some(k) = clicked_tile {
            self.focused_tile = if self.focused_tile.as_ref() == Some(&k) { None } else { Some(k) };
        }
        match action {
            Some(0) => self.toggle_mute(),
            Some(5) => self.toggle_deafen(),
            Some(1) => self.toggle_capture(Source::Camera),
            Some(2) => self.toggle_capture(Source::Screen),
            Some(3) => self.leave_voice(),
            Some(4) => {
                let src = if self.share_screen.is_some() { Source::Screen } else { Source::Camera };
                self.open_go_live(src, true);
            }
            _ => {}
        }
        if back {
            self.show_stage = false;
        }
    }

    /// Draws one stage box; returns true when it was clicked.
    fn draw_tile(&mut self, ui: &mut Ui, ctx: &egui::Context, p: &Palette, t: &Tile, rect: Rect, now: f64) -> bool {
        let resp = ui.interact(rect, EId::new(("tile", &t.key)), Sense::click());
        let user = self.data.user(t.user).cloned();
        let hue = user.as_ref().map(|u| u.hue).unwrap_or(0);
        let bg = theme::hue(hue).lerp_to_gamma(p.raised, 0.78);
        ui.painter().rect_filled(rect, CornerRadius::same(12), bg);
        let mut shown = false;
        if let Some(stream) = t.stream {
            if let Some((tex, size)) = self.stream_texture(ctx, &t.key, stream) {
                let inner = rect.shrink(2.0);
                let scale = (inner.width() / size.x).min(inner.height() / size.y);
                let img = Rect::from_center_size(inner.center(), size * scale);
                ui.painter().rect_filled(inner, CornerRadius::same(10), Color32::BLACK);
                ui.painter().image(tex, img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                shown = true;
            } else {
                let msg = self.stream_error(stream).unwrap_or_else(|| "Starting…".into());
                ui.painter().text(
                    rect.center() + vec2(0.0, rect.height() * 0.28),
                    Align2::CENTER_CENTER,
                    msg,
                    FontId::proportional(12.5),
                    p.muted,
                );
            }
        }
        if !shown {
            let size = (rect.height() * 0.38).clamp(28.0, 110.0);
            let speaking = self.speaking(t.user, now, 0);
            if speaking {
                ui.painter().circle_stroke(rect.center(), size / 2.0 + 4.0, Stroke::new(3.0, theme::status_color(Status::Online)));
            }
            let name = user.as_ref().map(|u| u.name.clone()).unwrap_or_default();
            paint_avatar(ui, rect.center(), size, &name, hue, None, bg);
        }
        if let Some((ME, src)) = t.stream {
            let cap = match src {
                Source::Screen => self.share_screen.as_ref(),
                Source::Camera => self.camera.as_ref(),
            };
            if let Some(cap) = cap {
                let s = cap.settings();
                let q = media::QUALITIES[(s.quality as usize).min(media::QUALITIES.len() - 1)].0;
                let q = q.split(" · ").nth(1).unwrap_or(q);
                let real = cap.fps();
                let rate = if real > 0.5 { format!("{:.0} of {} fps", real.min(s.fps as f32), s.fps) } else { format!("{} fps", s.fps) };
                let text = if cap.label.is_empty() { format!("{q} · {rate}") } else { format!("{} · {q} · {rate}", cap.label.chars().take(40).collect::<String>()) };
                let g = ui.painter().layout_no_wrap(text, FontId::proportional(11.0), Color32::WHITE);
                let chip = Rect::from_min_size(rect.min + vec2(56.0, 10.0), vec2(g.size().x + 12.0, 18.0));
                ui.painter().rect_filled(chip, CornerRadius::same(4), Color32::from_black_alpha(150));
                ui.painter().galley(chip.left_center() + vec2(6.0, -g.size().y / 2.0), g, Color32::WHITE);
            }
        }
        // Friends' streams: how smooth they arrive.
        if let Some((uid, src)) = t.stream {
            if uid != ME {
                if let Some(v) = self.viewers.get(&(uid, src)) {
                    let fps = v.fps();
                    if fps > 0.5 {
                        let g = ui.painter().layout_no_wrap(format!("{fps:.0} fps"), FontId::proportional(11.0), Color32::WHITE);
                        let chip = Rect::from_min_size(rect.min + vec2(56.0, 10.0), vec2(g.size().x + 12.0, 18.0));
                        ui.painter().rect_filled(chip, CornerRadius::same(4), Color32::from_black_alpha(150));
                        ui.painter().galley(chip.left_center() + vec2(6.0, -g.size().y / 2.0), g, Color32::WHITE);
                    }
                }
            }
        }
        if t.stream.is_some() {
            let live = Rect::from_min_size(rect.min + vec2(10.0, 10.0), vec2(40.0, 18.0));
            ui.painter().rect_filled(live, CornerRadius::same(4), p.danger);
            ui.painter().text(live.center(), Align2::CENTER_CENTER, "LIVE", FontId::proportional(10.5), Color32::WHITE);
        }
        // Name chip
        let g = ui.painter().layout_no_wrap(t.label.clone(), FontId::proportional(13.0), Color32::WHITE);
        let chip = Rect::from_min_size(rect.left_bottom() + vec2(10.0, -32.0), vec2(g.size().x + 16.0, 22.0));
        ui.painter().rect_filled(chip, CornerRadius::same(6), Color32::from_black_alpha(150));
        ui.painter().galley(chip.left_center() + vec2(8.0, -g.size().y / 2.0), g, Color32::WHITE);
        if t.user == ME && self.mic_muted {
            let m = Rect::from_min_size(chip.right_top() + vec2(6.0, 0.0), vec2(22.0, 22.0));
            ui.painter().rect_filled(m, CornerRadius::same(6), p.danger);
            ui.painter().text(m.center(), Align2::CENTER_CENTER, "🎤", FontId::proportional(12.0), Color32::WHITE);
        }
        if resp.hovered() {
            ui.painter().rect_stroke(rect, CornerRadius::same(12), Stroke::new(2.0, p.accent), StrokeKind::Inside);
        }
        let is_friend = t.user >= 1_000_000;
        let tip = match (self.focused_tile.as_ref() == Some(&t.key), is_friend) {
            (true, true) => "Click to show everyone · right-click for their volume",
            (true, false) => "Click to show everyone",
            (false, true) => "Click to make this big · right-click for their volume",
            (false, false) => "Click to make this big",
        };
        if is_friend {
            let name = user.as_ref().map(|u| u.name.clone()).unwrap_or_default();
            let mut vol = self.data.settings.user_volumes.get(&name).copied().unwrap_or(1.0) * 100.0;
            let mut changed = false;
            resp.context_menu(|ui| {
                ui.label(RichText::new(format!("{name}'s volume")).strong());
                if ui.add(egui::Slider::new(&mut vol, 0.0..=200.0).suffix("%").step_by(5.0)).changed() {
                    changed = true;
                }
                if ui.button("Reset to 100%").clicked() {
                    vol = 100.0;
                    changed = true;
                }
            });
            if changed {
                self.data.settings.user_volumes.insert(name, vol / 100.0);
                self.dirty = true;
            }
        }
        resp.on_hover_text(tip).clicked()
    }

    /// Stop a running share, or open the Go live picker to start one.
    fn toggle_capture(&mut self, source: Source) {
        let live = match source {
            Source::Screen => self.share_screen.is_some(),
            Source::Camera => self.camera.is_some(),
        };
        if live {
            match source {
                Source::Screen => self.share_screen = None,
                Source::Camera => self.camera = None,
            }
            self.textures.remove(&format!("me-{}", source.tag()));
            self.last_hello = -100.0; // tell the network right away
            self.sfx(Sfx::LiveStop);
        } else {
            self.open_go_live(source, false);
        }
    }

    fn open_go_live(&mut self, source: Source, changing: bool) {
        let s = &self.data.settings;
        let (mut quality, mut fps) = match source {
            Source::Screen => (s.screen_quality, s.screen_fps),
            Source::Camera => (s.camera_quality, s.camera_fps),
        };
        let current = match source {
            Source::Screen => self.share_screen.as_ref(),
            Source::Camera => self.camera.as_ref(),
        };
        let mut selected = None;
        let mut tab = 0;
        if let Some(cap) = current {
            let st = cap.settings();
            quality = st.quality;
            fps = st.fps;
            selected = Some(cap.what.clone());
            tab = if matches!(cap.what, What::Window(_)) { 1 } else { 0 };
        }
        let (screens, windows, cameras) = match source {
            Source::Screen => (media::list_screens(), media::list_windows(), vec![]),
            Source::Camera => (vec![], vec![], media::list_cameras()),
        };
        if selected.is_none() {
            selected = match source {
                Source::Screen => screens
                    .iter()
                    .find(|c| c.label.contains("(main)"))
                    .or(screens.first())
                    .map(|c| c.what.clone()),
                Source::Camera => cameras.first().map(|c| c.what.clone()),
            };
        }
        self.go_live = Some(GoLive { source, tab, screens, windows, cameras, selected, quality, fps, changing });
    }

    fn go_live_dialog(&mut self, ctx: &egui::Context, p: &Palette) {
        let Some(mut g) = self.go_live.take() else { return };
        let mut go = false;
        let mut cancel = false;
        let mut refresh = false;
        let pop_t = self.ease(self.popup_opened_at, 0.2);
        egui::Modal::new(EId::new("go_live"))
            .backdrop_color(Color32::from_black_alpha((150.0 * pop_t) as u8))
            .frame(Frame::new().fill(p.main).corner_radius(12.0).inner_margin(20.0))
            .show(ctx, |ui| {
                ui.multiply_opacity(pop_t);
                ui.set_width(520.0);
                let title = match (g.source, g.changing) {
                    (Source::Screen, false) => "Share your screen",
                    (Source::Camera, false) => "Turn on video",
                    (Source::Screen, true) => "Screen sharing options",
                    (Source::Camera, true) => "Video options",
                };
                ui.label(RichText::new(title).size(20.0).color(p.strong).strong());
                ui.add_space(8.0);

                // What to share
                if g.source == Source::Screen {
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut g.tab, 0, RichText::new("Whole screen").size(14.0));
                        ui.selectable_value(&mut g.tab, 1, RichText::new("App window").size(14.0));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.small_button("Refresh list").clicked() {
                                refresh = true;
                            }
                        });
                    });
                } else {
                    section_label(ui, "Camera", p);
                }
                ui.add_space(4.0);
                let list: &Vec<Choice> = match (g.source, g.tab) {
                    (Source::Camera, _) => &g.cameras,
                    (_, 0) => &g.screens,
                    _ => &g.windows,
                };
                let mut pick: Option<What> = None;
                Frame::new().fill(p.side).corner_radius(10.0).inner_margin(6.0).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    egui::ScrollArea::vertical().id_salt("go_live_list").max_height(220.0).show(ui, |ui| {
                        if list.is_empty() {
                            let empty = match (g.source, g.tab) {
                                (Source::Camera, _) => "No camera found. Plug one in, then reopen this.",
                                (_, 0) => "No screen found.",
                                _ => "No app windows to share. Open the app you want to show, then press Refresh list.",
                            };
                            ui.add_space(12.0);
                            ui.label(RichText::new(empty).color(p.muted));
                            ui.add_space(12.0);
                        }
                        for c in list {
                            let sel = g.selected.as_ref() == Some(&c.what);
                            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
                            if sel || resp.hovered() {
                                ui.painter().rect_filled(rect, CornerRadius::same(8), if sel { p.selected } else { p.hover });
                            }
                            if sel {
                                ui.painter().rect_stroke(rect, CornerRadius::same(8), Stroke::new(1.5, p.accent), StrokeKind::Inside);
                            }
                            let glyph = match c.what {
                                What::Monitor(_) => "🖥",
                                What::Window(_) => "🗔",
                                What::Camera(_) => "📷",
                            };
                            ui.painter().text(rect.left_center() + vec2(18.0, 0.0), Align2::CENTER_CENTER, glyph, FontId::proportional(18.0), if sel { p.accent } else { p.muted });
                            ui.painter().text(rect.left_center() + vec2(38.0, -8.0), Align2::LEFT_CENTER, &c.label, FontId::proportional(14.0), p.strong);
                            ui.painter().text(rect.left_center() + vec2(38.0, 9.0), Align2::LEFT_CENTER, &c.detail, FontId::proportional(11.5), p.muted);
                            if resp.clicked() {
                                pick = Some(c.what.clone());
                            }
                        }
                    });
                });
                if let Some(w) = pick {
                    g.selected = Some(w);
                }

                // Quality and frame rate
                ui.add_space(12.0);
                section_label(ui, "Quality", p);
                ui.horizontal_wrapped(|ui| {
                    for (i, (label, _, _)) in media::QUALITIES.iter().enumerate() {
                        ui.selectable_value(&mut g.quality, i as u8, *label);
                    }
                });
                ui.add_space(8.0);
                section_label(ui, "Frame rate", p);
                ui.horizontal(|ui| {
                    for fps in media::FRAME_RATES {
                        let hint = match fps {
                            5 => "Smooth enough for documents and slides; lightest on the network",
                            15 => "Good for most things",
                            30 => "Smooth video and games",
                            _ => "Smoothest; needs a fast computer and network",
                        };
                        ui.selectable_value(&mut g.fps, fps, format!("{fps} fps")).on_hover_text(hint);
                    }
                });
                ui.add_space(6.0);
                let (_, h, q) = media::QUALITIES[(g.quality as usize).min(media::QUALITIES.len() - 1)];
                // Rough guide: JPEG at this size and quality, times pictures per second (estimated).
                let px = if h == 0 { 1440.0 * 2560.0 } else { (h as f64) * (h as f64) * 16.0 / 9.0 };
                let bytes_per_px = 0.04 + 0.0025 * q as f64;
                let mbit = px * bytes_per_px * g.fps as f64 * 8.0 / 1_000_000.0;
                ui.label(
                    RichText::new(format!("Uses about {:.0} Mbit/s of your home network for each person watching (estimate).", mbit.max(1.0)))
                        .size(12.0)
                        .color(p.faint),
                );
                if g.quality == 3 || g.fps >= 60 {
                    ui.label(RichText::new("Best quality or 60 fps can stutter on slower computers or Wi-Fi.").size(12.0).color(p.faint));
                }

                ui.add_space(14.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let label = if g.changing { "Apply" } else { "Go live" };
                    let ready = g.selected.is_some();
                    let (fill, fg) = if ready { (p.accent, p.on_accent) } else { (p.input, p.faint) };
                    if pill_button(ui, label, fill, fg).clicked() && ready {
                        go = true;
                    }
                    if ui.add(egui::Button::new(RichText::new("Cancel").color(p.muted)).frame(false)).clicked() {
                        cancel = true;
                    }
                });
            });
        if refresh {
            g.screens = media::list_screens();
            g.windows = media::list_windows();
        }
        if cancel {
            return;
        }
        if !go {
            self.go_live = Some(g);
            return;
        }
        let Some(what) = g.selected.clone() else { return };
        let settings = StreamSettings { quality: g.quality, fps: g.fps };
        match g.source {
            Source::Screen => {
                self.data.settings.screen_quality = g.quality;
                self.data.settings.screen_fps = g.fps;
            }
            Source::Camera => {
                self.data.settings.camera_quality = g.quality;
                self.data.settings.camera_fps = g.fps;
            }
        }
        self.dirty = true;
        let label = [&g.screens, &g.windows, &g.cameras]
            .into_iter()
            .flatten()
            .find(|c| c.what == what)
            .map(|c| c.label.clone())
            .unwrap_or_default();
        let slot = match g.source {
            Source::Screen => &mut self.share_screen,
            Source::Camera => &mut self.camera,
        };
        match slot {
            // Same thing, new quality or frame rate: change it without stopping.
            Some(cap) if cap.what == what => cap.set_settings(settings),
            _ => {
                *slot = Some(Capture::start(what, label, settings, self.egui_ctx.clone()));
                self.textures.remove(&format!("me-{}", g.source.tag()));
                self.last_hello = -100.0;
                self.sfx(Sfx::LiveStart);
            }
        }
        if self.voice.is_some() {
            self.show_stage = true;
        }
    }

    fn home_sidebar(&mut self, ui: &mut Ui, p: &Palette) {
        ui.add_space(10.0);
        let mut open: Option<Id> = None;
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.find_dm)
                    .hint_text("Find a conversation")
                    .desired_width(SIDE_W - 20.0)
                    .background_color(p.raised),
            );
            if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                let q = self.find_dm.trim().to_lowercase();
                if !q.is_empty() {
                    open = self.data.users.iter().find(|u| u.id != ME && u.name.to_lowercase().starts_with(&q)).map(|u| u.id);
                }
            }
        });
        if open.is_some() {
            self.find_dm.clear();
        }
        ui.add_space(8.0);
        let mut open_friends = false;
        let mut close: Option<Id> = None;
        let mut new_dm = false;
        let pending = self.data.users.iter().filter(|u| u.pending_in).count();
        egui::ScrollArea::vertical().id_salt("home_side").show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let (rect, resp) = row_rect(ui, 40.0);
            let sel = self.dm.is_none();
            if sel || resp.hovered() {
                ui.painter().rect_filled(rect, CornerRadius::same(6), if sel { p.selected } else { p.hover });
            }
            let c = rect.left_center() + vec2(22.0, 0.0);
            let col = if sel { p.strong } else { p.muted };
            ui.painter().circle_filled(c + vec2(-4.0, -4.0), 4.5, col);
            ui.painter().circle_filled(c + vec2(5.0, -3.0), 3.5, col);
            ui.painter().rect_filled(Rect::from_center_size(c + vec2(0.0, 6.0), vec2(16.0, 7.0)), CornerRadius::same(3), col);
            ui.painter().text(rect.left_center() + vec2(44.0, 0.0), Align2::LEFT_CENTER, "Friends", FontId::proportional(15.0), if sel { p.strong } else { p.text });
            if pending > 0 && self.data.settings.show_badges {
                badge(ui, rect.right_center() - vec2(18.0, 0.0), pending, p.danger, if sel { p.selected } else { p.side });
            }
            if resp.clicked() {
                open_friends = true;
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                section_label(ui, "Direct messages", p);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_space(10.0);
                    if ui
                        .add(egui::Button::new(RichText::new("+").size(16.0).color(p.muted)).frame(false))
                        .on_hover_text("New message")
                        .clicked()
                    {
                        new_dm = true;
                    }
                });
            });
            ui.add_space(4.0);
            for d in &self.data.dms {
                let Some(u) = self.data.user(d.with) else { continue };
                let (rect, resp) = row_rect(ui, 44.0);
                let sel = self.dm == Some(d.with);
                let hovered = ui.rect_contains_pointer(rect);
                let k = hover_anim(ui, EId::new(("dm_row", d.with)), hovered, self.data.settings.reduced_motion);
                let bg = if sel { p.selected } else { p.side.lerp_to_gamma(p.hover, k) };
                if sel || k > 0.0 {
                    ui.painter().rect_filled(rect, CornerRadius::same(6), bg);
                }
                paint_avatar(ui, rect.left_center() + vec2(22.0, 0.0), 32.0, &u.name, u.hue, Some(u.status), bg);
                let col = if sel || d.unread > 0 { p.strong } else { p.muted };
                let y = if u.activity.is_empty() { 0.0 } else { -7.0 };
                ui.painter().text(rect.left_center() + vec2(46.0, y), Align2::LEFT_CENTER, &u.name, FontId::proportional(14.5), col);
                if !u.activity.is_empty() {
                    ui.painter().text(rect.left_center() + vec2(46.0, 9.0), Align2::LEFT_CENTER, &u.activity, FontId::proportional(11.5), p.faint);
                }
                let mut closing = false;
                if d.unread > 0 && self.data.settings.show_badges {
                    badge(ui, rect.right_center() - vec2(18.0, 0.0), d.unread, p.danger, bg);
                } else if hovered {
                    let x = Rect::from_center_size(rect.right_center() - vec2(18.0, 0.0), vec2(20.0, 20.0));
                    let xr = ui.interact(x, EId::new(("close_dm", d.with)), Sense::click());
                    ui.painter().text(x.center(), Align2::CENTER_CENTER, "×", FontId::proportional(12.0), if xr.hovered() { p.strong } else { p.muted });
                    if xr.on_hover_text("Close conversation").clicked() {
                        close = Some(d.with);
                        closing = true;
                    }
                }
                if resp.clicked() && !closing {
                    open = Some(d.with);
                }
            }
        });
        if open_friends {
            self.open_home();
        }
        if let Some(u) = open {
            self.open_dm(u);
        }
        if let Some(u) = close {
            self.data.dms.retain(|d| d.with != u);
            if self.dm == Some(u) {
                self.dm = None;
            }
            self.dirty = true;
        }
        if new_dm {
            self.open_home();
            self.friends_tab = FriendsTab::All;
        }
    }

    fn community_sidebar(&mut self, ui: &mut Ui, p: &Palette, cid: Id, now: f64) {
        let Some(c) = self.data.community(cid).cloned() else { return };
        let mut action: Option<Dialog> = None;
        let mut toggle_mute = false;
        let header_top = ui.cursor().top();
        ui.horizontal(|ui| {
            ui.set_height(HEADER_H);
            ui.add_space(6.0);
            let title = RichText::new(format!("{}   ", c.name)).size(15.0).color(p.strong).strong();
            let menu = ui.menu_button(title, |ui| {
                ui.set_min_width(200.0);
                if ui.button("New text channel").clicked() {
                    action = Some(Dialog::NewChannel { community: cid, name: String::new(), kind: ChannelKind::Text });
                }
                if ui.button("New voice channel").clicked() {
                    action = Some(Dialog::NewChannel { community: cid, name: String::new(), kind: ChannelKind::Voice });
                }
                if ui.button("Rename community").clicked() {
                    action = Some(Dialog::RenameCommunity { community: cid, name: c.name.clone() });
                }
                if ui.button(if c.muted { "Unmute community" } else { "Mute community" }).clicked() {
                    toggle_mute = true;
                }
                if cid != HOME_NET_COMMUNITY {
                    ui.separator();
                    if ui.button(RichText::new("Leave community").color(p.danger)).clicked() {
                        action = Some(Dialog::LeaveCommunity { community: cid });
                    }
                }
            });
            let br = menu.response.rect;
            paint_chevron(ui, br.right_center() - vec2(12.0, 0.0), true, p.muted);
        });
        let r = ui.max_rect();
        ui.painter().line_segment(
            [pos2(r.left(), header_top + HEADER_H), pos2(r.right(), header_top + HEADER_H)],
            Stroke::new(1.0, p.divider),
        );
        if let Some(a) = action {
            self.dialog = Some(a);
        }
        if toggle_mute {
            if let Some(c) = self.data.community_mut(cid) {
                c.muted = !c.muted;
            }
            self.dirty = true;
        }

        if cid == HOME_NET_COMMUNITY {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                let (txt, col) = match (&self.lan, &self.lan_error) {
                    (Some(l), _) => (
                        format!("Sharing on port {} · {} nearby", l.port, self.lan_seen.len()),
                        theme::status_color(Status::Online),
                    ),
                    (None, Some(e)) => (e.clone(), p.danger),
                    (None, None) => ("Home network chat is off (see Settings)".into(), p.faint),
                };
                ui.add(egui::Label::new(RichText::new(txt).size(11.5).color(col)).wrap());
            });
        }

        let current = self.current_channel(cid);
        let mut open_text: Option<Id> = None;
        let mut join: Option<Id> = None;
        let mut new_kind: Option<ChannelKind> = None;
        let mut toggle_cat: Option<bool> = None;
        let mut edit_topic: Option<(Id, String)> = None;
        egui::ScrollArea::vertical().id_salt(("chan_side", cid)).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for (voice, label) in [(false, "Text channels"), (true, "Voice channels")] {
                let kind = if voice { ChannelKind::Voice } else { ChannelKind::Text };

                let collapsed = self.collapsed.contains(&(cid, voice));
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    let (tri, tri_resp) = ui.allocate_exact_size(vec2(10.0, 14.0), Sense::click());
                    paint_chevron(ui, tri.center(), !collapsed, p.muted);
                    let r = ui.add(
                        egui::Button::new(RichText::new(label.to_uppercase()).size(11.5).color(p.muted).strong()).frame(false),
                    );
                    if r.clicked() || tri_resp.clicked() {
                        toggle_cat = Some(voice);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(10.0);
                        if ui
                            .add(egui::Button::new(RichText::new("+").size(15.0).color(p.muted)).frame(false))
                            .on_hover_text("Create channel")
                            .clicked()
                        {
                            new_kind = Some(kind);
                        }
                    });
                });
                for ch in c.channels.iter().filter(|x| x.kind == kind) {
                    let sel = !voice && current == Some(ch.id);
                    let in_voice = self.voice == Some((cid, ch.id));
                    if collapsed && !sel && ch.unread == 0 && !in_voice {
                        continue;
                    }
                    let (rect, resp) = row_rect(ui, 32.0);
                    let k = hover_anim(ui, EId::new(("chan_row", ch.id)), resp.hovered(), self.data.settings.reduced_motion);
                    if sel || in_voice || k > 0.0 {
                        let fill = if sel || in_voice { p.selected } else { p.side.lerp_to_gamma(p.hover, k) };
                        ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
                    }
                    let unread = ch.unread > 0 && !c.muted;
                    let col = if sel || unread || in_voice {
                        p.strong
                    } else if resp.hovered() {
                        p.text
                    } else {
                        p.muted
                    };
                    let ic = rect.left_center() + vec2(16.0, 0.0);
                    if voice {
                        paint_speaker(ui, ic, col);
                    } else {
                        paint_hash(ui, ic, col);
                    }
                    ui.painter().text(rect.left_center() + vec2(32.0, 0.0), Align2::LEFT_CENTER, &ch.name, FontId::proportional(14.5), col);
                    if unread && !sel {
                        ui.painter().circle_filled(pos2(rect.left() - 4.0, rect.center().y), 3.5, p.strong);
                    }
                    if ch.mentions > 0 && self.data.settings.show_badges {
                        badge(ui, rect.right_center() - vec2(16.0, 0.0), ch.mentions, p.danger, p.side);
                    }
                    if resp.clicked() {
                        if voice {
                            join = Some(ch.id);
                        } else {
                            open_text = Some(ch.id);
                        }
                    }
                    if voice {
                        resp.on_hover_text(if in_voice { "Open the voice stage" } else { "Click to join" });
                        for (k, pid) in ch.voice_people.iter().enumerate() {
                            let Some(u) = self.data.user(*pid) else { continue };
                            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
                            let ac = r.left_center() + vec2(46.0, 0.0);
                            let speaking = self.speaking(*pid, now, k);
                            if speaking {
                                ui.painter().circle_stroke(ac, 12.5, Stroke::new(2.0, theme::status_color(Status::Online)));
                            }
                            paint_avatar(ui, ac, 22.0, &u.name, u.hue, None, p.side);
                            ui.painter().text(ac + vec2(18.0, 0.0), Align2::LEFT_CENTER, &u.name, FontId::proportional(13.5), p.muted);
                            if *pid == ME && (self.mic_muted || self.deafened) {
                                ui.painter().text(
                                    r.right_center() - vec2(16.0, 0.0),
                                    Align2::RIGHT_CENTER,
                                    if self.deafened { "🎧" } else { "🎤" },
                                    FontId::proportional(12.0),
                                    p.danger,
                                );
                            }
                        }
                    } else {
                        let topic = ch.topic.clone();
                        let id = ch.id;
                        resp.context_menu(|ui| {
                            if ui.button("Edit topic").clicked() {
                                edit_topic = Some((id, topic.clone()));
                                ui.close();
                            }
                            if ui.button("Mark as read").clicked() {
                                open_text = Some(id);
                                ui.close();
                            }
                        });
                    }
                }
            }
            ui.add_space(12.0);
        });
        if let Some(v) = toggle_cat {
            if !self.collapsed.remove(&(cid, v)) {
                self.collapsed.insert((cid, v));
            }
        }
        if let Some(ch) = open_text {
            self.open_channel(cid, ch);
            self.show_stage = false;
        }
        if let Some(ch) = join {
            if self.voice == Some((cid, ch)) {
                self.show_stage = true;
            } else {
                self.join_voice(cid, ch);
            }
        }
        if let Some(kind) = new_kind {
            self.dialog = Some(Dialog::NewChannel { community: cid, name: String::new(), kind });
        }
        if let Some((ch, topic)) = edit_topic {
            self.dialog = Some(Dialog::EditTopic { community: cid, channel: ch, topic });
        }
    }

    // ------------------------------------------------------------ member list

    fn members(&mut self, ui: &mut Ui, p: &Palette) {
        let Some(cid) = self.community else { return };
        let Some(c) = self.data.community(cid) else { return };
        let mut people: Vec<User> = c.members.iter().filter_map(|id| self.data.user(*id).cloned()).collect();
        people.sort_by_key(|u| u.name.to_lowercase());
        let (on, off): (Vec<User>, Vec<User>) = people.into_iter().partition(|u| u.status.shows_online());
        let mut open: Option<(Id, Pos2)> = None;
        egui::Panel::right("members")
            .exact_size(MEMBERS_W)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::new().fill(p.side).inner_margin(Margin { left: 0, right: 0, top: 12, bottom: 8 }))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("members_scroll").show(ui, |ui| {
                    for (label, list, dim) in [("Online", &on, false), ("Offline", &off, true)] {
                        if list.is_empty() {
                            continue;
                        }
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            ui.add_space(16.0);
                            section_label(ui, &format!("{label} — {}", list.len()), p);
                        });
                        ui.add_space(4.0);
                        for u in list.iter() {
                            let (rect, resp) = row_rect(ui, 44.0);
                            let k = hover_anim(ui, EId::new(("member_row", u.id)), resp.hovered(), self.data.settings.reduced_motion);
                            let bg = p.side.lerp_to_gamma(p.hover, k);
                            if k > 0.0 {
                                ui.painter().rect_filled(rect, CornerRadius::same(6), bg);
                            }
                            let a = rect.left_center() + vec2(22.0, 0.0);
                            paint_avatar(ui, a, 32.0, &u.name, u.hue, if dim { None } else { Some(u.status) }, bg);
                            if dim {
                                let veil = if p.dark { Color32::from_black_alpha(120) } else { Color32::from_white_alpha(130) };
                                ui.painter().circle_filled(a, 16.5, veil);
                            }
                            let name_col = if dim { p.faint } else { theme::hue(u.hue).lerp_to_gamma(p.strong, 0.45) };
                            let y = if u.activity.is_empty() || dim { 0.0 } else { -7.0 };
                            let label = if u.id == ME && u.name != "You" { format!("{} (you)", u.name) } else { u.name.clone() };
                            ui.painter().text(rect.left_center() + vec2(46.0, y), Align2::LEFT_CENTER, label, FontId::proportional(14.5), name_col);
                            if !(u.activity.is_empty() || dim) {
                                ui.painter().text(rect.left_center() + vec2(46.0, 9.0), Align2::LEFT_CENTER, &u.activity, FontId::proportional(11.5), p.muted);
                            }
                            if resp.clicked() {
                                open = Some((u.id, rect.left_top() - vec2(296.0, 0.0)));
                            }
                        }
                    }
                });
            });
        if let Some(o) = open {
            self.profile = Some(o);
        }
    }

    fn dm_profile_panel(&mut self, ui: &mut Ui, p: &Palette, uid: Id) {
        let Some(u) = self.data.user(uid).cloned() else { return };
        if !self.data.settings.show_members {
            return;
        }
        let mutual: Vec<String> =
            self.data.communities.iter().filter(|c| c.members.contains(&uid)).map(|c| c.name.clone()).collect();
        let mut toggle_friend = false;
        egui::Panel::right("dm_profile")
            .exact_size(MEMBERS_W + 30.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::new().fill(p.side))
            .show(ui, |ui| {
                let (banner, _) = ui.allocate_exact_size(vec2(ui.available_width(), 100.0), Sense::hover());
                ui.painter().rect_filled(banner, CornerRadius::ZERO, theme::hue(u.hue));
                let ac = banner.left_bottom() + vec2(56.0, 0.0);
                ui.painter().circle_filled(ac, 44.0, p.side);
                paint_avatar(ui, ac, 80.0, &u.name, u.hue, Some(u.status), p.side);
                ui.add_space(52.0);
                Frame::new().inner_margin(Margin::symmetric(16, 0)).show(ui, |ui| {
                    ui.label(RichText::new(&u.name).size(20.0).color(p.strong).strong());
                    ui.label(RichText::new(u.status.label()).color(p.muted));
                    ui.add_space(12.0);
                    Frame::new().fill(p.raised).corner_radius(8.0).inner_margin(12.0).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        section_label(ui, "About", p);
                        ui.label(RichText::new(&u.about).color(p.text));
                        if !u.activity.is_empty() {
                            ui.add_space(8.0);
                            section_label(ui, "Right now", p);
                            ui.label(RichText::new(&u.activity).color(p.text));
                        }
                    });
                    ui.add_space(10.0);
                    if !mutual.is_empty() {
                        Frame::new().fill(p.raised).corner_radius(8.0).inner_margin(12.0).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            section_label(ui, &format!("Shared communities — {}", mutual.len()), p);
                            for m in &mutual {
                                ui.label(RichText::new(m).color(p.text));
                            }
                        });
                        ui.add_space(10.0);
                    }
                    if uid < 1_000_000 {
                        let label = if u.friend { "Remove friend" } else { "Add friend" };
                        if ui.button(label).clicked() {
                            toggle_friend = true;
                        }
                    }
                });
            });
        if toggle_friend {
            if let Some(u) = self.data.user_mut(uid) {
                u.friend = !u.friend;
                u.pending_in = false;
                u.pending_out = false;
            }
            self.dirty = true;
        }
    }

    fn search_results(&mut self, ui: &mut Ui, p: &Palette) {
        let q = self.search_query.to_lowercase();
        let mut hits: Vec<(Target, String, Message)> = vec![];
        match self.current_target() {
            Some(Target::Channel(c, _)) => {
                if let Some(com) = self.data.community(c) {
                    for ch in com.channels.iter().filter(|x| x.kind == ChannelKind::Text) {
                        for m in &ch.messages {
                            if m.text.to_lowercase().contains(&q) || self.display_name(m).to_lowercase().contains(&q) {
                                hits.push((Target::Channel(c, ch.id), format!("#{}", ch.name), m.clone()));
                            }
                        }
                    }
                }
            }
            Some(Target::Dm(u)) => {
                if let Some(d) = self.data.dms.iter().find(|d| d.with == u) {
                    for m in &d.messages {
                        if m.text.to_lowercase().contains(&q) {
                            hits.push((Target::Dm(u), "Direct message".into(), m.clone()));
                        }
                    }
                }
            }
            None => {}
        }
        hits.sort_by_key(|h| -h.2.ts);
        let mut go: Option<(Target, Id)> = None;
        let mut close = false;
        egui::Panel::right("search")
            .exact_size(MEMBERS_W + 120.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::new().fill(p.side).inner_margin(12.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let n = hits.len();
                    ui.label(RichText::new(format!("{n} result{}", if n == 1 { "" } else { "s" })).color(p.strong).strong());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if icon_button(ui, "×", "Close search", false, p).clicked() {
                            close = true;
                        }
                    });
                });
                ui.add_space(6.0);
                egui::ScrollArea::vertical().id_salt("search_scroll").show(ui, |ui| {
                    if hits.is_empty() {
                        ui.add_space(30.0);
                        ui.vertical_centered(|ui| ui.label(RichText::new("Nothing found. Try another word.").color(p.muted)));
                    }
                    for (t, place, m) in &hits {
                        let r = Frame::new().fill(p.main).corner_radius(8.0).inner_margin(10.0).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(self.display_name(m)).color(p.strong).strong());
                                ui.label(RichText::new(format!("{place} · {}", time_label(m.ts))).color(p.faint).size(11.0));
                            });
                            ui.label(RichText::new(&m.text).color(p.text));
                        });
                        if ui.interact(r.response.rect, EId::new(("hit", m.id)), Sense::click()).on_hover_text("Jump to message").clicked() {
                            go = Some((*t, m.id));
                        }
                        ui.add_space(6.0);
                    }
                });
            });
        if close {
            self.search_query.clear();
            self.search.clear();
        }
        if let Some((t, mid)) = go {
            if let Target::Channel(c, ch) = t {
                if Some(t) != self.current_target() {
                    self.open_channel(c, ch);
                }
            }
            self.search_query.clear();
            self.search.clear();
            self.jump_to = Some(mid);
        }
    }

    fn pins_panel(&mut self, ui: &mut Ui, p: &Palette) {
        let Some(t) = self.current_target() else { return };
        let pins: Vec<Message> =
            self.messages(t).map(|l| l.iter().filter(|m| m.pinned).cloned().collect()).unwrap_or_default();
        let mut close = false;
        let mut unpin: Option<Id> = None;
        let mut go: Option<Id> = None;
        egui::Panel::right("pins")
            .exact_size(MEMBERS_W + 80.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::new().fill(p.side).inner_margin(12.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("📌 Pinned messages").color(p.strong).strong());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if icon_button(ui, "×", "Close", false, p).clicked() {
                            close = true;
                        }
                    });
                });
                ui.add_space(6.0);
                if pins.is_empty() {
                    ui.add_space(30.0);
                    ui.label(RichText::new("Nothing pinned yet. Hover a message and press 📌 to keep it here.").color(p.muted));
                }
                egui::ScrollArea::vertical().id_salt("pins_scroll").show(ui, |ui| {
                    for m in &pins {
                        Frame::new().fill(p.main).corner_radius(8.0).inner_margin(10.0).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(self.display_name(m)).color(p.strong).strong());
                                ui.label(RichText::new(time_label(m.ts)).color(p.faint).size(11.0));
                            });
                            ui.label(RichText::new(&m.text).color(p.text));
                            ui.horizontal(|ui| {
                                if ui.small_button("Jump").clicked() {
                                    go = Some(m.id);
                                }
                                if ui.small_button("Unpin").clicked() {
                                    unpin = Some(m.id);
                                }
                            });
                        });
                        ui.add_space(6.0);
                    }
                });
            });
        if close {
            self.show_pins = false;
        }
        if let Some(id) = unpin {
            if let Some(m) = self.messages_mut(t).and_then(|l| l.iter_mut().find(|m| m.id == id)) {
                m.pinned = false;
            }
            self.dirty = true;
        }
        if let Some(id) = go {
            self.jump_to = Some(id);
        }
    }

    // ------------------------------------------------------------ chat

    fn chat(&mut self, ui: &mut Ui, p: &Palette, t: Target, now: f64) {
        let (title, topic, is_channel) = match t {
            Target::Channel(c, ch) => {
                let x = self.data.community(c).and_then(|c| c.channels.iter().find(|x| x.id == ch));
                (x.map(|x| x.name.clone()).unwrap_or_default(), x.map(|x| x.topic.clone()).unwrap_or_default(), true)
            }
            Target::Dm(u) => {
                let x = self.data.user(u);
                (x.map(|u| u.name.clone()).unwrap_or_default(), x.map(|u| u.activity.clone()).unwrap_or_default(), false)
            }
        };
        egui::Panel::top("chat_header")
            .exact_size(HEADER_H)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::new().fill(p.main).inner_margin(Margin::symmetric(14, 0)))
            .show(ui, |ui| {
                let full = ui.max_rect();
                ui.painter().line_segment([full.left_bottom(), full.right_bottom()], Stroke::new(1.0, p.divider));
                ui.horizontal_centered(|ui| {
                    if is_channel {
                        let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                        paint_hash(ui, r.center(), p.muted);
                    } else if let Target::Dm(u) = t {
                        if let Some(u) = self.data.user(u) {
                            let (r, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
                            paint_avatar(ui, r.center(), 24.0, &u.name, u.hue, Some(u.status), p.main);
                        }
                    }
                    ui.label(RichText::new(&title).size(16.0).color(p.strong).strong());
                    if !topic.is_empty() {
                        ui.add_space(6.0);
                        let (r, _) = ui.allocate_exact_size(vec2(1.0, 22.0), Sense::hover());
                        ui.painter().rect_filled(r, CornerRadius::ZERO, p.hover);
                        ui.add_space(6.0);
                        let w = (ui.available_width() - 260.0).max(40.0);
                        ui.add_sized([w, 20.0], egui::Label::new(RichText::new(&topic).color(p.muted).size(13.0)).truncate());
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut self.search)
                                .hint_text("Search")
                                .desired_width(150.0)
                                .background_color(p.raised),
                        );
                        if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                            self.search_query = self.search.trim().to_string();
                            self.show_pins = false;
                        }
                        ui.add_space(4.0);
                        let label = if is_channel { "Member list" } else { "Profile" };
                        if icon_button(ui, "👥", label, self.data.settings.show_members, p).clicked() {
                            self.data.settings.show_members = !self.data.settings.show_members;
                            self.show_pins = false;
                            self.dirty = true;
                        }
                        if icon_button(ui, "📌", "Pinned messages", self.show_pins, p).clicked() {
                            self.show_pins = !self.show_pins;
                            self.search_query.clear();
                        }
                    });
                });
            });

        let typing: Vec<String> = self
            .pending
            .iter()
            .filter(|r| r.target == t && now >= r.typing_from)
            .filter_map(|r| self.data.user(r.author).map(|u| u.name.clone()))
            .collect();
        let reply_name =
            self.reply_to.and_then(|id| self.messages(t).and_then(|l| l.iter().find(|m| m.id == id)).map(|m| self.display_name(m)));
        let placeholder = if is_channel { format!("Message #{title}") } else { format!("Message {title}") };
        let mut send = false;
        let mut edit_last = false;
        let mut open_picker: Option<Pos2> = None;
        let mut cancel_reply = false;
        egui::Panel::bottom("composer")
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::new().fill(p.main).inner_margin(Margin { left: 16, right: 16, top: 0, bottom: 4 }))
            .show(ui, |ui| {
                if let Some(n) = &reply_name {
                    Frame::new()
                        .fill(p.raised)
                        .corner_radius(CornerRadius { nw: 10, ne: 10, sw: 0, se: 0 })
                        .inner_margin(Margin::symmetric(12, 6))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(format!("↩ Replying to {n}")).color(p.muted).size(13.0));
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui.add(egui::Button::new(RichText::new("×").color(p.muted)).frame(false)).clicked() {
                                        cancel_reply = true;
                                    }
                                });
                            });
                        });
                }
                let rounding = if reply_name.is_some() {
                    CornerRadius { nw: 0, ne: 0, sw: 10, se: 10 }
                } else {
                    CornerRadius::same(10)
                };
                Frame::new().fill(p.input).corner_radius(rounding).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let w = ui.available_width() - 40.0;
                        let enter_sends = self.data.settings.enter_sends;
                        // Enter sends (Shift+Enter = new line), or Ctrl+Enter sends (Enter = new line).
                        let newline_key = if enter_sends {
                            egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, Key::Enter)
                        } else {
                            egui::KeyboardShortcut::new(egui::Modifiers::NONE, Key::Enter)
                        };
                        let resp = ui.add(
                            egui::TextEdit::multiline(&mut self.composer)
                                .id_salt("composer")
                                .hint_text(RichText::new(&placeholder).color(p.faint))
                                .desired_rows(1)
                                .desired_width(w)
                                .frame(Frame::NONE)
                                .char_limit(MAX_MESSAGE_CHARS)
                                .return_key(Some(newline_key))
                                .font(FontId::proportional(15.0)),
                        );
                        if self.focus_composer && self.dialog.is_none() && self.editing.is_none() {
                            resp.request_focus();
                            self.focus_composer = false;
                        }
                        if resp.has_focus() {
                            let (enter, up) =
                                ui.input(|i| (i.key_pressed(Key::Enter) && !i.modifiers.shift && (enter_sends || i.modifiers.command), i.key_pressed(Key::ArrowUp)));
                            if enter {
                                send = true;
                            }
                            if up && self.composer.is_empty() {
                                edit_last = true;
                            }
                        }
                        let e = icon_button(ui, "😊", "Emoji", matches!(self.picker, Some((PickerFor::Composer, _))), p);
                        if e.clicked() {
                            open_picker = Some(e.rect.left_top() - vec2(290.0, 250.0));
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.set_height(20.0);
                    if !typing.is_empty() && self.data.settings.show_typing {
                        let dots = if self.data.settings.reduced_motion { "...".to_string() } else { ".".repeat(1 + ((now * 3.0) as usize % 3)) };
                        ui.label(RichText::new(format!("{} is typing{dots}", typing.join(", "))).size(12.0).color(p.muted));
                    }
                    let n = self.composer.chars().count();
                    if n > MAX_MESSAGE_CHARS - 200 {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(format!("{} left", MAX_MESSAGE_CHARS - n)).size(12.0).color(p.danger));
                        });
                    }
                });
            });
        if cancel_reply {
            self.reply_to = None;
        }
        if send {
            self.send(now);
            self.focus_composer = true;
        }
        if edit_last {
            if let Some(m) = self.messages(t).and_then(|l| l.iter().rev().find(|m| m.author == ME)) {
                self.editing = Some((m.id, m.text.clone()));
            }
        }
        if let Some(pos) = open_picker {
            self.picker = if matches!(self.picker, Some((PickerFor::Composer, _))) { None } else { Some((PickerFor::Composer, pos)) };
        }

        let msgs: Vec<Message> = self.messages(t).cloned().unwrap_or_default();
        let names: Vec<String> = self.data.users.iter().map(|u| u.name.clone()).collect();
        let me_name = self.data.me().name.clone();
        let mut actions: Vec<MsgAction> = vec![];
        let compact = self.data.settings.compact;
        let scroll_id = match t {
            Target::Channel(_, ch) => ch,
            Target::Dm(u) => u + 7_000_000_000,
        };
        let view_t = self.ease(self.view_changed_at, 0.22);
        egui::CentralPanel::default().frame(Frame::new().fill(p.main)).show(ui, |ui| {
            ui.multiply_opacity(view_t);
            egui::ScrollArea::vertical()
                .id_salt(("chat_scroll", scroll_id))
                .stick_to_bottom(true)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_space(16.0);
                    Frame::new().inner_margin(Margin::symmetric(16, 8)).show(ui, |ui| {
                        if is_channel {
                            let (r, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
                            ui.painter().circle_filled(r.center(), 32.0, p.input);
                            paint_hash(ui, r.center(), p.strong);
                            ui.label(RichText::new(format!("Welcome to #{title}")).size(26.0).color(p.strong).strong());
                            ui.label(RichText::new(format!("This is the start of #{title}. {topic}")).color(p.muted));
                        } else if let Target::Dm(u) = t {
                            if let Some(u) = self.data.user(u) {
                                let (r, _) = ui.allocate_exact_size(vec2(72.0, 72.0), Sense::hover());
                                paint_avatar(ui, r.center(), 72.0, &u.name, u.hue, None, p.main);
                                ui.label(RichText::new(&u.name).size(26.0).color(p.strong).strong());
                                ui.label(
                                    RichText::new(format!("This is the beginning of your conversation with {}.", u.name)).color(p.muted),
                                );
                            }
                        }
                    });
                    ui.add_space(8.0);
                    let mut prev: Option<&Message> = None;
                    for m in &msgs {
                        let new_day = (self.data.settings.day_dividers || prev.is_none()) && prev.map(|pm| day_label(pm.ts) != day_label(m.ts)).unwrap_or(true);
                        if new_day {
                            ui.add_space(10.0);
                            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
                            let g = ui.painter().layout_no_wrap(day_label(m.ts), FontId::proportional(11.5), p.muted);
                            let mid = r.center();
                            let gap = g.size().x / 2.0 + 10.0;
                            ui.painter().line_segment([pos2(r.left() + 16.0, mid.y), pos2(mid.x - gap, mid.y)], Stroke::new(1.0, p.hover));
                            ui.painter().line_segment([pos2(mid.x + gap, mid.y), pos2(r.right() - 16.0, mid.y)], Stroke::new(1.0, p.hover));
                            ui.painter().galley(mid - g.size() / 2.0, g, p.muted);
                            ui.add_space(4.0);
                        }
                        let grouped = self.data.settings.group_messages && !new_day
                            && m.reply_to.is_none()
                            && prev
                                .map(|pm| {
                                    pm.author == m.author && pm.author_name == m.author_name && m.ts - pm.ts < GROUP_WINDOW_SECS
                                })
                                .unwrap_or(false);
                        let reply_src = m.reply_to.and_then(|rid| msgs.iter().find(|x| x.id == rid));
                        self.message_row(ui, p, m, grouped, compact, reply_src, &names, &me_name, &mut actions);
                        prev = Some(m);
                    }
                    ui.add_space(12.0);
                });
        });

        for a in actions {
            self.apply(t, a);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn message_row(
        &mut self,
        ui: &mut Ui,
        p: &Palette,
        m: &Message,
        grouped: bool,
        compact: bool,
        reply_src: Option<&Message>,
        names: &[String],
        me_name: &str,
        actions: &mut Vec<MsgAction>,
    ) {
        // New messages slide in and fade up.
        let born_t = match self.msg_born.get(&m.id).copied() {
            Some(t0) => self.ease(t0, 0.3),
            None => 1.0,
        };
        let prev_opacity = ui.opacity();
        ui.multiply_opacity(born_t);
        let slide = (1.0 - born_t) * 18.0;
        let bg_idx = ui.painter().add(egui::Shape::Noop);
        let full_w = ui.available_width();
        let left = ui.max_rect().left();
        let left_pad = if compact { 16.0 } else { 72.0 } + slide;
        let mention = m.author != ME && m.text.to_lowercase().contains(&format!("@{}", me_name.to_lowercase()));
        let name = self.display_name(m);
        let hue = self.author_hue(m);
        let show_ts = self.data.settings.show_timestamps;
        let text_w = (full_w - left_pad - 24.0).max(100.0);
        let name_col = theme::hue(hue).lerp_to_gamma(p.strong, 0.35);

        ui.add_space(if !grouped && !compact { 14.0 } else { 1.0 });
        let row_top = ui.cursor().top() - if !grouped && !compact { 4.0 } else { 1.0 };
        let content_top = ui.cursor().top();
        let mut edit_buf = self.editing.as_ref().filter(|(id, _)| *id == m.id).map(|(_, s)| s.clone());
        let inner = ui.scope_builder(
            UiBuilder::new().max_rect(Rect::from_min_size(pos2(left + left_pad, content_top), vec2(text_w, 100_000.0))),
            |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                if let Some(src) = reply_src {
                    let txt: String = src.text.chars().take(80).collect();
                    let r = ui.add(
                        egui::Label::new(RichText::new(format!("↪ {}: {}", self.display_name(src), txt)).size(12.5).color(p.muted))
                            .truncate()
                            .sense(Sense::click()),
                    );
                    if r.on_hover_text("Jump to the original").clicked() {
                        actions.push(MsgAction::Jump(src.id));
                    }
                }
                if compact {
                    ui.horizontal_wrapped(|ui| {
                        if show_ts {
                            ui.label(RichText::new(short_time(m.ts)).size(11.0).color(p.faint));
                        }
                        let r = ui.add(egui::Label::new(RichText::new(&name).color(name_col).strong()).sense(Sense::click()));
                        if r.clicked() {
                            actions.push(MsgAction::Profile(m.author, r.rect.right_top() + vec2(8.0, 0.0)));
                        }
                        if edit_buf.is_none() {
                            let mut job = rich_text(&m.text, p, names, 15.0, text_w - 120.0);
                            if m.edited {
                                job.append(" (edited)", 0.0, egui::text::TextFormat { font_id: FontId::proportional(10.5), color: p.faint, ..Default::default() });
                            }
                            ui.label(job);
                        }
                    });
                } else if !grouped {
                    ui.horizontal(|ui| {
                        let r = ui.add(egui::Label::new(RichText::new(&name).size(15.0).color(name_col).strong()).sense(Sense::click()));
                        if r.clicked() {
                            actions.push(MsgAction::Profile(m.author, r.rect.right_top() + vec2(8.0, 0.0)));
                        }
                        if m.author_name.is_some() {
                            let (b, _) = ui.allocate_exact_size(vec2(66.0, 16.0), Sense::hover());
                            ui.painter().rect_filled(b, CornerRadius::same(4), p.accent);
                            ui.painter().text(b.center(), Align2::CENTER_CENTER, "NEARBY", FontId::proportional(10.0), p.on_accent);
                        }
                        if show_ts {
                            ui.label(RichText::new(time_label(m.ts)).size(11.5).color(p.faint));
                        }
                    });
                }
                if let Some(buf) = &mut edit_buf {
                    let r = ui.add(
                        egui::TextEdit::multiline(buf)
                            .id_salt(("edit", m.id))
                            .desired_rows(1)
                            .desired_width(text_w)
                            .char_limit(MAX_MESSAGE_CHARS)
                            .return_key(Some(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, Key::Enter)))
                            .background_color(p.input),
                    );
                    if !r.has_focus() && !r.lost_focus() {
                        r.request_focus();
                    }
                    if r.has_focus() && ui.input(|i| i.key_pressed(Key::Enter) && !i.modifiers.shift) {
                        actions.push(MsgAction::SaveEdit(m.id));
                    }
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Enter to save · Esc to cancel · empty deletes").size(11.5).color(p.muted));
                        if ui.small_button("Save").clicked() {
                            actions.push(MsgAction::SaveEdit(m.id));
                        }
                        if ui.small_button("Cancel").clicked() {
                            actions.push(MsgAction::CancelEdit);
                        }
                    });
                } else if !compact {
                    let mut job = rich_text(&m.text, p, names, 15.0, text_w);
                    if m.edited {
                        job.append(
                            " (edited)",
                            0.0,
                            egui::text::TextFormat { font_id: FontId::proportional(10.5), color: p.faint, ..Default::default() },
                        );
                    }
                    ui.label(job);
                }
                if !m.reactions.is_empty() {
                    ui.add_space(2.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for r in &m.reactions {
                            let mine = r.by.contains(&ME);
                            let label = format!("{} {}", r.emoji, r.by.len());
                            let g = ui.painter().layout_no_wrap(label, FontId::proportional(13.5), if mine { p.strong } else { p.text });
                            let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 16.0, 24.0), Sense::click());
                            ui.painter().rect_filled(rect, CornerRadius::same(8), if mine { p.mention_bg } else { p.input });
                            if mine || resp.hovered() {
                                ui.painter().rect_stroke(
                                    rect,
                                    CornerRadius::same(8),
                                    Stroke::new(1.0, if mine { p.accent } else { p.faint }),
                                    StrokeKind::Inside,
                                );
                            }
                            ui.painter().galley(rect.center() - g.size() / 2.0, g, p.text);
                            let who: Vec<String> = r
                                .by
                                .iter()
                                .filter_map(|id| self.data.user(*id).map(|u| if u.id == ME { "you".to_string() } else { u.name.clone() }))
                                .collect();
                            if resp.on_hover_text(format!("{} reacted with {}", who.join(", "), r.emoji)).clicked() {
                                actions.push(MsgAction::React(m.id, r.emoji.clone()));
                            }
                        }
                    });
                }
            },
        );
        if let (Some(b), Some((id, s))) = (edit_buf, &mut self.editing) {
            if *id == m.id {
                *s = b;
            }
        }
        let min_h = if grouped || compact { 20.0 } else { 40.0 };
        let content_h = inner.response.rect.height().max(min_h);
        let bottom = content_top + content_h;
        let used = bottom - ui.cursor().top();
        if used > 0.0 {
            ui.allocate_exact_size(vec2(full_w, used), Sense::hover());
        }
        ui.add_space(2.0);
        let row = Rect::from_min_max(pos2(left, row_top), pos2(left + full_w, ui.cursor().top()));

        if !compact {
            if !grouped {
                let c = pos2(left + 36.0, content_top + 20.0);
                let ar = Rect::from_center_size(c, vec2(AVATAR + 2.0, AVATAR + 2.0));
                paint_avatar(ui, c, AVATAR + 2.0, &name, hue, None, p.main);
                if ui.interact(ar, EId::new(("av", m.id)), Sense::click()).clicked() {
                    actions.push(MsgAction::Profile(m.author, ar.right_top() + vec2(8.0, 0.0)));
                }
            } else if ui.rect_contains_pointer(row) && show_ts {
                ui.painter().text(pos2(left + 36.0, content_top + 10.0), Align2::CENTER_CENTER, short_time(m.ts), FontId::proportional(10.5), p.faint);
            }
        }

        let is_editing = matches!(&self.editing, Some((e, _)) if *e == m.id);
        let hovered = ui.rect_contains_pointer(row) && !is_editing && self.picker.is_none();
        let highlight = self.jump_to == Some(m.id);
        let bg = if highlight {
            Some(p.selected)
        } else if mention {
            Some(p.mention_bg)
        } else if hovered {
            Some(p.hover.gamma_multiply(0.75))
        } else {
            None
        };
        if let Some(bg) = bg {
            ui.painter().set(bg_idx, egui::Shape::rect_filled(row, CornerRadius::ZERO, bg));
        }
        if mention {
            ui.painter().rect_filled(Rect::from_min_size(row.min, vec2(3.0, row.height())), CornerRadius::ZERO, p.accent);
        }
        if m.pinned && !hovered {
            ui.painter().text(row.right_top() + vec2(-14.0, 14.0), Align2::RIGHT_CENTER, "📌", FontId::proportional(11.0), p.faint);
        }
        if highlight {
            ui.scroll_to_rect(row, Some(Align::Center));
            self.jump_to = None;
        }

        if hovered {
            let mut tools: Vec<(&str, &str, u8)> = vec![
                ("😊", "Add reaction", 0),
                ("👍", "Thumbs up", 1),
                ("↩", "Reply", 2),
                ("📌", if m.pinned { "Unpin" } else { "Pin" }, 3),
            ];
            if m.author == ME {
                tools.push(("✏", "Edit", 4));
                tools.push(("🗑", "Delete", 5));
            }
            let w = tools.len() as f32 * 30.0 + 8.0;
            let bar = Rect::from_min_size(pos2(row.right() - w - 16.0, row.top() - 12.0), vec2(w, 32.0));
            ui.painter().rect_filled(bar.translate(vec2(0.0, 2.0)), CornerRadius::same(8), Color32::from_black_alpha(50));
            ui.painter().rect_filled(bar, CornerRadius::same(8), p.raised);
            ui.painter().rect_stroke(bar, CornerRadius::same(8), Stroke::new(1.0, p.divider), StrokeKind::Inside);
            for (i, (g, tip, k)) in tools.iter().enumerate() {
                let r = Rect::from_min_size(bar.min + vec2(4.0 + i as f32 * 30.0, 2.0), vec2(28.0, 28.0));
                let resp = ui.interact(r, EId::new(("tool", m.id, *k)), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(r, CornerRadius::same(6), p.hover);
                }
                let col = if *k == 5 {
                    p.danger
                } else if resp.hovered() {
                    p.strong
                } else {
                    p.muted
                };
                ui.painter().text(r.center(), Align2::CENTER_CENTER, *g, FontId::proportional(15.0), col);
                if resp.on_hover_text(*tip).clicked() {
                    actions.push(match k {
                        0 => MsgAction::Picker(m.id, bar.left_bottom() - vec2(200.0, 0.0)),
                        1 => MsgAction::React(m.id, "👍".into()),
                        2 => MsgAction::Reply(m.id),
                        3 => MsgAction::Pin(m.id),
                        4 => MsgAction::StartEdit(m.id),
                        _ => MsgAction::Delete(m.id),
                    });
                }
            }
        }
        ui.set_opacity(prev_opacity);
    }

    fn apply(&mut self, t: Target, a: MsgAction) {
        match a {
            MsgAction::React(id, e) => self.toggle_reaction(t, id, &e, ME),
            MsgAction::Reply(id) => {
                self.reply_to = Some(id);
                self.focus_composer = true;
            }
            MsgAction::StartEdit(id) => {
                if let Some(m) = self.messages(t).and_then(|l| l.iter().find(|m| m.id == id)) {
                    self.editing = Some((id, m.text.clone()));
                }
            }
            MsgAction::SaveEdit(id) => {
                if let Some((eid, text)) = self.editing.take() {
                    if eid == id {
                        let text = text.trim().to_string();
                        if text.is_empty() {
                            if let Some(l) = self.messages_mut(t) {
                                l.retain(|m| m.id != id);
                            }
                        } else if let Some(m) = self.messages_mut(t).and_then(|l| l.iter_mut().find(|m| m.id == id)) {
                            if m.text != text {
                                m.text = text;
                                m.edited = true;
                            }
                        }
                        self.dirty = true;
                    }
                }
                self.focus_composer = true;
            }
            MsgAction::CancelEdit => {
                self.editing = None;
                self.focus_composer = true;
            }
            MsgAction::Delete(id) => {
                if let Some(l) = self.messages_mut(t) {
                    l.retain(|m| m.id != id);
                }
                if self.reply_to == Some(id) {
                    self.reply_to = None;
                }
                self.dirty = true;
            }
            MsgAction::Pin(id) => {
                if let Some(m) = self.messages_mut(t).and_then(|l| l.iter_mut().find(|m| m.id == id)) {
                    m.pinned = !m.pinned;
                }
                self.dirty = true;
            }
            MsgAction::Profile(u, pos) => {
                if self.data.user(u).is_some() {
                    self.profile = Some((u, pos));
                }
            }
            MsgAction::Picker(id, pos) => self.picker = Some((PickerFor::Reaction(id), pos)),
            MsgAction::Jump(id) => self.jump_to = Some(id),
        }
    }

    // ------------------------------------------------------------ friends (home)

    fn friends(&mut self, ui: &mut Ui, p: &Palette) {
        let mut tab = self.friends_tab;
        egui::Panel::top("friends_header")
            .exact_size(HEADER_H)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::new().fill(p.main).inner_margin(Margin::symmetric(14, 0)))
            .show(ui, |ui| {
                let full = ui.max_rect();
                ui.painter().line_segment([full.left_bottom(), full.right_bottom()], Stroke::new(1.0, p.divider));
                ui.horizontal_centered(|ui| {
                    ui.label(RichText::new("Friends").size(16.0).color(p.strong).strong());
                    ui.add_space(10.0);
                    let pending = self.data.users.iter().filter(|u| u.pending_in || u.pending_out).count();
                    let tabs = [
                        (FriendsTab::Online, "Online".to_string()),
                        (FriendsTab::All, "All".to_string()),
                        (FriendsTab::Pending, if pending > 0 && self.data.settings.show_badges { format!("Pending ({pending})") } else { "Pending".to_string() }),
                    ];
                    for (t, label) in tabs {
                        let b = egui::Button::new(RichText::new(label).color(if tab == t { p.strong } else { p.muted }))
                            .fill(if tab == t { p.selected } else { Color32::TRANSPARENT })
                            .corner_radius(6.0);
                        if ui.add(b).clicked() {
                            tab = t;
                        }
                    }
                    let add = tab == FriendsTab::Add;
                    if pill_button(ui, "Add friend", if add { p.selected } else { p.accent }, if add { p.accent } else { p.on_accent }).clicked() {
                        tab = FriendsTab::Add;
                    }
                });
            });
        if tab != self.friends_tab {
            self.friends_tab = tab;
            self.add_friend_result = None;
        }

        let active: Vec<User> =
            self.data.users.iter().filter(|u| u.friend && u.status.shows_online() && !u.activity.is_empty()).cloned().collect();
        let mut open_dm: Option<Id> = None;
        egui::Panel::right("active_now")
            .exact_size(MEMBERS_W + 60.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::new().fill(p.main).inner_margin(16.0))
            .show(ui, |ui| {
                let full = ui.max_rect();
                ui.painter().line_segment(
                    [full.left_top() - vec2(16.0, 16.0), full.left_bottom() + vec2(-16.0, 16.0)],
                    Stroke::new(1.0, p.divider),
                );
                ui.label(RichText::new("Active now").size(18.0).color(p.strong).strong());
                ui.add_space(10.0);
                if active.is_empty() {
                    ui.label(RichText::new("It's quiet for now. When friends start doing something, it shows up here.").color(p.muted));
                }
                for u in &active {
                    let r = Frame::new().fill(p.side).corner_radius(10.0).inner_margin(12.0).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
                            paint_avatar(ui, r.center(), 32.0, &u.name, u.hue, Some(u.status), p.side);
                            ui.vertical(|ui| {
                                ui.label(RichText::new(&u.name).color(p.strong).strong());
                                ui.label(RichText::new(&u.activity).color(p.muted).size(12.5));
                            });
                        });
                    });
                    if ui.interact(r.response.rect, EId::new(("active", u.id)), Sense::click()).on_hover_text("Send a message").clicked() {
                        open_dm = Some(u.id);
                    }
                    ui.add_space(8.0);
                }
            });

        let mut accept: Option<Id> = None;
        let mut ignore: Option<Id> = None;
        let mut remove: Option<Id> = None;
        let mut send_request = false;
        let view_t = self.ease(self.view_changed_at, 0.22);
        let slide = ((1.0 - view_t) * 14.0) as i8;
        egui::CentralPanel::default()
            .frame(Frame::new().fill(p.main).inner_margin(Margin { left: 24, right: 24, top: 16 + slide, bottom: 16 }))
            .show(ui, |ui| {
            ui.multiply_opacity(view_t);
            if self.friends_tab == FriendsTab::Add {
                ui.label(RichText::new("Add a friend").size(18.0).color(p.strong).strong());
                ui.label(RichText::new("Type their name to send a friend request.").color(p.muted));
                ui.add_space(10.0);
                Frame::new().fill(p.raised).corner_radius(10.0).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let w = (ui.available_width() - 180.0).max(80.0);
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut self.friend_name).hint_text("Their name").frame(Frame::NONE).desired_width(w),
                        );
                        if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                            send_request = true;
                        }
                        if pill_button(ui, "Send friend request", p.accent, p.on_accent).clicked() {
                            send_request = true;
                        }
                    });
                });
                if let Some((ok, msg)) = &self.add_friend_result {
                    ui.add_space(6.0);
                    ui.label(RichText::new(msg).color(if *ok { theme::status_color(Status::Online) } else { p.danger }));
                }
                ui.add_space(14.0);
                ui.label(RichText::new("Try: Rowan, Odette or Kai.").color(p.faint).size(12.5));
                return;
            }
            let list: Vec<User> = self
                .data
                .users
                .iter()
                .filter(|u| u.id != ME)
                .filter(|u| match self.friends_tab {
                    FriendsTab::Online => u.friend && u.status.shows_online(),
                    FriendsTab::All => u.friend,
                    FriendsTab::Pending => u.pending_in || u.pending_out,
                    FriendsTab::Add => false,
                })
                .cloned()
                .collect();
            let title = match self.friends_tab {
                FriendsTab::Online => "Online",
                FriendsTab::All => "All friends",
                _ => "Pending",
            };
            section_label(ui, &format!("{title} — {}", list.len()), p);
            ui.add_space(6.0);
            if list.is_empty() {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(80.0, 80.0), Sense::hover());
                    paint_logo(ui, r, p.input);
                    ui.add_space(10.0);
                    let msg = if self.friends_tab == FriendsTab::Pending { "No pending requests. Add someone!" } else { "No one around right now." };
                    ui.label(RichText::new(msg).color(p.muted));
                });
            }
            egui::ScrollArea::vertical().id_salt("friends_scroll").show(ui, |ui| {
                for u in &list {
                    let top = ui.cursor().left_top();
                    ui.painter().line_segment([top, top + vec2(ui.available_width(), 0.0)], Stroke::new(1.0, p.hover));
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 60.0), Sense::click());
                    let hovered = ui.rect_contains_pointer(rect);
                    if hovered {
                        ui.painter().rect_filled(rect.shrink2(vec2(0.0, 2.0)), CornerRadius::same(8), p.hover);
                    }
                    let bg = if hovered { p.hover } else { p.main };
                    paint_avatar(ui, rect.left_center() + vec2(24.0, 0.0), 36.0, &u.name, u.hue, Some(u.status), bg);
                    ui.painter().text(rect.left_center() + vec2(52.0, -8.0), Align2::LEFT_CENTER, &u.name, FontId::proportional(15.0), p.strong);
                    let sub = if u.pending_in {
                        "Incoming friend request".to_string()
                    } else if u.pending_out {
                        "Outgoing friend request".to_string()
                    } else if u.activity.is_empty() {
                        u.status.label().to_string()
                    } else {
                        u.activity.clone()
                    };
                    ui.painter().text(rect.left_center() + vec2(52.0, 10.0), Align2::LEFT_CENTER, sub, FontId::proportional(12.5), p.muted);
                    let mut buttons: Vec<(&str, &str, u8)> = vec![];
                    if u.pending_in {
                        buttons.push(("✔", "Accept", 0));
                        buttons.push(("×", "Ignore", 1));
                    } else if u.pending_out {
                        buttons.push(("×", "Cancel", 1));
                    } else {
                        buttons.push(("×", "Remove friend", 3));
                        buttons.push(("💬", "Message", 2));
                    }
                    let mut on_button = false;
                    for (i, (g, tip, k)) in buttons.iter().enumerate() {
                        let c = rect.right_center() - vec2(24.0 + i as f32 * 44.0, 0.0);
                        let br = Rect::from_center_size(c, vec2(36.0, 36.0));
                        let r = ui.interact(br, EId::new(("fbtn", u.id, *k)), Sense::click());
                        ui.painter().circle_filled(c, 18.0, if r.hovered() { p.selected } else { p.side });
                        let col = match k {
                            0 if r.hovered() => theme::status_color(Status::Online),
                            1 | 3 if r.hovered() => p.danger,
                            _ if r.hovered() => p.strong,
                            _ => p.muted,
                        };
                        ui.painter().text(c, Align2::CENTER_CENTER, *g, FontId::proportional(15.0), col);
                        if r.hovered() {
                            on_button = true;
                        }
                        if r.on_hover_text(*tip).clicked() {
                            match k {
                                0 => accept = Some(u.id),
                                1 => ignore = Some(u.id),
                                2 => open_dm = Some(u.id),
                                _ => remove = Some(u.id),
                            }
                        }
                    }
                    if resp.clicked() && !on_button && !u.pending_in && !u.pending_out {
                        open_dm = Some(u.id);
                    }
                }
            });
        });
        if let Some(id) = accept {
            if let Some(u) = self.data.user_mut(id) {
                u.pending_in = false;
                u.friend = true;
            }
            self.dirty = true;
        }
        if let Some(id) = ignore {
            if let Some(u) = self.data.user_mut(id) {
                u.pending_in = false;
                u.pending_out = false;
            }
            self.dirty = true;
        }
        if let Some(id) = remove {
            if let Some(u) = self.data.user_mut(id) {
                u.friend = false;
            }
            self.dirty = true;
        }
        if send_request {
            let q = self.friend_name.trim().to_lowercase();
            let found =
                self.data.users.iter().find(|u| u.id != ME && u.name.to_lowercase() == q).map(|u| (u.id, u.friend, u.name.clone()));
            self.add_friend_result = Some(match found {
                None if q.is_empty() => (false, "Type a name first.".into()),
                None => (false, "Nobody by that name yet. Check the spelling?".into()),
                Some((_, true, n)) => (false, format!("You and {n} are already friends.")),
                Some((id, false, n)) => {
                    if let Some(u) = self.data.user_mut(id) {
                        u.pending_in = false;
                        u.pending_out = false;
                        u.friend = true; // sample people say yes right away
                    }
                    self.dirty = true;
                    (true, format!("{n} accepted your friend request!"))
                }
            });
            self.friend_name.clear();
        }
        if let Some(u) = open_dm {
            self.open_dm(u);
        }
    }

    // ------------------------------------------------------------ popups

    fn popups(&mut self, ctx: &egui::Context, p: &Palette) {
        // Pop-ups fade in and rise a little when they open.
        let pop_t = self.ease(self.popup_opened_at, 0.18);
        let rise = vec2(0.0, (1.0 - pop_t) * 10.0);
        let shadow = |a: u8| egui::Shadow { offset: [0, 6], blur: 20, spread: 0, color: Color32::from_black_alpha(a) };
        let pressed_outside = |rect: Rect| {
            ctx.input(|i| i.pointer.any_pressed() && i.pointer.press_origin().map(|o| !rect.contains(o)).unwrap_or(false))
        };
        let screen = ctx.content_rect();

        if let Some((target, pos)) = self.picker {
            let mut chosen: Option<String> = None;
            let pos = pos2(pos.x.clamp(8.0, (screen.right() - 316.0).max(8.0)), pos.y.clamp(8.0, (screen.bottom() - 250.0).max(8.0)));
            let resp = egui::Area::new(EId::new("emoji_picker")).order(egui::Order::Foreground).fixed_pos(pos + rise).show(ctx, |ui| {
                ui.multiply_opacity(pop_t);
                Frame::new()
                    .fill(p.raised)
                    .corner_radius(10.0)
                    .stroke(Stroke::new(1.0, p.divider))
                    .inner_margin(10.0)
                    .shadow(shadow(80))
                    .show(ui, |ui| {
                        section_label(ui, "Pick an emoji", p);
                        ui.add_space(4.0);
                        egui::Grid::new("emoji_grid").spacing(vec2(2.0, 2.0)).show(ui, |ui| {
                            for (i, e) in EMOJI.iter().enumerate() {
                                let (r, resp) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::click());
                                if resp.hovered() {
                                    ui.painter().rect_filled(r, CornerRadius::same(6), p.hover);
                                }
                                ui.painter().text(r.center(), Align2::CENTER_CENTER, *e, FontId::proportional(20.0), p.strong);
                                if resp.clicked() {
                                    chosen = Some(e.to_string());
                                }
                                if i % 8 == 7 {
                                    ui.end_row();
                                }
                            }
                        });
                    });
            });
            if let Some(e) = chosen {
                match target {
                    PickerFor::Composer => {
                        self.composer.push_str(&e);
                        self.focus_composer = true;
                    }
                    PickerFor::Reaction(id) => {
                        if let Some(t) = self.current_target() {
                            self.toggle_reaction(t, id, &e, ME);
                        }
                    }
                }
                self.picker = None;
            } else if pressed_outside(resp.response.rect) && !ctx.input(|i| i.pointer.press_origin()).map(|o| (o - pos).length() < 1.0).unwrap_or(false) {
                self.picker = None;
            }
        }

        if let Some((uid, pos)) = self.profile {
            let Some(u) = self.data.user(uid).cloned() else {
                self.profile = None;
                return;
            };
            let pos = pos2(pos.x.clamp(8.0, (screen.right() - 300.0).max(8.0)), pos.y.clamp(8.0, (screen.bottom() - 380.0).max(8.0)));
            let mut msg = false;
            let mut set_status: Option<Status> = None;
            let mut friend = false;
            let resp = egui::Area::new(EId::new("profile_card")).order(egui::Order::Foreground).fixed_pos(pos + rise).show(ctx, |ui| {
                ui.multiply_opacity(pop_t);
                Frame::new().fill(p.raised).corner_radius(12.0).stroke(Stroke::new(1.0, p.divider)).shadow(shadow(90)).show(ui, |ui| {
                    ui.set_width(284.0);
                    let (banner, _) = ui.allocate_exact_size(vec2(284.0, 64.0), Sense::hover());
                    ui.painter().rect_filled(banner, CornerRadius { nw: 12, ne: 12, sw: 0, se: 0 }, theme::hue(u.hue));
                    let ac = banner.left_bottom() + vec2(50.0, 4.0);
                    ui.painter().circle_filled(ac, 40.0, p.raised);
                    paint_avatar(ui, ac, 72.0, &u.name, u.hue, Some(u.status), p.raised);
                    ui.add_space(46.0);
                    Frame::new().inner_margin(Margin { left: 14, right: 14, top: 0, bottom: 14 }).show(ui, |ui| {
                        ui.label(RichText::new(&u.name).size(19.0).color(p.strong).strong());
                        ui.label(RichText::new(u.status.label()).color(p.muted).size(12.5));
                        ui.add_space(8.0);
                        Frame::new().fill(p.side).corner_radius(8.0).inner_margin(10.0).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            section_label(ui, "About", p);
                            ui.label(RichText::new(&u.about).color(p.text));
                            if !u.activity.is_empty() {
                                ui.add_space(6.0);
                                section_label(ui, "Right now", p);
                                ui.label(RichText::new(&u.activity).color(p.text));
                            }
                        });
                        ui.add_space(8.0);
                        if uid == ME {
                            section_label(ui, "Set status", p);
                            for s in Status::ALL {
                                let txt = RichText::new(format!("• {}", s.label())).color(theme::status_color(s)).size(13.0);
                                if ui.selectable_label(u.status == s, txt).clicked() {
                                    set_status = Some(s);
                                }
                            }
                        } else {
                            ui.horizontal(|ui| {
                                if pill_button(ui, &format!("Message {}", u.name), p.accent, p.on_accent).clicked() {
                                    msg = true;
                                }
                                if uid < 1_000_000 && !u.friend && ui.button("Add friend").clicked() {
                                    friend = true;
                                }
                            });
                        }
                    });
                });
            });
            if msg {
                self.profile = None;
                self.open_dm(uid);
            } else if let Some(s) = set_status {
                if let Some(me) = self.data.user_mut(ME) {
                    me.status = s;
                }
                self.dirty = true;
                self.last_hello = -100.0;
            } else if friend {
                if let Some(u) = self.data.user_mut(uid) {
                    u.friend = true;
                    u.pending_in = false;
                }
                self.dirty = true;
            } else if pressed_outside(resp.response.rect) && ctx.input(|i| i.pointer.press_start_time()).map(|t| t < ctx.input(|i| i.time) - 0.05).unwrap_or(true) {
                self.profile = None;
            }
        }
    }

    // ------------------------------------------------------------ dialogs

    fn dialogs(&mut self, ctx: &egui::Context, p: &Palette) {
        let Some(mut d) = self.dialog.take() else { return };
        let mut done = false;
        let mut cancel = false;
        let title = match &d {
            Dialog::NewCommunity { .. } => "Create your community",
            Dialog::NewChannel { .. } => "New channel",
            Dialog::RenameCommunity { .. } => "Rename community",
            Dialog::EditTopic { .. } => "Edit channel topic",
            Dialog::LeaveCommunity { .. } => "Leave community?",
        };
        let leave_name = match &d {
            Dialog::LeaveCommunity { community } => self.data.community(*community).map(|c| c.name.clone()).unwrap_or_default(),
            _ => String::new(),
        };
        let pop_t = self.ease(self.popup_opened_at, 0.2);
        egui::Modal::new(EId::new("dialog"))
            .backdrop_color(Color32::from_black_alpha((150.0 * pop_t) as u8))
            .frame(Frame::new().fill(p.main).corner_radius(12.0).inner_margin(20.0))
            .show(ctx, |ui| {
                ui.multiply_opacity(pop_t);
                ui.set_width(380.0);
                ui.label(RichText::new(title).size(20.0).color(p.strong).strong());
                ui.add_space(10.0);
                let field = |ui: &mut Ui, label: &str, s: &mut String, hint: &str| -> bool {
                    section_label(ui, label, p);
                    let r = ui.add(
                        egui::TextEdit::singleline(s).id_salt("dialog_field").hint_text(hint).desired_width(f32::INFINITY).background_color(p.raised),
                    );
                    if ui.memory(|m| m.focused().is_none()) {
                        r.request_focus();
                    }
                    r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))
                };
                match &mut d {
                    Dialog::NewCommunity { name, hue } => {
                        ui.label(RichText::new("A place for your people. You can change it later.").color(p.muted));
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
                            ui.painter().rect_filled(r, CornerRadius::same(18), theme::hue(*hue));
                            ui.painter().text(
                                r.center(),
                                Align2::CENTER_CENTER,
                                initials(if name.is_empty() { "New" } else { name }),
                                FontId::proportional(22.0),
                                Color32::WHITE,
                            );
                            ui.vertical(|ui| {
                                section_label(ui, "Colour", p);
                                ui.horizontal(|ui| {
                                    for i in 0..8u8 {
                                        let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
                                        ui.painter().circle_filled(r.center(), 10.0, theme::hue(i));
                                        if *hue == i {
                                            ui.painter().circle_stroke(r.center(), 11.5, Stroke::new(2.0, p.strong));
                                        }
                                        if resp.clicked() {
                                            *hue = i;
                                        }
                                    }
                                });
                            });
                        });
                        ui.add_space(10.0);
                        if field(ui, "Community name", name, "My community") {
                            done = true;
                        }
                    }
                    Dialog::NewChannel { name, kind, .. } => {
                        ui.horizontal(|ui| {
                            ui.selectable_value(kind, ChannelKind::Text, "# Text");
                            ui.selectable_value(kind, ChannelKind::Voice, "Voice");
                        });
                        ui.add_space(6.0);
                        if field(ui, "Channel name", name, "new-channel") {
                            done = true;
                        }
                    }
                    Dialog::RenameCommunity { name, .. } => {
                        if field(ui, "Community name", name, "") {
                            done = true;
                        }
                    }
                    Dialog::EditTopic { topic, .. } => {
                        if field(ui, "Topic", topic, "What's this channel about?") {
                            done = true;
                        }
                    }
                    Dialog::LeaveCommunity { .. } => {
                        ui.label(
                            RichText::new(format!("You'll leave {leave_name} and its messages will be removed from this computer."))
                                .color(p.text),
                        );
                    }
                }
                ui.add_space(16.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let leave = matches!(d, Dialog::LeaveCommunity { .. });
                    let label = match &d {
                        Dialog::LeaveCommunity { .. } => "Leave",
                        Dialog::NewCommunity { .. } | Dialog::NewChannel { .. } => "Create",
                        _ => "Save",
                    };
                    if pill_button(ui, label, if leave { p.danger } else { p.accent }, if leave { Color32::WHITE } else { p.on_accent }).clicked() {
                        done = true;
                    }
                    if ui.add(egui::Button::new(RichText::new("Cancel").color(p.muted)).frame(false)).clicked() {
                        cancel = true;
                    }
                });
            });
        if cancel {
            return;
        }
        if !done {
            self.dialog = Some(d);
            return;
        }
        match d {
            Dialog::NewCommunity { name, hue } => {
                let name = if name.trim().is_empty() { format!("{}'s place", self.data.me().name) } else { name.trim().to_string() };
                let id = self.data.new_id();
                let general = self.data.new_id();
                let voice = self.data.new_id();
                let welcome = self.data.new_id();
                let at = self.data.communities.iter().position(|c| c.id == HOME_NET_COMMUNITY).unwrap_or(self.data.communities.len());
                self.data.communities.insert(
                    at,
                    Community {
                        id,
                        name: name.clone(),
                        hue,
                        channels: vec![
                            Channel {
                                id: general,
                                name: "general".into(),
                                kind: ChannelKind::Text,
                                topic: "Say hello!".into(),
                                messages: vec![Message {
                                    id: welcome,
                                    author: ME,
                                    author_name: None,
                                    text: format!("Welcome to **{name}**! 🎉"),
                                    ts: now_secs(),
                                    edited: false,
                                    reactions: vec![],
                                    reply_to: None,
                                    pinned: true,
                                }],
                                unread: 0,
                                mentions: 0,
                                voice_people: vec![],
                            },
                            Channel {
                                id: voice,
                                name: "Hangout".into(),
                                kind: ChannelKind::Voice,
                                topic: String::new(),
                                messages: vec![],
                                unread: 0,
                                mentions: 0,
                                voice_people: vec![],
                            },
                        ],
                        members: vec![ME],
                        muted: false,
                    },
                );
                self.open_channel(id, general);
            }
            Dialog::NewChannel { community, name, kind } => {
                let id = self.data.new_id();
                let name = match kind {
                    ChannelKind::Text => {
                        let clean: String =
                            name.trim().to_lowercase().chars().map(|c| if c.is_whitespace() { '-' } else { c }).take(40).collect();
                        if clean.is_empty() { "new-channel".to_string() } else { clean }
                    }
                    ChannelKind::Voice => {
                        let n: String = name.trim().chars().take(40).collect();
                        if n.is_empty() { "Voice".to_string() } else { n }
                    }
                };
                if let Some(c) = self.data.community_mut(community) {
                    c.channels.push(Channel {
                        id,
                        name,
                        kind,
                        topic: String::new(),
                        messages: vec![],
                        unread: 0,
                        mentions: 0,
                        voice_people: vec![],
                    });
                }
                if kind == ChannelKind::Text {
                    self.open_channel(community, id);
                }
            }
            Dialog::RenameCommunity { community, name } => {
                if let Some(c) = self.data.community_mut(community) {
                    if !name.trim().is_empty() {
                        c.name = name.trim().to_string();
                    }
                }
            }
            Dialog::EditTopic { community, channel, topic } => {
                if let Some(x) = self.data.community_mut(community).and_then(|c| c.channels.iter_mut().find(|x| x.id == channel)) {
                    x.topic = topic.trim().to_string();
                }
            }
            Dialog::LeaveCommunity { community } => {
                if self.voice.map(|v| v.0) == Some(community) {
                    self.leave_voice();
                }
                self.data.communities.retain(|c| c.id != community);
                self.open_home();
            }
        }
        self.dirty = true;
    }

    // ------------------------------------------------------------ settings

    fn settings(&mut self, ui: &mut Ui, p: &Palette) {
        let mut page = self.settings_page;
        let mut close = false;
        egui::Panel::left("settings_nav")
            .exact_size(260.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::new().fill(p.side).inner_margin(Margin { left: 24, right: 12, top: 48, bottom: 12 }))
            .show(ui, |ui| {
                section_label(ui, "Your settings", p);
                ui.add_space(4.0);
                for (pg, label) in [
                    (SettingsPage::Account, "My account"),
                    (SettingsPage::Appearance, "Appearance"),
                    (SettingsPage::Chat, "Chat"),
                    (SettingsPage::Voice, "Voice & sound"),
                    (SettingsPage::Notifications, "Notifications"),
                    (SettingsPage::Accessibility, "Accessibility & window"),
                    (SettingsPage::Privacy, "Privacy"),
                    (SettingsPage::Shortcuts, "Keyboard shortcuts"),
                    (SettingsPage::Share, "Invite friends"),
                    (SettingsPage::Network, "Home network"),
                    (SettingsPage::Updates, "Updates"),
                    (SettingsPage::Data, "Data & reset"),
                ] {
                    let sel = page == pg;
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
                    if sel || resp.hovered() {
                        ui.painter().rect_filled(r, CornerRadius::same(6), if sel { p.selected } else { p.hover });
                    }
                    ui.painter().text(r.left_center() + vec2(10.0, 0.0), Align2::LEFT_CENTER, label, FontId::proportional(15.0), if sel { p.strong } else { p.muted });
                    if resp.clicked() {
                        page = pg;
                    }
                }
                ui.add_space(16.0);
                ui.separator();
                ui.label(RichText::new(format!("Hearth {} · your own chat app", update::CURRENT)).size(11.5).color(p.faint));
            });
        self.settings_page = page;
        // Voice & sound page state
        if page != SettingsPage::Voice {
            self.mic_test = false;
            self.recording_ptt = false;
        } else if self.device_lists.is_none() {
            self.device_lists = Some((audio::device_names(true), audio::device_names(false)));
        }
        if self.recording_ptt {
            let pressed = ui.ctx().input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Key { key, pressed: true, .. } => Some(*key),
                    _ => None,
                })
            });
            if let Some(k) = pressed {
                if k != Key::Escape {
                    self.data.settings.ptt_key = k.name().to_string();
                }
                self.recording_ptt = false;
                self.dirty = true;
            }
        }
        let (inputs, outputs) = self.device_lists.clone().unwrap_or_default();
        let mic_db = self.audio.mic_db();
        let mic_live = self.audio.transmitting();
        let audio_error = self.audio.error();
        let glitches = self.audio.glitches();
        let update_state = self.updater.state();
        let update_failed = self.update_failed.clone();
        let mut check_updates = false;
        let mut download_update = false;
        let mut restart_update = false;
        let mut toggle_mic_test = false;
        let mut record_key = false;
        let mut refresh_devices = false;
        let mut preview: Option<Sfx> = None;
        let mic_test_on = self.mic_test;
        let recording = self.recording_ptt;
        let mut reset = false;
        let mut restart_lan = false;
        let lan_status: (String, Color32) = match (&self.lan, &self.lan_error) {
            (Some(l), _) => (format!("• Connected on port {} · {} nearby", l.port, self.lan_seen.len()), theme::status_color(Status::Online)),
            (None, Some(e)) => (e.clone(), p.danger),
            _ => ("Off".into(), p.faint),
        };
        let save_path = self.save_path.as_ref().map(|p| p.display().to_string());
        let view_t = self.ease(self.view_changed_at, 0.25);
        let slide = ((1.0 - view_t) * 28.0) as i8;
        egui::CentralPanel::default()
            .frame(Frame::new().fill(p.main).inner_margin(Margin { left: 40 + slide, right: 40, top: 40, bottom: 20 }))
            .show(ui, |ui| {
                ui.multiply_opacity(view_t);
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    let (r, resp) = ui.allocate_exact_size(vec2(40.0, 52.0), Sense::click());
                    let c = r.center_top() + vec2(0.0, 18.0);
                    let col = if resp.hovered() { p.strong } else { p.muted };
                    ui.painter().circle_stroke(c, 17.0, Stroke::new(2.0, col));
                    ui.painter().text(c, Align2::CENTER_CENTER, "×", FontId::proportional(16.0), col);
                    ui.painter().text(c + vec2(0.0, 28.0), Align2::CENTER_CENTER, "ESC", FontId::proportional(11.0), p.muted);
                    if resp.on_hover_text("Close settings").clicked() {
                        close = true;
                    }
                    ui.with_layout(Layout::top_down(Align::Min), |ui| {
                        egui::ScrollArea::vertical().id_salt("settings_scroll").show(ui, |ui| {
                            ui.set_max_width(660.0);
                            let s = &mut self.data.settings;
                            match page {
                                SettingsPage::Account => {
                                    ui.label(RichText::new("My account").size(22.0).color(p.strong).strong());
                                    ui.add_space(14.0);
                                    if let Some(me) = self.data.users.iter_mut().find(|u| u.id == ME) {
                                        Frame::new().fill(p.side).corner_radius(12.0).inner_margin(16.0).show(ui, |ui| {
                                            ui.set_width(ui.available_width());
                                            ui.horizontal(|ui| {
                                                let (r, _) = ui.allocate_exact_size(vec2(80.0, 80.0), Sense::hover());
                                                paint_avatar(ui, r.center(), 76.0, &me.name, me.hue, Some(me.status), p.side);
                                                ui.add_space(12.0);
                                                ui.vertical(|ui| {
                                                    section_label(ui, "Display name", p);
                                                    let mut n = me.name.clone();
                                                    let r = ui.add(egui::TextEdit::singleline(&mut n).char_limit(32).desired_width(260.0).background_color(p.raised));
                                                    if r.changed() && !n.trim().is_empty() {
                                                        me.name = n.trim_start().to_string();
                                                    }
                                                    ui.add_space(6.0);
                                                    section_label(ui, "Avatar colour", p);
                                                    ui.horizontal(|ui| {
                                                        for i in 0..8u8 {
                                                            let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
                                                            ui.painter().circle_filled(r.center(), 10.0, theme::hue(i));
                                                            if me.hue == i {
                                                                ui.painter().circle_stroke(r.center(), 11.5, Stroke::new(2.0, p.strong));
                                                            }
                                                            if resp.clicked() {
                                                                me.hue = i;
                                                            }
                                                        }
                                                    });
                                                });
                                            });
                                            ui.add_space(12.0);
                                            section_label(ui, "About me", p);
                                            ui.add(egui::TextEdit::multiline(&mut me.about).desired_rows(3).desired_width(f32::INFINITY).char_limit(190).background_color(p.raised));
                                            ui.add_space(8.0);
                                            section_label(ui, "What I'm up to", p);
                                            ui.add(egui::TextEdit::singleline(&mut me.activity).hint_text("e.g. Listening to music").desired_width(f32::INFINITY).char_limit(60).background_color(p.raised));
                                            ui.add_space(8.0);
                                            section_label(ui, "Status", p);
                                            ui.horizontal(|ui| {
                                                for st in Status::ALL {
                                                    let txt = RichText::new(format!("• {}", st.label())).color(theme::status_color(st));
                                                    if ui.selectable_label(me.status == st, txt).clicked() {
                                                        me.status = st;
                                                    }
                                                }
                                            });
                                        });
                                    }
                                }
                                SettingsPage::Appearance => {
                                    ui.label(RichText::new("Appearance").size(22.0).color(p.strong).strong());
                                    ui.add_space(14.0);
                                    section_label(ui, "Theme", p);
                                    ui.add_space(4.0);
                                    ui.horizontal(|ui| {
                                        for (t, label) in [(Theme::Ember, "Ember"), (Theme::Midnight, "Midnight"), (Theme::Daylight, "Daylight")] {
                                            let tp = theme::palette(t, s.accent);
                                            let (r, resp) = ui.allocate_exact_size(vec2(120.0, 100.0), Sense::click());
                                            let card = Rect::from_min_size(r.min, vec2(120.0, 78.0));
                                            ui.painter().rect_filled(card, CornerRadius::same(10), tp.main);
                                            ui.painter().rect_filled(Rect::from_min_size(card.min, vec2(22.0, card.height())), CornerRadius { nw: 10, sw: 10, ne: 0, se: 0 }, tp.rail);
                                            ui.painter().rect_filled(Rect::from_min_size(card.min + vec2(22.0, 0.0), vec2(30.0, card.height())), CornerRadius::ZERO, tp.side);
                                            ui.painter().rect_filled(Rect::from_min_size(card.min + vec2(60.0, 54.0), vec2(52.0, 14.0)), CornerRadius::same(4), tp.input);
                                            ui.painter().circle_filled(card.min + vec2(11.0, 14.0), 6.0, tp.accent);
                                            ui.painter().text(card.center_bottom() + vec2(0.0, 12.0), Align2::CENTER_CENTER, label, FontId::proportional(13.0), p.text);
                                            if s.theme == t {
                                                ui.painter().rect_stroke(card, CornerRadius::same(10), Stroke::new(2.5, p.accent), StrokeKind::Outside);
                                            }
                                            if resp.clicked() {
                                                s.theme = t;
                                            }
                                            ui.add_space(8.0);
                                        }
                                    });
                                    ui.add_space(14.0);
                                    section_label(ui, "Accent colour", p);
                                    ui.horizontal(|ui| {
                                        for (i, (name, col)) in theme::ACCENTS.iter().enumerate() {
                                            let (r, resp) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::click());
                                            ui.painter().circle_filled(r.center(), 16.0, *col);
                                            if s.accent as usize == i {
                                                ui.painter().circle_stroke(r.center(), 19.0, Stroke::new(2.5, p.strong));
                                            }
                                            if resp.on_hover_text(*name).clicked() {
                                                s.accent = i as u8;
                                            }
                                        }
                                    });
                                    ui.add_space(14.0);
                                    section_label(ui, "Text size", p);
                                    ui.horizontal(|ui| {
                                        for pct in [90, 100, 110, 125] {
                                            let sel = (s.text_scale * 100.0).round() as i32 == pct;
                                            if ui.selectable_label(sel, format!("{pct}%")).clicked() {
                                                s.text_scale = pct as f32 / 100.0;
                                            }
                                        }
                                    });
                                    ui.add_space(10.0);
                                    ui.checkbox(&mut s.compact, "Compact messages (no pictures, one line per message)");
                                    ui.checkbox(&mut s.square_icons, "Squarer community icons");
                                }
                                SettingsPage::Chat => {
                                    ui.label(RichText::new("Chat").size(22.0).color(p.strong).strong());
                                    ui.add_space(14.0);
                                    section_label(ui, "Messages", p);
                                    ui.checkbox(&mut s.show_timestamps, "Show times next to messages");
                                    ui.checkbox(&mut s.clock_24h, "Use a 24-hour clock (off: 3:45 PM)");
                                    ui.checkbox(&mut s.day_dividers, "Show a line between days");
                                    ui.checkbox(&mut s.group_messages, "Group messages sent close together by the same person");
                                    ui.checkbox(&mut s.show_members, "Show the member list beside channels");
                                    ui.add_space(10.0);
                                    section_label(ui, "Typing", p);
                                    ui.radio_value(&mut s.enter_sends, true, "Enter sends, Shift+Enter starts a new line");
                                    ui.radio_value(&mut s.enter_sends, false, "Ctrl+Enter sends, Enter starts a new line");
                                    ui.checkbox(&mut s.show_typing, "Show when someone is typing");
                                    ui.add_space(10.0);
                                    section_label(ui, "Sample people", p);
                                    ui.checkbox(&mut s.sample_replies, "Sample people reply to my messages");
                                    ui.add_enabled_ui(s.sample_replies, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label("How fast they answer");
                                            for (v, label) in [(0u8, "Relaxed"), (1, "Normal"), (2, "Quick")] {
                                                ui.selectable_value(&mut s.reply_speed, v, label);
                                            }
                                        });
                                    });
                                    ui.add_space(14.0);
                                    Frame::new().fill(p.side).corner_radius(10.0).inner_margin(14.0).show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        section_label(ui, "Formatting", p);
                                        ui.label("**strong**  ·  *italic*  ·  `code`  ·  @name to mention someone");
                                    });
                                }
                                SettingsPage::Voice => {
                                    ui.label(RichText::new("Voice & sound").size(22.0).color(p.strong).strong());
                                    ui.add_space(6.0);
                                    ui.label(RichText::new("You can talk with friends in the Home Network community's Lounge voice channel when you're on the same Wi-Fi.").color(p.muted));
                                    if let Some(e) = &audio_error {
                                        ui.add_space(6.0);
                                        ui.label(RichText::new(format!("Sound problem: {e}")).color(p.danger));
                                    }
                                    if glitches > 0 {
                                        ui.add_space(4.0);
                                        ui.label(
                                            RichText::new(format!(
                                                "Short sound glitches since Hearth started: {glitches}. A few are normal (for example when a device starts); many in a row mean the computer is very busy."
                                            ))
                                            .size(12.0)
                                            .color(p.faint),
                                        );
                                    }
                                    ui.add_space(12.0);

                                    // Devices
                                    let device_combo = |ui: &mut Ui, id: &str, label: &str, value: &mut Option<String>, names: &[String]| {
                                        section_label(ui, label, p);
                                        let shown = value.clone().unwrap_or_else(|| "Computer default".into());
                                        egui::ComboBox::from_id_salt(id).width(380.0).selected_text(shown).show_ui(ui, |ui| {
                                            ui.selectable_value(value, None, "Computer default");
                                            for n in names {
                                                ui.selectable_value(value, Some(n.clone()), n);
                                            }
                                        });
                                    };
                                    ui.horizontal(|ui| {
                                        ui.vertical(|ui| device_combo(ui, "in_dev", "Microphone", &mut s.input_device, &inputs));
                                        ui.add_space(12.0);
                                        ui.vertical(|ui| device_combo(ui, "out_dev", "Speakers or headphones", &mut s.output_device, &outputs));
                                    });
                                    if ui.small_button("Refresh device list").clicked() {
                                        refresh_devices = true;
                                    }
                                    ui.add_space(10.0);
                                    ui.horizontal(|ui| {
                                        ui.vertical(|ui| {
                                            section_label(ui, "Microphone volume", p);
                                            let mut v = s.input_volume * 100.0;
                                            if ui.add(egui::Slider::new(&mut v, 0.0..=200.0).suffix("%").step_by(5.0)).changed() {
                                                s.input_volume = v / 100.0;
                                            }
                                        });
                                        ui.add_space(24.0);
                                        ui.vertical(|ui| {
                                            section_label(ui, "Voices volume", p);
                                            let mut v = s.output_volume * 100.0;
                                            if ui.add(egui::Slider::new(&mut v, 0.0..=200.0).suffix("%").step_by(5.0)).changed() {
                                                s.output_volume = v / 100.0;
                                            }
                                        });
                                    });

                                    // Mic test + meter
                                    ui.add_space(12.0);
                                    section_label(ui, "Mic test", p);
                                    ui.horizontal(|ui| {
                                        let (fill, fg) = if mic_test_on { (p.danger, Color32::WHITE) } else { (p.accent, p.on_accent) };
                                        if pill_button(ui, if mic_test_on { "Stop test" } else { "Test my mic" }, fill, fg).clicked() {
                                            toggle_mic_test = true;
                                        }
                                        ui.add_space(8.0);
                                        let (bar, _) = ui.allocate_exact_size(vec2(320.0, 14.0), Sense::hover());
                                        ui.painter().rect_filled(bar, CornerRadius::same(7), p.input);
                                        let frac = ((mic_db + 60.0) / 60.0).clamp(0.0, 1.0);
                                        let lit = Rect::from_min_size(bar.min, vec2(bar.width() * frac, bar.height()));
                                        let col = if mic_live { theme::status_color(Status::Online) } else { p.muted };
                                        ui.painter().rect_filled(lit, CornerRadius::same(7), col);
                                        if !s.push_to_talk {
                                            let tx = bar.left() + bar.width() * ((s.mic_sensitivity_db + 60.0) / 60.0).clamp(0.0, 1.0);
                                            ui.painter().line_segment([pos2(tx, bar.top() - 4.0), pos2(tx, bar.bottom() + 4.0)], Stroke::new(2.0, p.strong));
                                        }
                                    });
                                    ui.label(
                                        RichText::new(if mic_test_on {
                                            "Talk: you'll hear yourself (headphones help). The bar turns green when your mic would be sending."
                                        } else {
                                            "Press Test to hear yourself and check the level. The bar also moves while you're in a voice channel."
                                        })
                                        .size(12.0)
                                        .color(p.faint),
                                    );

                                    // Input mode
                                    ui.add_space(12.0);
                                    section_label(ui, "When your mic sends", p);
                                    ui.radio_value(&mut s.push_to_talk, false, "Voice activity: when you talk loud enough");
                                    ui.radio_value(&mut s.push_to_talk, true, "Push to talk: only while you hold a key");
                                    if s.push_to_talk {
                                        ui.horizontal(|ui| {
                                            ui.label("Key:");
                                            let label = if recording { "Press a key… (Esc cancels)".to_string() } else { s.ptt_key.clone() };
                                            if ui.add(egui::Button::new(RichText::new(label).strong()).fill(p.input)).clicked() {
                                                record_key = true;
                                            }
                                            ui.label(RichText::new("Works while Hearth is the active window.").size(12.0).color(p.faint));
                                        });
                                    } else {
                                        ui.horizontal(|ui| {
                                            ui.label("Sensitivity");
                                            ui.add(egui::Slider::new(&mut s.mic_sensitivity_db, -70.0..=-10.0).suffix(" dB").step_by(1.0))
                                                .on_hover_text("The mic opens when the level bar passes the white line. Lower = picks up quieter voices (and more background noise).");
                                        });
                                    }
                                    ui.checkbox(&mut s.noise_suppression, "Noise suppression: remove background noise like fans, typing and hum")
                                        .on_hover_text("Uses RNNoise, an open-source noise suppressor (here its Rust version, nnnoiseless).");
                                    ui.checkbox(&mut s.echo_reduction, "Reduce echo: don't send while friends are talking (use it without headphones)");

                                    // Per-friend volumes
                                    if !s.user_volumes.is_empty() {
                                        ui.add_space(12.0);
                                        section_label(ui, "Friends' volumes", p);
                                        let mut remove: Option<String> = None;
                                        let mut names: Vec<String> = s.user_volumes.keys().cloned().collect();
                                        names.sort();
                                        for n in names {
                                            ui.horizontal(|ui| {
                                                ui.add_sized([140.0, 18.0], egui::Label::new(&n).truncate());
                                                if let Some(v) = s.user_volumes.get_mut(&n) {
                                                    let mut pct = *v * 100.0;
                                                    if ui.add(egui::Slider::new(&mut pct, 0.0..=200.0).suffix("%").step_by(5.0)).changed() {
                                                        *v = pct / 100.0;
                                                    }
                                                }
                                                if ui.small_button("Reset").clicked() {
                                                    remove = Some(n.clone());
                                                }
                                            });
                                        }
                                        if let Some(n) = remove {
                                            s.user_volumes.remove(&n);
                                        }
                                    }
                                    ui.label(RichText::new("Tip: right-click a friend on the voice screen to change just their volume.").size(12.0).color(p.faint));

                                    // Interface sounds
                                    ui.add_space(14.0);
                                    section_label(ui, "Interface sounds", p);
                                    ui.checkbox(&mut s.ui_sounds, "Play sounds");
                                    ui.add_enabled_ui(s.ui_sounds, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label("Volume");
                                            let mut v = s.ui_volume * 100.0;
                                            if ui.add(egui::Slider::new(&mut v, 0.0..=100.0).suffix("%").step_by(5.0)).changed() {
                                                s.ui_volume = v / 100.0;
                                            }
                                        });
                                        let rows: [(&str, Sfx, u8); 4] = [
                                            ("New messages", Sfx::Message, 0),
                                            ("Someone @mentions me", Sfx::Mention, 1),
                                            ("People joining and leaving voice", Sfx::PeerJoin, 2),
                                            ("Mute, deafen and going live", Sfx::Mute, 3),
                                        ];
                                        for (label, sound, k) in rows {
                                            ui.horizontal(|ui| {
                                                let flag = match k {
                                                    0 => &mut s.sound_messages,
                                                    1 => &mut s.sound_mentions,
                                                    2 => &mut s.sound_voice,
                                                    _ => &mut s.sound_controls,
                                                };
                                                ui.checkbox(flag, label);
                                                if ui.small_button("▶ Play").on_hover_text("Hear it").clicked() {
                                                    preview = Some(sound);
                                                }
                                            });
                                        }
                                    });

                                    // Troubleshooting
                                    ui.add_space(14.0);
                                    Frame::new().fill(p.side).corner_radius(10.0).inner_margin(14.0).show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        section_label(ui, "Can't hear a friend?", p);
                                        for line in [
                                            "1. Both of you need this version of Hearth (send them the new zip).",
                                            "2. Both join the Lounge voice channel in the Home Network community, on the same Wi-Fi.",
                                            "3. Allow Hearth through Windows Firewall on private networks (both computers).",
                                            "4. Check you aren't deafened, and the right speakers are picked above.",
                                            "5. Ask them to run the mic test: their bar should turn green when they talk.",
                                            "6. Voices only work in Home Network; the sample people never talk.",
                                        ] {
                                            ui.label(RichText::new(line).color(p.text));
                                        }
                                    });
                                }
                                SettingsPage::Notifications => {
                                    ui.label(RichText::new("Notifications").size(22.0).color(p.strong).strong());
                                    ui.add_space(14.0);
                                    ui.checkbox(&mut s.flash_on_message, "Flash the taskbar when a message arrives while Hearth is in the background");
                                    ui.add_enabled_ui(s.flash_on_message, |ui| {
                                        ui.checkbox(&mut s.mentions_only, "In communities, only when someone @mentions me (direct messages always count)");
                                    });
                                    ui.checkbox(&mut s.show_badges, "Show red number badges for unread messages and mentions");
                                    ui.add_space(10.0);
                                    ui.label(RichText::new("Muted communities never flash. Mute one from its name menu at the top of the channel list.").size(12.5).color(p.faint));
                                }
                                SettingsPage::Accessibility => {
                                    ui.label(RichText::new("Accessibility & window").size(22.0).color(p.strong).strong());
                                    ui.add_space(14.0);
                                    section_label(ui, "Easier to see", p);
                                    ui.checkbox(&mut s.high_contrast, "Stronger text contrast");
                                    ui.checkbox(&mut s.reduced_motion, "Reduce motion (no pulsing, speaking rings or moving dots)");
                                    ui.label(RichText::new("Text size is under Appearance.").size(12.5).color(p.faint));
                                    ui.add_space(10.0);
                                    section_label(ui, "Window", p);
                                    ui.checkbox(&mut s.always_on_top, "Keep Hearth on top of other windows");
                                    ui.checkbox(&mut s.skip_splash, "Skip the start-up screen");
                                }
                                SettingsPage::Privacy => {
                                    ui.label(RichText::new("Privacy").size(22.0).color(p.strong).strong());
                                    ui.add_space(6.0);
                                    ui.label(RichText::new("These choose what other people on your home network can see about you.").color(p.muted));
                                    ui.add_space(12.0);
                                    if ui.checkbox(&mut s.share_status_lan, "Show my status (off: I appear offline to them)").changed() {
                                        self.last_hello = -100.0;
                                    }
                                    if ui.checkbox(&mut s.share_activity, "Show what I'm up to").changed() {
                                        self.last_hello = -100.0;
                                    }
                                }
                                SettingsPage::Share => {
                                    ui.label(RichText::new("Invite friends").size(22.0).color(p.strong).strong());
                                    ui.add_space(6.0);
                                    ui.label(RichText::new("Hearth is one program file. Send it to a friend and you can chat in the Home Network community whenever you're on the same Wi-Fi or home network.").color(p.muted));
                                    ui.add_space(12.0);
                                    let steps: [(&str, &str); 5] = [
                                        ("Find the file", "Press the button below. It opens the folder with Hearth's program selected, or use the Hearth.zip you were given."),
                                        ("Send it", "Put the zip in a shared drive (Google Drive, OneDrive, Dropbox), a file-transfer site, or a USB stick, and send your friend the link. Email and many chat apps block programs, even inside a zip."),
                                        ("Your friend opens it", "Unzip, then double-click Hearth.exe. If Windows says it protected the PC, click \"More info\" then \"Run anyway\" (Hearth isn't signed by a company)."),
                                        ("Allow the network", "The first time, Windows Firewall may ask about Hearth. Tick \"Private networks\" and allow it."),
                                        ("Chat", "Both of you open the Home Network community (HN on the left). Each person picks their name under My account."),
                                    ];
                                    for (i, (title, text)) in steps.iter().enumerate() {
                                        Frame::new().fill(p.side).corner_radius(10.0).inner_margin(12.0).show(ui, |ui| {
                                            ui.set_width(ui.available_width());
                                            ui.horizontal(|ui| {
                                                let (r, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
                                                ui.painter().circle_filled(r.center(), 14.0, p.accent);
                                                ui.painter().text(r.center(), Align2::CENTER_CENTER, (i + 1).to_string(), FontId::proportional(14.0), p.on_accent);
                                                ui.add_space(6.0);
                                                ui.vertical(|ui| {
                                                    ui.label(RichText::new(*title).color(p.strong).strong());
                                                    ui.add(egui::Label::new(RichText::new(*text).color(p.text)).wrap());
                                                });
                                            });
                                        });
                                        ui.add_space(6.0);
                                    }
                                    ui.add_space(8.0);
                                    if pill_button(ui, "Show Hearth's program file", p.accent, p.on_accent).clicked() {
                                        if let Ok(exe) = std::env::current_exe() {
                                            #[cfg(target_os = "windows")]
                                            let _ = std::process::Command::new("explorer").arg(format!("/select,{}", exe.display())).spawn();
                                            #[cfg(not(target_os = "windows"))]
                                            if let Some(dir) = exe.parent() {
                                                let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
                                            }
                                        }
                                    }
                                    ui.add_space(10.0);
                                    ui.label(RichText::new(&lan_status.0).color(lan_status.1));
                                    ui.label(RichText::new("Good to know: chatting only works on the same network. Friends somewhere else can still use Hearth, but you won't see each other's messages.").size(12.5).color(p.faint));
                                }
                                SettingsPage::Shortcuts => {
                                    ui.label(RichText::new("Keyboard shortcuts").size(22.0).color(p.strong).strong());
                                    ui.add_space(14.0);
                                    let send = if s.enter_sends { ("Enter", "Shift+Enter") } else { ("Ctrl+Enter", "Enter") };
                                    egui::Grid::new("shortcuts").num_columns(2).spacing(vec2(30.0, 10.0)).striped(true).show(ui, |ui| {
                                        for (k, what) in [
                                            (send.0, "Send the message"),
                                            (send.1, "Start a new line"),
                                            ("↑ (empty box)", "Edit your last message"),
                                            ("Esc", "Cancel a reply or edit, close pop-ups and settings"),
                                            ("Enter (in Search)", "Search this community or conversation"),
                                        ] {
                                            ui.label(RichText::new(k).color(p.strong).strong());
                                            ui.label(RichText::new(what).color(p.text));
                                            ui.end_row();
                                        }
                                    });
                                }
                                SettingsPage::Network => {
                                    ui.label(RichText::new("Home network").size(22.0).color(p.strong).strong());
                                    ui.add_space(6.0);
                                    ui.label(RichText::new("Chat with people running Hearth on the same Wi-Fi or home network. Messages in the Home Network community stay inside your network and never go to the internet.").color(p.muted));
                                    ui.add_space(12.0);
                                    if ui.checkbox(&mut s.lan_enabled, "Share the Home Network community").changed() {
                                        restart_lan = true;
                                    }
                                    ui.horizontal(|ui| {
                                        ui.label("Port");
                                        let mut port = s.lan_port as u32;
                                        if ui.add(egui::DragValue::new(&mut port).range(1024..=65535)).changed() {
                                            s.lan_port = port as u16;
                                        }
                                        if ui.button("Apply").clicked() {
                                            restart_lan = true;
                                        }
                                    });
                                    ui.add_space(8.0);
                                    ui.label(RichText::new(&lan_status.0).color(lan_status.1));
                                    ui.add_space(8.0);
                                    ui.label(RichText::new("Your firewall may ask about Hearth the first time; allow it on private networks.").size(12.5).color(p.faint));
                                }
                                SettingsPage::Updates => {
                                    ui.label(RichText::new("Updates").size(22.0).color(p.strong).strong());
                                    ui.add_space(6.0);
                                    ui.label(RichText::new(format!("You have Hearth {}.", update::CURRENT)).color(p.text));
                                    ui.label(RichText::new(format!("New versions come from github.com/{}/releases.", update::REPO)).color(p.muted));
                                    ui.add_space(12.0);
                                    ui.checkbox(&mut s.auto_update_check, "Check for new versions automatically");
                                    ui.add_enabled_ui(s.auto_update_check, |ui| {
                                        ui.checkbox(&mut s.auto_update_install, "Download them automatically and install when Hearth closes");
                                    });
                                    ui.add_space(12.0);
                                    let (status, col) = match &update_state {
                                        update::State::Idle => ("Not checked yet.".to_string(), p.muted),
                                        update::State::Checking => ("Checking…".to_string(), p.muted),
                                        update::State::UpToDate => ("You have the newest version.".to_string(), theme::status_color(Status::Online)),
                                        update::State::NoReleases => ("No versions have been published yet.".to_string(), p.muted),
                                        update::State::Available { version, .. } => (format!("Hearth {version} is available."), p.accent),
                                        update::State::Downloading { version, done, total } => (
                                            format!("Downloading Hearth {version}… {:.1} of {:.1} MB", *done as f64 / 1e6, *total as f64 / 1e6),
                                            p.accent,
                                        ),
                                        update::State::Ready { version, .. } => (format!("Hearth {version} is downloaded and ready."), theme::status_color(Status::Online)),
                                        update::State::Failed(e) => (e.clone(), p.danger),
                                    };
                                    ui.label(RichText::new(status).color(col).strong());
                                    if let update::State::Downloading { done, total, .. } = &update_state {
                                        let frac = if *total > 0 { *done as f32 / *total as f32 } else { 0.0 };
                                        ui.add(egui::ProgressBar::new(frac).desired_width(360.0));
                                    }
                                    if let Some(e) = &update_failed {
                                        ui.label(RichText::new(format!("Last install problem: {e}")).size(12.0).color(p.danger));
                                    }
                                    let notes = match &update_state {
                                        update::State::Available { notes, .. } | update::State::Ready { notes, .. } => notes.clone(),
                                        _ => String::new(),
                                    };
                                    if !notes.trim().is_empty() {
                                        ui.add_space(8.0);
                                        Frame::new().fill(p.side).corner_radius(10.0).inner_margin(12.0).show(ui, |ui| {
                                            ui.set_width(ui.available_width());
                                            section_label(ui, "What's new", p);
                                            ui.label(RichText::new(notes).color(p.text));
                                        });
                                    }
                                    ui.add_space(12.0);
                                    ui.horizontal(|ui| {
                                        let busy = matches!(update_state, update::State::Checking | update::State::Downloading { .. });
                                        match &update_state {
                                            update::State::Ready { .. } => {
                                                if pill_button(ui, "Restart to update", p.accent, p.on_accent).clicked() {
                                                    restart_update = true;
                                                }
                                            }
                                            update::State::Available { .. } => {
                                                if pill_button(ui, "Download", p.accent, p.on_accent).clicked() {
                                                    download_update = true;
                                                }
                                            }
                                            _ => {
                                                if ui.add_enabled(!busy, egui::Button::new("Check now")).clicked() {
                                                    check_updates = true;
                                                }
                                            }
                                        }
                                    });
                                    ui.add_space(14.0);
                                    Frame::new().fill(p.side).corner_radius(10.0).inner_margin(14.0).show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        section_label(ui, "Publishing a new version", p);
                                        let steps = [
                                            format!("1. Raise the version number when you build Hearth (it's {} now).", update::CURRENT),
                                            "2. On GitHub, create a release whose tag is that version, like v0.2.1.".to_string(),
                                            "3. Attach Hearth.exe (or the Hearth zip) to the release.".to_string(),
                                            "4. Optional: attach Hearth.exe.sha256 so downloads are checked.".to_string(),
                                            "Everyone with an older version gets it the next time Hearth checks.".to_string(),
                                        ];
                                        for line in steps {
                                            ui.label(RichText::new(line).color(p.text));
                                        }
                                    });
                                }
                                SettingsPage::Data => {
                                    ui.label(RichText::new("Data & reset").size(22.0).color(p.strong).strong());
                                    ui.add_space(8.0);
                                    ui.label(RichText::new("Everything is saved on this computer.").color(p.muted));
                                    if let Some(path) = &save_path {
                                        ui.label(RichText::new(path).size(12.0).color(p.faint));
                                    }
                                    ui.add_space(14.0);
                                    if pill_button(ui, "Reset everything to a fresh start", p.danger, Color32::WHITE).clicked() {
                                        reset = true;
                                    }
                                }
                            }
                        });
                    });
                });
            });
        if close {
            self.settings_open = false;
            self.focus_composer = true;
            self.mic_test = false;
            self.recording_ptt = false;
        }
        if toggle_mic_test {
            self.mic_test = !self.mic_test;
        }
        if check_updates {
            self.update_failed = None;
            self.last_update_check = self.now;
            self.updater.check(false);
        }
        if download_update {
            self.updater.download();
        }
        if restart_update {
            let ctx = ui.ctx().clone();
            self.restart_to_update(&ctx);
        }
        if record_key {
            self.recording_ptt = true;
        }
        if refresh_devices {
            self.device_lists = None;
        }
        if let Some(s) = preview {
            // Previews play even if that kind of sound is switched off.
            let dev = self.data.settings.output_device.clone();
            self.audio.play(s, dev.as_deref());
        }
        if restart_lan {
            self.lan = None;
            self.lan_error = None;
            self.lan_seen.clear();
            if self.data.settings.lan_enabled {
                self.start_lan();
            }
        }
        if reset {
            let name = self.data.me().name.clone();
            let settings = self.data.settings.clone();
            self.data = sample(now_secs(), &name);
            self.data.settings = settings;
            self.leave_voice();
            self.pending.clear();
            self.channel_for.clear();
            self.open_community(HOME_NET_COMMUNITY);
        }
        self.dirty = true;
    }
}
