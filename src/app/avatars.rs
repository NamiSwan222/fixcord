//! Profile pictures: choosing one, sharing it with friends, and drawing it.
//!
//! A picture is cropped to a square, shrunk to 128 × 128 and kept as a small JPEG with the
//! account. Friends see its fingerprint in each hello and ask for the picture only when it changed.

use super::*;
use base64::Engine;

/// Side of a stored profile picture, in pixels.
pub(super) const AVATAR_PX: u32 = 128;
/// Largest picture accepted from the network (base64 text).
const MAX_AVATAR_B64: usize = 48_000;

/// Pictures to draw for people, by display name and colour (how avatars are drawn everywhere).
static AVATARS: Mutex<Vec<(String, u8, egui::TextureId)>> = Mutex::new(Vec::new());

/// The picture for a name and colour, if that person has one.
pub(super) fn avatar_texture(name: &str, hue: u8) -> Option<egui::TextureId> {
    AVATARS.lock().ok()?.iter().find(|(n, h, _)| n == name && *h == hue).map(|(_, _, t)| *t)
}

fn fingerprint(bytes: &[u8]) -> u64 {
    let d = Sha256::digest(bytes);
    u64::from_le_bytes(d[..8].try_into().unwrap()).max(1)
}

/// Any picture file → a 128 × 128 JPEG (centre square), base64, and its fingerprint.
pub(super) fn make_avatar(file_bytes: &[u8]) -> Result<(String, u64), String> {
    let img = image::load_from_memory(file_bytes).map_err(|_| "That file isn't a picture Hearth can read (PNG, JPEG, GIF, BMP or WebP).".to_string())?;
    let side = img.width().min(img.height());
    if side < 16 {
        return Err("That picture is too small.".into());
    }
    let x = (img.width() - side) / 2;
    let y = (img.height() - side) / 2;
    let square = img.crop_imm(x, y, side, side).resize_exact(AVATAR_PX, AVATAR_PX, image::imageops::FilterType::Triangle).to_rgb8();
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 85)
        .encode(square.as_raw(), AVATAR_PX, AVATAR_PX, image::ExtendedColorType::Rgb8)
        .map_err(|e| e.to_string())?;
    let hash = fingerprint(&jpeg);
    Ok((base64::engine::general_purpose::STANDARD.encode(jpeg), hash))
}

fn decode_avatar(b64: &str) -> Option<egui::ColorImage> {
    if b64.len() > MAX_AVATAR_B64 {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg).ok()?;
    if img.width() > 512 || img.height() > 512 {
        return None;
    }
    let rgba = img.to_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], &rgba))
}

impl HearthApp {
    /// Open the file picker (in the background so the window keeps drawing).
    pub(super) fn pick_avatar(&mut self) {
        if self.avatar_pick.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let file = rfd::FileDialog::new()
                .set_title("Choose a profile picture")
                .add_filter("Pictures", &["png", "jpg", "jpeg", "gif", "bmp", "webp"])
                .pick_file();
            let _ = tx.send(file);
        });
        self.avatar_pick = Some(rx);
    }

    pub(super) fn remove_avatar(&mut self) {
        if let Some(me) = self.data.user_mut(ME) {
            me.avatar = None;
            me.avatar_hash = 0;
        }
        self.avatar_note = None;
        self.dirty = true;
        self.last_hello = -100.0;
    }

    /// Every frame: finish a picture choice, and keep the drawn pictures up to date.
    pub(super) fn update_avatars(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.avatar_pick {
            match rx.try_recv() {
                Ok(Some(path)) => {
                    self.avatar_pick = None;
                    let result = std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| make_avatar(&b));
                    match result {
                        Ok((b64, hash)) => {
                            if let Some(me) = self.data.user_mut(ME) {
                                me.avatar = Some(b64);
                                me.avatar_hash = hash;
                            }
                            self.avatar_note = None;
                            self.dirty = true;
                            self.last_hello = -100.0; // tell friends right away
                        }
                        Err(e) => self.avatar_note = Some(e),
                    }
                }
                Ok(None) | Err(std::sync::mpsc::TryRecvError::Disconnected) => self.avatar_pick = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        // Textures for everyone with a picture (made once per picture).
        let mut list = vec![];
        for u in &self.data.users {
            let (Some(b64), hash) = (&u.avatar, u.avatar_hash) else { continue };
            let key = (u.id, hash);
            if !self.avatar_tex.contains_key(&key) {
                if let Some(img) = decode_avatar(b64) {
                    let tex = ctx.load_texture(format!("avatar-{}-{hash}", u.id), img, egui::TextureOptions::LINEAR);
                    self.avatar_tex.insert(key, tex);
                }
            }
            if let Some(t) = self.avatar_tex.get(&key) {
                list.push((u.name.clone(), u.hue, t.id()));
            }
        }
        self.avatar_tex.retain(|(id, hash), _| self.data.users.iter().any(|u| u.id == *id && u.avatar_hash == *hash));
        if let Ok(mut a) = AVATARS.lock() {
            *a = list;
        }
    }

    /// A friend's hello mentioned their picture: ask for it if ours is out of date.
    pub(super) fn want_avatar(&mut self, uid: Id, account: u64, hash: u64) {
        if account == 0 || hash == 0 {
            return;
        }
        let have = self.data.user(uid).map(|u| u.avatar_hash == hash).unwrap_or(false);
        let asked = self.avatar_asked.get(&uid).map(|t| self.now - *t < 10.0).unwrap_or(false);
        if !have && !asked {
            self.avatar_asked.insert(uid, self.now);
            self.lan_send(Packet::AvatarAsk { from: self.instance, account });
        }
    }

    pub(super) fn on_avatar_packet(&mut self, p: Packet) {
        match p {
            Packet::AvatarAsk { account, .. } if account == self.data.account.id => {
                if self.now - self.avatar_sent_at < 2.0 {
                    return;
                }
                let me = self.data.me().clone();
                if let Some(b64) = me.avatar {
                    self.avatar_sent_at = self.now;
                    self.lan_send(Packet::Avatar { from: self.instance, account, hash: me.avatar_hash, jpeg: b64 });
                }
            }
            Packet::Avatar { account, hash, jpeg, .. } => {
                if account == 0 || jpeg.len() > MAX_AVATAR_B64 || decode_avatar(&jpeg).is_none() {
                    return;
                }
                let uid = peer_user_id(account);
                if let Some(u) = self.data.user_mut(uid) {
                    if u.avatar_hash != hash {
                        u.avatar = Some(jpeg);
                        u.avatar_hash = hash;
                        self.dirty = true;
                    }
                }
            }
            _ => {}
        }
    }
}
