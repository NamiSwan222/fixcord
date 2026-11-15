//! Home-network chat: every running copy of Hearth on the same network shares the
//! "Home Network" community. Messages and presence are sent as small JSON packets
//! by UDP broadcast; nothing leaves the local network. Screen and camera pictures go
//! directly between computers over TCP (see media.rs); the hello packet says where.

use serde::{Deserialize, Serialize};
use std::net::{IpAddr, UdpSocket};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

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
    },
    /// A chat message in one of the shared channels.
    Say { from: u64, name: String, channel: String, text: String, id: u64 },
}

pub struct Lan {
    socket: UdpSocket,
    pub port: u16,
    pub instance: u64,
    rx: Receiver<(Packet, IpAddr)>,
}

impl Lan {
    pub fn start(port: u16, instance: u64) -> Result<Lan, String> {
        let socket = UdpSocket::bind(("0.0.0.0", port)).map_err(|e| format!("Couldn't open port {port}: {e}"))?;
        socket.set_broadcast(true).map_err(|e| e.to_string())?;
        let reader = socket.try_clone().map_err(|e| e.to_string())?;
        reader.set_read_timeout(Some(Duration::from_millis(500))).ok();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.recv_from(&mut buf) {
                    Ok((n, addr)) => {
                        if let Ok(p) = serde_json::from_slice::<Packet>(&buf[..n]) {
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
        Ok(Lan { socket, port, instance, rx })
    }

    pub fn send(&self, p: &Packet) {
        if let Ok(bytes) = serde_json::to_vec(p) {
            let _ = self.socket.send_to(&bytes, ("255.255.255.255", self.port));
        }
    }

    /// Packets from other copies with the address they came from (our own echoes are dropped).
    pub fn poll(&self) -> Vec<(Packet, IpAddr)> {
        let mut out = vec![];
        while let Ok((p, ip)) = self.rx.try_recv() {
            let from = match &p {
                Packet::Hello { from, .. } | Packet::Say { from, .. } => *from,
            };
            if from != self.instance {
                out.push((p, ip));
            }
        }
        out
    }
}
