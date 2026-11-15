//! Data for Hearth: people, communities, channels, messages, direct messages and settings.
//! Everything here is original sample content.

use serde::{Deserialize, Serialize};

pub type Id = u64;

/// The person using the app always has id 0.
pub const ME: Id = 0;
/// Id of the built-in community whose text channels are shared over the home network.
pub const HOME_NET_COMMUNITY: Id = 900;

/// Longest message the composer accepts (estimated; see MECHANICS.md).
pub const MAX_MESSAGE_CHARS: usize = 2000;
/// Messages by the same author closer together than this are shown as one group (estimated).
pub const GROUP_WINDOW_SECS: i64 = 5 * 60;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum Status {
    Online,
    Away,
    Busy,
    Hidden,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Online => "Online",
            Status::Away => "Away",
            Status::Busy => "Do not disturb",
            Status::Hidden => "Invisible",
        }
    }
    pub fn shows_online(self) -> bool {
        !matches!(self, Status::Hidden)
    }
    pub const ALL: [Status; 4] = [Status::Online, Status::Away, Status::Busy, Status::Hidden];
}

#[derive(Clone, Serialize, Deserialize)]
pub struct User {
    pub id: Id,
    pub name: String,
    pub status: Status,
    pub hue: u8, // index into avatar palette
    pub about: String,
    pub activity: String,
    pub friend: bool,
    pub pending_in: bool,
    pub pending_out: bool,
    /// Profile picture: a small square JPEG, base64 (None = coloured circle with a letter).
    #[serde(default)]
    pub avatar: Option<String>,
    /// Fingerprint of the picture (0 = none).
    #[serde(default)]
    pub avatar_hash: u64,
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Reaction {
    pub emoji: String,
    pub by: Vec<Id>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: Id,
    pub author: Id,
    /// Name to show for authors that aren't in the user list (home-network peers).
    #[serde(default)]
    pub author_name: Option<String>,
    pub text: String,
    pub ts: i64,
    #[serde(default)]
    pub edited: bool,
    #[serde(default)]
    pub reactions: Vec<Reaction>,
    #[serde(default)]
    pub reply_to: Option<Id>,
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelKind {
    Text,
    Voice,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Channel {
    pub id: Id,
    pub name: String,
    pub kind: ChannelKind,
    pub topic: String,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub unread: usize,
    #[serde(default)]
    pub mentions: usize,
    /// Sample people sitting in a voice channel.
    #[serde(default)]
    pub voice_people: Vec<Id>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Community {
    pub id: Id,
    pub name: String,
    pub hue: u8,
    pub channels: Vec<Channel>,
    pub members: Vec<Id>,
    #[serde(default)]
    pub muted: bool,
}

impl Community {
    pub fn unread(&self) -> bool {
        self.channels.iter().any(|c| c.unread > 0)
    }
    pub fn mentions(&self) -> usize {
        self.channels.iter().map(|c| c.mentions).sum()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct DirectThread {
    pub with: Id,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub unread: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    Ember,
    Midnight,
    Daylight,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: Theme,
    pub accent: u8,
    pub compact: bool,
    pub text_scale: f32,
    pub show_timestamps: bool,
    pub sample_replies: bool,
    pub lan_enabled: bool,
    pub lan_port: u16,
    pub show_members: bool,
    // Chat
    pub clock_24h: bool,
    pub day_dividers: bool,
    pub group_messages: bool,
    pub enter_sends: bool,
    pub show_typing: bool,
    /// 0 = relaxed, 1 = normal, 2 = quick
    pub reply_speed: u8,
    // Notifications
    pub flash_on_message: bool,
    pub mentions_only: bool,
    pub show_badges: bool,
    // Appearance & accessibility
    pub reduced_motion: bool,
    pub high_contrast: bool,
    pub square_icons: bool,
    // Window
    pub skip_splash: bool,
    pub always_on_top: bool,
    // Privacy
    pub share_status_lan: bool,
    pub share_activity: bool,
    // Streaming (last choices in the Go live picker)
    /// Index into media::QUALITIES.
    pub screen_quality: u8,
    pub screen_fps: u8,
    pub camera_quality: u8,
    pub camera_fps: u8,
    // Voice
    /// None = the computer's default.
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    /// 1.0 = 100 %.
    pub input_volume: f32,
    pub output_volume: f32,
    pub push_to_talk: bool,
    /// Key name (as egui names keys, e.g. "F8").
    pub ptt_key: String,
    /// Voice activity: the microphone opens above this level (dB).
    pub mic_sensitivity_db: f32,
    pub echo_reduction: bool,
    /// Remove background noise from the microphone (RNNoise).
    pub noise_suppression: bool,
    /// 0 = RNNoise (light), 1 = DeepFilterNet (stronger).
    pub noise_model: u8,
    /// DeepFilterNet: most it may turn noise down, in dB (100 = no limit).
    pub df_strength_db: f32,
    // Updates
    pub auto_update_check: bool,
    pub auto_update_install: bool,
    /// Volume for each home-network friend by name (1.0 = 100 %).
    pub user_volumes: std::collections::HashMap<String, f32>,
    // Interface sounds
    pub ui_sounds: bool,
    pub ui_volume: f32,
    pub sound_messages: bool,
    pub sound_mentions: bool,
    pub sound_voice: bool,
    pub sound_controls: bool,
    /// Friends' computers added by address (for networks where broadcasts don't get through).
    pub manual_peers: Vec<String>,
    /// Arena: how fast the mouse turns you (1.0 = normal).
    pub mouse_sensitivity: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: Theme::Ember,
            accent: 0,
            compact: false,
            text_scale: 1.0,
            show_timestamps: true,
            sample_replies: true,
            lan_enabled: true,
            lan_port: 47615,
            show_members: true,
            clock_24h: true,
            day_dividers: true,
            group_messages: true,
            enter_sends: true,
            show_typing: true,
            reply_speed: 1,
            flash_on_message: true,
            mentions_only: false,
            show_badges: true,
            reduced_motion: false,
            high_contrast: false,
            square_icons: false,
            skip_splash: false,
            always_on_top: false,
            share_status_lan: true,
            share_activity: true,
            screen_quality: 1,
            screen_fps: 15,
            camera_quality: 0,
            camera_fps: 30,
            input_device: None,
            output_device: None,
            input_volume: 1.0,
            output_volume: 1.0,
            push_to_talk: false,
            ptt_key: "F8".into(),
            mic_sensitivity_db: -45.0,
            echo_reduction: true,
            noise_suppression: true,
            noise_model: 1,
            df_strength_db: 100.0,
            auto_update_check: true,
            auto_update_install: true,
            user_volumes: Default::default(),
            ui_sounds: true,
            ui_volume: 0.6,
            sound_messages: true,
            sound_mentions: true,
            sound_voice: true,
            sound_controls: true,
            manual_peers: vec![],
            mouse_sensitivity: 1.0,
        }
    }
}

/// Who you are: a stable id that never changes (friends recognise you by it), a username and an
/// optional PIN (stored only as a salted hash). id 0 = not set up yet.
#[derive(Clone, Serialize, Deserialize, Default, Debug)]
pub struct Account {
    pub id: u64,
    pub username: String,
    pub created: i64,
    #[serde(default)]
    pub pin_hash: Option<String>,
    #[serde(default)]
    pub pin_salt: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Data {
    #[serde(default)]
    pub account: Account,
    pub users: Vec<User>,
    pub communities: Vec<Community>,
    pub dms: Vec<DirectThread>,
    pub settings: Settings,
    pub next_id: Id,
}

impl Data {
    pub fn new_id(&mut self) -> Id {
        self.next_id += 1;
        self.next_id
    }
    pub fn user(&self, id: Id) -> Option<&User> {
        self.users.iter().find(|u| u.id == id)
    }
    pub fn user_mut(&mut self, id: Id) -> Option<&mut User> {
        self.users.iter_mut().find(|u| u.id == id)
    }
    pub fn me(&self) -> &User {
        self.user(ME).expect("me exists")
    }
    pub fn community(&self, id: Id) -> Option<&Community> {
        self.communities.iter().find(|c| c.id == id)
    }
    pub fn community_mut(&mut self, id: Id) -> Option<&mut Community> {
        self.communities.iter_mut().find(|c| c.id == id)
    }
    pub fn dm_mut(&mut self, with: Id) -> &mut DirectThread {
        if let Some(i) = self.dms.iter().position(|d| d.with == with) {
            return &mut self.dms[i];
        }
        self.dms.insert(0, DirectThread { with, messages: vec![], unread: 0 });
        &mut self.dms[0]
    }
}

// ---------------------------------------------------------------- sample content

pub const REPLIES: &[&str] = &[
    "Ha, that's a good one.",
    "Totally agree with you there.",
    "Wait, really? Tell me more!",
    "I was just thinking the same thing.",
    "Love that idea 🙂",
    "Let's do it this weekend?",
    "Hmm, not sure about that, but I'm curious.",
    "Saving this for later.",
    "Okay that made my day 😄",
    "Count me in!",
    "Good point. I hadn't looked at it that way.",
    "Brb, getting tea ☕",
];

pub const GREETINGS: &[&str] = &["Hey there! 👋", "Hi! Good to see you.", "Hello hello!", "Heyy, what's up?"];
pub const ANSWERS: &[&str] = &[
    "Good question. I think yes, mostly?",
    "No idea, but someone here must know!",
    "I'd say go for it.",
    "Let me check and get back to you.",
];

fn user(id: Id, name: &str, status: Status, hue: u8, about: &str, activity: &str, friend: bool) -> User {
    User {
        id,
        name: name.into(),
        status,
        hue,
        about: about.into(),
        activity: activity.into(),
        friend,
        pending_in: false,
        pending_out: false,
        avatar: None,
        avatar_hash: 0,
    }
}

struct Seq {
    next: std::cell::Cell<Id>,
}
impl Seq {
    fn id(&self) -> Id {
        self.next.set(self.next.get() + 1);
        self.next.get()
    }
}

fn msg(seq: &Seq, author: Id, text: &str, ts: i64) -> Message {
    Message {
        id: seq.id(),
        author,
        author_name: None,
        text: text.into(),
        ts,
        edited: false,
        reactions: vec![],
        reply_to: None,
        pinned: false,
    }
}

fn text_channel(seq: &Seq, name: &str, topic: &str, messages: Vec<Message>) -> Channel {
    Channel {
        id: seq.id(),
        name: name.into(),
        kind: ChannelKind::Text,
        topic: topic.into(),
        messages,
        unread: 0,
        mentions: 0,
        voice_people: vec![],
    }
}

fn voice_channel(seq: &Seq, name: &str, people: Vec<Id>) -> Channel {
    Channel {
        id: seq.id(),
        name: name.into(),
        kind: ChannelKind::Voice,
        topic: String::new(),
        messages: vec![],
        unread: 0,
        mentions: 0,
        voice_people: people,
    }
}

/// Fresh sample data. `now` is the current unix time in seconds.
pub fn sample(now: i64, my_name: &str) -> Data {
    let users = vec![
        user(ME, my_name, Status::Online, 0, "Building my own corner of the internet.", "", false),
        user(1, "Mira", Status::Online, 1, "Reads too many novels. Tea over coffee.", "Reading", true),
        user(2, "Juno", Status::Online, 2, "Pixel artist and plant parent.", "Drawing", true),
        user(3, "Theo", Status::Away, 3, "Trail runner, bad at stretching.", "", true),
        user(4, "Pax", Status::Busy, 4, "Focus mode. Ping me if it's urgent.", "Focusing", true),
        user(5, "Rowan", Status::Online, 5, "Collects maps of places I've never been.", "", false),
        user(6, "Ilse", Status::Hidden, 6, "Night owl.", "", true),
        user(7, "Odette", Status::Online, 7, "Bakes on Sundays.", "Baking", false),
        user(8, "Kai", Status::Away, 0, "Game nights organiser.", "", false),
        User { pending_in: true, ..user(9, "Sol", Status::Online, 1, "New here, say hi!", "", false) },
        user(10, "Bram", Status::Hidden, 2, "Mostly lurking.", "", false),
    ];

    let s = Seq { next: std::cell::Cell::new(1000) };
    let h = 3600;
    let d = 86_400;

    // Home network: shared with other computers on the same network.
    let home = Community {
        id: HOME_NET_COMMUNITY,
        name: "Home Network".into(),
        hue: 4,
        channels: vec![
            text_channel(&s, "lobby", "Messages here go to everyone running Hearth on this network.", vec![]),
            text_channel(&s, "random", "Anything goes, shared over the home network.", vec![]),
            voice_channel(&s, "Lounge", vec![]),
        ],
        members: vec![ME],
        muted: false,
    };

    let dms = vec![
        DirectThread {
            with: 1,
            messages: vec![
                msg(&s, 1, "Did you get the book I recommended?", now - 26 * h),
                msg(&s, ME, "Yes! Starting tonight.", now - 25 * h),
                msg(&s, 1, "Yay, tell me what you think 🙂", now - 50 * 60),
            ],
            unread: 1,
        },
        DirectThread {
            with: 2,
            messages: vec![msg(&s, 2, "Sent you the palette file.", now - 3 * d)],
            unread: 0,
        },
        DirectThread {
            with: 4,
            messages: vec![msg(&s, ME, "Good luck with the deadline!", now - 4 * d)],
            unread: 0,
        },
    ];


    let data = Data {
        account: Account::default(),
        users,
        communities: vec![home],
        dms,
        settings: Settings::default(),
        next_id: s.next.get() + 1,
    };
    data
}
