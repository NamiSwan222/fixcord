//! Home-network chat: every running copy of Hearth on the same network shares the
//! "Home Network" community.
//!
//! Everything uses one fixed port (47615) so friends always find each other and only one
//! firewall rule is needed: chat, presence and games as small JSON messages over UDP, voice as
//! UDP pieces on the same port (they start with "HV1"), and screen/camera pictures over TCP on
//! the same port number. Messages go to the network's broadcast address on every network
//! adapter (so a VPN or virtual adapter can't hide friends), and straight to each friend's
//! computer once it has been heard from or added by its address. Nothing leaves the local network.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The one port every copy of Hearth uses.
pub const PORT: u16 = 47615;

/// "I'm streaming my screen/camera; connect to this TCP port to watch."
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct StreamAd {
    pub kind: String,
    pub port: u16,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "kind")]
pub enum Packet {
    /// "I'm here" — sent every few seconds.
    Hello {
        from: u64,
        name: String,
        status: String,
        #[serde(default)]
        activity: String,
        /// Name of the Home Network voice channel I'm in ("" when none).
        #[serde(default)]
        voice: String,
        #[serde(default)]
        streams: Vec<StreamAd>,
        /// UDP port that receives my friends' voices (0 = none).
        #[serde(default)]
        voice_port: u16,
        /// My account's stable id (0 = a version without accounts).
        #[serde(default)]
        account: u64,
        /// Fingerprint of my profile picture (0 = none), so friends know when to ask for it.
        #[serde(default)]
        avatar: u64,
    },
    /// A chat message in one of the shared channels.
    Say {
        from: u64,
        name: String,
        channel: String,
        text: String,
        id: u64,
        #[serde(default)]
        account: u64,
    },
    /// Snake: the host's board (also tells everyone a game exists, so they can join).
    SnakeBoard { from: u64, game: u64, host: String, board: crate::snake::Board },
    /// Snake: "let me play" (sent to the host's game).
    SnakeJoin { from: u64, game: u64, name: String, hue: u8 },
    /// Snake: a key press.
    SnakeInput { from: u64, game: u64, dir: crate::snake::Dir },
    /// Snake: "I'm leaving".
    SnakeLeave { from: u64, game: u64 },
    /// "Please send me your profile picture" (to the account given).
    AvatarAsk { from: u64, account: u64 },
    /// A profile picture: a small JPEG, base64.
    Avatar { from: u64, account: u64, hash: u64, jpeg: String },
    /// Arena (3D game): the host's match state; also an invite.
    ArenaMatch { from: u64, game: u64, host: String, state: crate::arena::MatchState },
    /// Arena: where a player is and how they're doing (sent many times a second).
    ArenaPlayer { from: u64, game: u64, player: crate::arena::PlayerState },
    /// Arena: a shot (for everyone's effects); `hit` is who it hit (0 = nobody).
    ArenaShot { from: u64, game: u64, shot: crate::arena::Shot },
    /// Arena: "I was knocked out by …".
    ArenaDown { from: u64, game: u64, by: u64 },
    /// Arena: "I'm leaving".
    ArenaLeave { from: u64, game: u64 },
}

impl Packet {
    pub fn sender(&self) -> u64 {
        match self {
            Packet::Hello { from, .. }
            | Packet::Say { from, .. }
            | Packet::SnakeBoard { from, .. }
            | Packet::SnakeJoin { from, .. }
            | Packet::SnakeInput { from, .. }
            | Packet::SnakeLeave { from, .. }
            | Packet::AvatarAsk { from, .. }
            | Packet::Avatar { from, .. }
            | Packet::ArenaMatch { from, .. }
            | Packet::ArenaPlayer { from, .. }
            | Packet::ArenaShot { from, .. }
            | Packet::ArenaDown { from, .. }
            | Packet::ArenaLeave { from, .. } => *from,
        }
    }
}

/// Where voice pieces arriving on the shared port are handed over.
pub type VoiceSink = Arc<dyn Fn(&[u8]) + Send + Sync>;

/// This computer's IPv4 addresses on the network (not the loopback), with each one's
/// broadcast address.
pub fn local_addresses() -> Vec<(Ipv4Addr, Ipv4Addr)> {
    let Ok(list) = if_addrs::get_if_addrs() else { return vec![] };
    list.into_iter()
        .filter_map(|i| match i.addr {
            if_addrs::IfAddr::V4(v4) if !v4.ip.is_loopback() && !v4.ip.is_link_local() => {
                let bcast = v4.broadcast.unwrap_or_else(|| {
                    let ip = u32::from(v4.ip);
                    let mask = u32::from(v4.netmask);
                    Ipv4Addr::from(ip | !mask)
                });
                Some((v4.ip, bcast))
            }
            _ => None,
        })
        .collect()
}

pub struct Lan {
    socket: UdpSocket,
    pub port: u16,
    pub instance: u64,
    rx: Receiver<(Packet, IpAddr)>,
    /// Computers heard from (sent to directly as well as by broadcast).
    known: Arc<Mutex<HashSet<IpAddr>>>,
    /// Computers the person added by address.
    manual: Mutex<HashSet<IpAddr>>,
    /// Broadcast addresses of every adapter, refreshed now and then.
    broadcasts: Mutex<(Instant, Vec<Ipv4Addr>)>,
}

impl Lan {
    pub fn start(port: u16, instance: u64, voice: Option<VoiceSink>) -> Result<Lan, String> {
        let socket = UdpSocket::bind(("0.0.0.0", port)).map_err(|e| {
            format!("Couldn't open port {port} (is another copy of Hearth already running on this computer?): {e}")
        })?;
        socket.set_broadcast(true).map_err(|e| e.to_string())?;
        let reader = socket.try_clone().map_err(|e| e.to_string())?;
        reader.set_read_timeout(Some(Duration::from_millis(500))).ok();
        let (tx, rx) = channel();
        let known: Arc<Mutex<HashSet<IpAddr>>> = Arc::new(Mutex::new(HashSet::new()));
        let known_r = known.clone();
        std::thread::spawn(move || {
            // Big enough for a full game board or a profile picture.
            let mut buf = vec![0u8; 65_536];
            loop {
                match reader.recv_from(&mut buf) {
                    Ok((n, addr)) => {
                        let data = &buf[..n];
                        if data.starts_with(b"HV1") {
                            if let Some(v) = &voice {
                                v(data);
                            }
                            continue;
                        }
                        if let Ok(p) = serde_json::from_slice::<Packet>(data) {
                            if p.sender() != instance {
                                let mut k = known_r.lock().unwrap();
                                if k.len() < 256 {
                                    k.insert(addr.ip());
                                }
                            }
                            if tx.send((p, addr.ip())).is_err() {
                                break;
                            }
                        }
                    }
                    Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                    Err(_) => std::thread::sleep(Duration::from_millis(200)),
                }
            }
        });
        Ok(Lan {
            socket,
            port,
            instance,
            rx,
            known,
            manual: Mutex::new(HashSet::new()),
            broadcasts: Mutex::new((Instant::now() - Duration::from_secs(3600), vec![])),
        })
    }

    /// A copy of the shared socket (voice sends from it, so friends reply to the same port).
    pub fn socket(&self) -> Option<UdpSocket> {
        self.socket.try_clone().ok()
    }

    /// Reach a computer by its address even when broadcasts don't get through.
    pub fn add_address(&self, ip: IpAddr) {
        self.manual.lock().unwrap().insert(ip);
    }

    pub fn known_count(&self) -> usize {
        self.known.lock().unwrap().len()
    }

    fn destinations(&self) -> Vec<SocketAddr> {
        let mut out: Vec<SocketAddr> = vec![SocketAddr::from((Ipv4Addr::BROADCAST, self.port))];
        {
            let mut b = self.broadcasts.lock().unwrap();
            if b.0.elapsed() > Duration::from_secs(15) {
                b.1 = local_addresses().into_iter().map(|(_, bc)| bc).collect();
                b.0 = Instant::now();
            }
            out.extend(b.1.iter().map(|bc| SocketAddr::from((*bc, self.port))));
        }
        out.extend(self.known.lock().unwrap().iter().map(|ip| SocketAddr::new(*ip, self.port)));
        out.extend(self.manual.lock().unwrap().iter().map(|ip| SocketAddr::new(*ip, self.port)));
        out.sort();
        out.dedup();
        out
    }

    /// Send to everyone on the network (receivers ignore duplicates).
    pub fn send(&self, p: &Packet) {
        if let Ok(bytes) = serde_json::to_vec(p) {
            for d in self.destinations() {
                let _ = self.socket.send_to(&bytes, d);
            }
        }
    }

    /// Packets from other copies with the address they came from (our own echoes are dropped).
    pub fn poll(&self) -> Vec<(Packet, IpAddr)> {
        let mut out = vec![];
        while let Ok((p, ip)) = self.rx.try_recv() {
            if p.sender() != self.instance {
                out.push((p, ip));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_messages_still_read_and_new_fields_default() {
        let old = r#"{"kind":"Hello","from":5,"name":"Mira","status":"Online"}"#;
        match serde_json::from_str::<Packet>(old).unwrap() {
            Packet::Hello { account, avatar, voice_port, .. } => assert_eq!((account, avatar, voice_port), (0, 0, 0)),
            _ => panic!("not a hello"),
        }
    }

    #[test]
    fn this_computer_has_a_broadcast_address_for_each_network() {
        for (ip, bc) in local_addresses() {
            assert!(!ip.is_loopback());
            assert_ne!(bc, Ipv4Addr::UNSPECIFIED);
        }
    }
}
