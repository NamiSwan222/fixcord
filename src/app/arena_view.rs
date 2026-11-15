//! The Arena activity on screen: a first-person 3D view drawn with egui's own triangles,
//! mouse look, movement, the blaster, and the match's networking.

use super::*;
use crate::arena::{self, Body, Input as MoveInput, MatchState, Phase as ArenaPhase, PlayerState, Shot, V3};

/// A friend in the arena, as last heard (with the previous report, to move them smoothly).
struct Remote {
    state: PlayerState,
    prev_pos: V3,
    heard: f64,
    prev_heard: f64,
}

pub(super) struct ArenaSession {
    id: u64,
    host_instance: u64,
    host_name: String,
    /// Only on the host: the match clock.
    m: Option<arena::Match>,
    state: MatchState,
    others: HashMap<u64, Remote>,
    me: PlayerState,
    body: Body,
    fire_cd: f32,
    respawn_at: Option<f64>,
    tracers: Vec<(V3, V3, f64, u8)>,
    feed: Vec<(String, f64)>,
    last_send: f64,
    last_match_send: f64,
    last_frame: f64,
    last_heard_host: f64,
    hit_marker_at: f64,
    hurt_at: f64,
    world: Vec<arena::Block>,
}

pub(super) struct ArenaInvite {
    pub(super) host_name: String,
    pub(super) players: u8,
    pub(super) phase: ArenaPhase,
    seen: f64,
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn lerp3(a: V3, b: V3, t: f32) -> V3 {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Looking out from the eye: turns world points into screen points.
struct Camera {
    eye: V3,
    right: V3,
    up: V3,
    fwd: V3,
    center: Pos2,
    focal: f32,
}

const NEAR: f32 = 0.05;

impl Camera {
    fn new(eye: V3, yaw: f32, pitch: f32, rect: Rect) -> Camera {
        let fwd = arena::look_dir(yaw, pitch);
        let right = [yaw.cos(), 0.0, yaw.sin()];
        // up = right × forward
        let up = [
            right[1] * fwd[2] - right[2] * fwd[1],
            right[2] * fwd[0] - right[0] * fwd[2],
            right[0] * fwd[1] - right[1] * fwd[0],
        ];
        let fov = 75f32.to_radians();
        Camera { eye, right, up, fwd, center: rect.center(), focal: (rect.height() / 2.0) / (fov / 2.0).tan() }
    }
    fn to_cam(&self, p: V3) -> V3 {
        let d = sub(p, self.eye);
        [dot(d, self.right), dot(d, self.up), dot(d, self.fwd)]
    }
    fn project(&self, c: V3) -> Pos2 {
        pos2(self.center.x + c[0] / c[2] * self.focal, self.center.y - c[1] / c[2] * self.focal)
    }
    /// A world point on screen, if it's in front of us.
    fn screen(&self, p: V3) -> Option<Pos2> {
        let c = self.to_cam(p);
        (c[2] > NEAR).then(|| self.project(c))
    }
}

/// Keep the part of a polygon (camera space) in front of the near plane.
fn clip_near(poly: &[V3]) -> Vec<V3> {
    let mut out = vec![];
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        let (ia, ib) = (a[2] >= NEAR, b[2] >= NEAR);
        if ia {
            out.push(a);
        }
        if ia != ib {
            let t = (NEAR - a[2]) / (b[2] - a[2]);
            out.push(lerp3(a, b, t));
        }
    }
    out
}

/// A flat-coloured polygon waiting to be drawn, with its distance for back-to-front order.
struct Poly {
    pts: Vec<Pos2>,
    color: Color32,
    depth: f32,
}

fn shade(base: Color32, normal: V3, dist: f32, sky: Color32) -> Color32 {
    let light = [0.42, 0.8, 0.3];
    let l = (dot(normal, light) / 0.97).max(0.0);
    let k = 0.55 + 0.45 * l;
    let lit = Color32::from_rgb((base.r() as f32 * k) as u8, (base.g() as f32 * k) as u8, (base.b() as f32 * k) as u8);
    let fog = ((dist / 70.0).clamp(0.0, 1.0)).powf(1.5) * 0.65;
    lit.lerp_to_gamma(sky, fog)
}

/// The six faces of a box, as (corners, outward normal).
fn box_faces(min: V3, max: V3) -> [([V3; 4], V3); 6] {
    let (x0, y0, z0) = (min[0], min[1], min[2]);
    let (x1, y1, z1) = (max[0], max[1], max[2]);
    [
        ([[x0, y1, z0], [x1, y1, z0], [x1, y1, z1], [x0, y1, z1]], [0.0, 1.0, 0.0]),
        ([[x0, y0, z1], [x1, y0, z1], [x1, y0, z0], [x0, y0, z0]], [0.0, -1.0, 0.0]),
        ([[x0, y0, z1], [x0, y1, z1], [x1, y1, z1], [x1, y0, z1]], [0.0, 0.0, 1.0]),
        ([[x1, y0, z0], [x1, y1, z0], [x0, y1, z0], [x0, y0, z0]], [0.0, 0.0, -1.0]),
        ([[x1, y0, z1], [x1, y1, z1], [x1, y1, z0], [x1, y0, z0]], [1.0, 0.0, 0.0]),
        ([[x0, y0, z0], [x0, y1, z0], [x0, y1, z1], [x0, y0, z1]], [-1.0, 0.0, 0.0]),
    ]
}

fn push_face(cam: &Camera, corners: &[V3], normal: V3, base: Color32, sky: Color32, out: &mut Vec<Poly>) {
    let centre = [
        corners.iter().map(|c| c[0]).sum::<f32>() / corners.len() as f32,
        corners.iter().map(|c| c[1]).sum::<f32>() / corners.len() as f32,
        corners.iter().map(|c| c[2]).sum::<f32>() / corners.len() as f32,
    ];
    let to_eye = sub(cam.eye, centre);
    if dot(normal, to_eye) <= 0.0 {
        return; // facing away
    }
    let cam_pts: Vec<V3> = corners.iter().map(|c| cam.to_cam(*c)).collect();
    let clipped = clip_near(&cam_pts);
    if clipped.len() < 3 {
        return;
    }
    let dist = dot(to_eye, to_eye).sqrt();
    let depth = clipped.iter().map(|c| c[2]).sum::<f32>() / clipped.len() as f32;
    out.push(Poly { pts: clipped.iter().map(|c| cam.project(*c)).collect(), color: shade(base, normal, dist, sky), depth });
}

fn draw_polys(painter: &egui::Painter, polys: &mut [Poly]) {
    polys.sort_by(|a, b| b.depth.partial_cmp(&a.depth).unwrap_or(std::cmp::Ordering::Equal));
    let mut mesh = egui::Mesh::default();
    for poly in polys.iter() {
        let base = mesh.vertices.len() as u32;
        for p in &poly.pts {
            mesh.colored_vertex(*p, poly.color);
        }
        for i in 1..poly.pts.len() as u32 - 1 {
            mesh.add_triangle(base, base + i, base + i + 1);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

fn block_color(kind: u8) -> Color32 {
    match kind {
        0 => Color32::from_rgb(0xB8, 0x86, 0x4E), // crate
        1 => Color32::from_rgb(0x8A, 0x8F, 0x9C), // stone
        2 => Color32::from_rgb(0x6F, 0x8F, 0xB8), // platform
        _ => Color32::from_rgb(0x5E, 0x5A, 0x6E), // outer wall
    }
}

impl HearthApp {
    fn new_arena(&mut self, id: u64, host_instance: u64, host_name: String, host: bool) -> ArenaSession {
        let me_user = self.data.me().clone();
        let salt = self.rand();
        let spawn = arena::pick_spawn(&[], salt);
        let m = host.then(arena::Match::new);
        let state = m.as_ref().map(|m| m.state.clone()).unwrap_or_else(|| arena::Match::new().state);
        ArenaSession {
            id,
            host_instance,
            host_name,
            m,
            state,
            others: HashMap::new(),
            me: PlayerState {
                id: self.instance,
                name: me_user.name.clone(),
                hue: me_user.hue,
                pos: spawn,
                yaw: (-spawn[0]).atan2(spawn[2]) + std::f32::consts::PI, // face the middle
                pitch: 0.0,
                hp: 100,
                alive: true,
                kos: 0,
                downs: 0,
                round: 0,
            },
            body: Body { pos: spawn, vy: 0.0, on_ground: true },
            fire_cd: 0.0,
            respawn_at: None,
            tracers: vec![],
            feed: vec![],
            last_send: -100.0,
            last_match_send: -100.0,
            last_frame: self.now,
            last_heard_host: self.now,
            hit_marker_at: -100.0,
            hurt_at: -100.0,
            world: arena::world(),
        }
    }

    pub(super) fn host_arena(&mut self) {
        self.leave_activities();
        let id = self.rand() | 1;
        let name = self.data.me().name.clone();
        let s = self.new_arena(id, self.instance, name, true);
        self.arena = Some(s);
        self.show_arena = true;
        self.activities_open = false;
        self.sfx(Sfx::Join);
    }

    pub(super) fn join_arena(&mut self, game: u64) {
        let Some(inv) = self.arena_invites.get(&game) else { return };
        let host_name = inv.host_name.clone();
        let host_instance = self.arena_hosts.get(&game).copied().unwrap_or(0);
        self.leave_activities();
        let s = self.new_arena(game, host_instance, host_name, false);
        self.arena = Some(s);
        self.show_arena = true;
        self.activities_open = false;
        self.sfx(Sfx::Join);
    }

    /// Leave any game (Snake or Arena) before starting another.
    pub(super) fn leave_activities(&mut self) {
        if self.game.is_some() {
            self.leave_game();
        }
        if self.arena.is_some() {
            self.leave_arena();
        }
    }

    pub(super) fn leave_arena(&mut self) {
        if let Some(s) = self.arena.take() {
            self.lan_send(Packet::ArenaLeave { from: self.instance, game: s.id });
            self.sfx(Sfx::Leave);
        }
        self.show_arena = false;
        self.release_mouse();
    }

    pub(super) fn release_mouse(&mut self) {
        if self.mouse_captured {
            if let Some(ctx) = &self.egui_ctx {
                ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(egui::CursorGrab::None));
                ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(true));
            }
            self.mouse_captured = false;
        }
    }

    fn capture_mouse(&mut self, ctx: &egui::Context) {
        if !self.mouse_captured {
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(egui::CursorGrab::Confined));
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(false));
            self.mouse_captured = true;
        }
    }

    /// Every frame: the host's clock, our position report, respawning, and old effects.
    pub(super) fn tick_arena(&mut self, now: f64) {
        self.arena_invites.retain(|_, i| now - i.seen < 5.0);
        let instance = self.instance;
        let mut out: Vec<Packet> = vec![];
        let mut sound: Option<Sfx> = None;
        let Some(s) = &mut self.arena else { return };
        let dt = ((now - s.last_frame) as f32).clamp(0.0, 0.25);
        s.last_frame = now;
        s.fire_cd = (s.fire_cd - dt).max(0.0);
        s.others.retain(|_, r| now - r.heard < 5.0);
        // Back in the game after being knocked out.
        if let Some(t) = s.respawn_at {
            if now >= t {
                let others: Vec<V3> = s.others.values().map(|r| r.state.pos).collect();
                let spawn = arena::pick_spawn(&others, now as u64);
                s.body = Body { pos: spawn, vy: 0.0, on_ground: true };
                s.me.hp = 100;
                s.me.alive = true;
                s.respawn_at = None;
            }
        }
        if let Some(m) = &mut s.m {
            let mut all: Vec<PlayerState> = s.others.values().map(|r| r.state.clone()).collect();
            all.push(s.me.clone());
            let before = m.state.phase;
            m.update(dt, &all);
            m.state.players = all.len().min(255) as u8;
            if before != m.state.phase || now - s.last_match_send > 0.5 {
                s.last_match_send = now;
                out.push(Packet::ArenaMatch { from: instance, game: s.id, host: s.host_name.clone(), state: m.state.clone() });
            }
            let st = m.state.clone();
            s.state = st;
        }
        // New round: start fresh.
        if s.state.round != s.me.round && s.state.phase == ArenaPhase::Playing {
            s.me.round = s.state.round;
            s.me.kos = 0;
            s.me.downs = 0;
            s.me.hp = 100;
            s.me.alive = true;
            s.respawn_at = None;
            let others: Vec<V3> = s.others.values().map(|r| r.state.pos).collect();
            let spawn = arena::pick_spawn(&others, now as u64);
            s.body = Body { pos: spawn, vy: 0.0, on_ground: true };
            sound = Some(Sfx::LiveStart);
        }
        s.me.pos = s.body.pos;
        if now - s.last_send >= 1.0 / arena::SEND_HZ as f64 {
            s.last_send = now;
            out.push(Packet::ArenaPlayer { from: instance, game: s.id, player: s.me.clone() });
        }
        s.tracers.retain(|t| now - t.2 < 0.18);
        s.feed.retain(|f| now - f.1 < 6.0);
        for p in out {
            self.lan_send(p);
        }
        if let Some(snd) = sound {
            self.sfx(snd);
        }
    }

    pub(super) fn on_arena_packet(&mut self, p: Packet, now: f64) {
        let instance = self.instance;
        let mut sound: Option<Sfx> = None;
        let mut send: Option<Packet> = None;
        match p {
            Packet::ArenaMatch { from, game, host, state } => {
                self.arena_hosts.insert(game, from);
                self.arena_invites.insert(
                    game,
                    ArenaInvite { host_name: host.chars().take(32).collect(), players: state.players, phase: state.phase, seen: now },
                );
                if let Some(s) = &mut self.arena {
                    if s.id == game && s.m.is_none() {
                        s.host_instance = from;
                        s.last_heard_host = now;
                        if s.state.phase != state.phase && state.phase == ArenaPhase::Over {
                            sound = Some(if state.winner == instance { Sfx::Mention } else { Sfx::LiveStop });
                        }
                        s.state = state;
                    }
                }
            }
            Packet::ArenaPlayer { from, game, player } => {
                if let Some(s) = &mut self.arena {
                    if s.id == game && player.id == from {
                        let mut player = player;
                        player.name = player.name.chars().take(24).collect();
                        let r = s.others.entry(from).or_insert(Remote { prev_pos: player.pos, state: player.clone(), heard: now, prev_heard: now });
                        r.prev_pos = r.state.pos;
                        r.prev_heard = r.heard;
                        r.state = player;
                        r.heard = now;
                    }
                }
            }
            Packet::ArenaShot { from, game, shot } => {
                if let Some(s) = &mut self.arena {
                    if s.id == game {
                        let hue = s.others.get(&from).map(|r| r.state.hue).unwrap_or(0);
                        s.tracers.push((shot.from, shot.to, now, hue));
                        let playing = s.state.phase == ArenaPhase::Playing;
                        if shot.hit == instance && s.me.alive && playing {
                            s.me.hp -= shot.dmg.clamp(0, arena::HEADSHOT);
                            s.hurt_at = now;
                            sound = Some(Sfx::Hurt);
                            if s.me.hp <= 0 {
                                s.me.hp = 0;
                                s.me.alive = false;
                                s.me.downs += 1;
                                s.respawn_at = Some(now + arena::RESPAWN as f64);
                                let by_name = s.others.get(&from).map(|r| r.state.name.clone()).unwrap_or_default();
                                s.feed.push((format!("{by_name} tagged you"), now));
                                send = Some(Packet::ArenaDown { from: instance, game, by: from });
                                sound = Some(Sfx::Leave);
                            }
                        }
                    }
                }
            }
            Packet::ArenaDown { from, game, by } => {
                if let Some(s) = &mut self.arena {
                    if s.id == game {
                        let victim = s.others.get(&from).map(|r| r.state.name.clone()).unwrap_or_default();
                        if by == instance {
                            s.me.kos += 1;
                            s.feed.push((format!("You tagged {victim}"), now));
                            sound = Some(Sfx::Tagged);
                        } else {
                            let who = s.others.get(&by).map(|r| r.state.name.clone()).unwrap_or_else(|| "Someone".into());
                            s.feed.push((format!("{who} tagged {victim}"), now));
                        }
                    }
                }
            }
            Packet::ArenaLeave { from, game } => {
                if let Some(s) = &mut self.arena {
                    if s.id == game {
                        s.others.remove(&from);
                    }
                }
            }
            _ => {}
        }
        if let Some(p) = send {
            self.lan_send(p);
        }
        if let Some(snd) = sound {
            self.sfx(snd);
        }
    }

    /// The Arena screen.
    pub(super) fn arena_view(&mut self, ui: &mut Ui, p: &Palette, now: f64) {
        let ctx = ui.ctx().clone();
        let focused = self.window_focused;
        if !focused || ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.release_mouse();
        }
        let captured = self.mouse_captured;
        let sens = 0.0025 * self.data.settings.mouse_sensitivity;
        let instance = self.instance;
        let mut fire = false;
        let mut start = false;
        let mut leave = false;
        let mut back = false;
        let mut want_capture = false;
        let mut fired_shot: Option<Shot> = None;
        ctx.request_repaint(); // 3D view: draw every frame

        // Movement and looking (only while the mouse is captured).
        let (input, look, firing, tab) = ctx.input(|i| {
            let mut mv = MoveInput::default();
            if captured {
                if i.key_down(Key::W) || i.key_down(Key::ArrowUp) {
                    mv.forward += 1.0;
                }
                if i.key_down(Key::S) || i.key_down(Key::ArrowDown) {
                    mv.forward -= 1.0;
                }
                if i.key_down(Key::D) || i.key_down(Key::ArrowRight) {
                    mv.right += 1.0;
                }
                if i.key_down(Key::A) || i.key_down(Key::ArrowLeft) {
                    mv.right -= 1.0;
                }
                mv.jump = i.key_down(Key::Space);
            }
            let look = if captured { i.pointer.motion().unwrap_or(egui::Vec2::ZERO) } else { egui::Vec2::ZERO };
            (mv, look, captured && i.pointer.primary_down(), i.key_down(Key::Tab))
        });
        {
            let Some(s) = &mut self.arena else { return };
            let dt = ((1.0 / 60.0f64).max(ctx.input(|i| i.stable_dt as f64))) as f32;
            s.me.yaw += look.x * sens;
            s.me.pitch = (s.me.pitch - look.y * sens).clamp(-1.45, 1.45);
            if s.me.alive {
                arena::step(&mut s.body, input, s.me.yaw, dt.min(0.05), &s.world);
            }
            if firing && s.me.alive && s.fire_cd <= 0.0 {
                s.fire_cd = arena::FIRE_EVERY;
                fire = true;
                let eye = [s.body.pos[0], s.body.pos[1] + arena::EYE, s.body.pos[2]];
                let dir = arena::look_dir(s.me.yaw, s.me.pitch);
                let players: Vec<(u64, V3)> = s.others.iter().filter(|(_, r)| r.state.alive).map(|(id, r)| (*id, r.state.pos)).collect();
                let (hit, dist) = arena::trace(eye, dir, &s.world, &players, instance);
                let to = [eye[0] + dir[0] * dist, eye[1] + dir[1] * dist, eye[2] + dir[2] * dist];
                // Start the beam a little below and right of the eye, like a held blaster.
                let r = [s.me.yaw.cos(), 0.0, s.me.yaw.sin()];
                let from = [eye[0] + r[0] * 0.25, eye[1] - 0.2, eye[2] + r[2] * 0.25];
                let (hit_id, dmg) = match hit {
                    Some((id, head)) if s.state.phase == ArenaPhase::Playing => (id, if head { arena::HEADSHOT } else { arena::DAMAGE }),
                    _ => (0, 0),
                };
                if hit_id != 0 {
                    s.hit_marker_at = now;
                }
                s.tracers.push((from, to, now, s.me.hue));
                fired_shot = Some(Shot { from, to, hit: hit_id, dmg });
            }
        }
        if let Some(shot) = fired_shot {
            let game = self.arena.as_ref().map(|s| s.id).unwrap_or(0);
            let hit = shot.hit != 0;
            self.lan_send(Packet::ArenaShot { from: instance, game, shot });
            self.sfx(if hit { Sfx::Hit } else { Sfx::Shoot });
        }
        let _ = fire;

        let Some(s) = &self.arena else { return };
        let is_host = s.m.is_some();
        let host_gone = !is_host && now - s.last_heard_host > 6.0;

        // Header
        egui::Panel::top("arena_header")
            .exact_size(HEADER_H)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::new().fill(p.rail).inner_margin(Margin::symmetric(14, 0)))
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(RichText::new("🎯").size(16.0));
                    ui.label(RichText::new("Arena").size(16.0).color(p.strong).strong());
                    let who = if is_host { "You're hosting".to_string() } else { format!("Hosted by {}", s.host_name) };
                    ui.label(RichText::new(who).size(13.0).color(p.muted));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(egui::Button::new(RichText::new("Leave game").color(p.danger)).fill(p.input)).clicked() {
                            leave = true;
                        }
                        if ui.add(egui::Button::new(RichText::new("Back to chat").color(p.text)).fill(p.input)).clicked() {
                            back = true;
                        }
                    });
                });
            });

        egui::CentralPanel::default().frame(Frame::new().fill(Color32::BLACK)).show(ui, |ui| {
            let rect = ui.max_rect();
            let resp = ui.interact(rect, EId::new("arena_view"), Sense::click());
            if resp.clicked() && !captured {
                want_capture = true;
            }
            let painter = ui.painter_at(rect);
            // Sky
            let sky_top = Color32::from_rgb(0x2A, 0x3B, 0x63);
            let sky = Color32::from_rgb(0x9C, 0xB4, 0xD8);
            let mut sky_mesh = egui::Mesh::default();
            sky_mesh.colored_vertex(rect.left_top(), sky_top);
            sky_mesh.colored_vertex(rect.right_top(), sky_top);
            sky_mesh.colored_vertex(rect.right_bottom(), sky);
            sky_mesh.colored_vertex(rect.left_bottom(), sky);
            sky_mesh.add_triangle(0, 1, 2);
            sky_mesh.add_triangle(0, 2, 3);
            painter.add(egui::Shape::mesh(sky_mesh));

            let eye = [s.body.pos[0], s.body.pos[1] + arena::EYE, s.body.pos[2]];
            let cam = Camera::new(eye, s.me.yaw, s.me.pitch, rect);

            // Floor tiles first (always underneath), then walls, then everything else by distance.
            let mut floor: Vec<Poly> = vec![];
            let n = 8;
            let tile = arena::SIZE * 2.0 / n as f32;
            for zi in 0..n {
                for xi in 0..n {
                    let x0 = -arena::SIZE + xi as f32 * tile;
                    let z0 = -arena::SIZE + zi as f32 * tile;
                    let c = if (xi + zi) % 2 == 0 { Color32::from_rgb(0x4E, 0x6B, 0x4A) } else { Color32::from_rgb(0x46, 0x62, 0x43) };
                    let corners = [[x0, 0.0, z0], [x0 + tile, 0.0, z0], [x0 + tile, 0.0, z0 + tile], [x0, 0.0, z0 + tile]];
                    push_face(&cam, &corners, [0.0, 1.0, 0.0], c, sky, &mut floor);
                }
            }
            draw_polys(&painter, &mut floor);
            let mut walls: Vec<Poly> = vec![];
            let mut things: Vec<Poly> = vec![];
            for blk in &s.world {
                let target = if blk.kind == 3 { &mut walls } else { &mut things };
                for (corners, normal) in box_faces(blk.min, blk.max) {
                    push_face(&cam, &corners, normal, block_color(blk.kind), sky, target);
                }
            }
            // Friends: a body and a head in their colour, moving smoothly between reports.
            let mut tags: Vec<(V3, String, i16)> = vec![];
            for r in s.others.values() {
                if !r.state.alive {
                    continue;
                }
                let span = (r.heard - r.prev_heard).max(1.0 / arena::SEND_HZ as f64);
                let t = (((now - r.heard) / span) as f32).clamp(0.0, 1.0);
                let pos = lerp3(r.prev_pos, r.state.pos, t);
                let col = theme::hue(r.state.hue);
                let w = arena::RADIUS;
                for (corners, normal) in box_faces([pos[0] - w, pos[1], pos[2] - w], [pos[0] + w, pos[1] + 1.35, pos[2] + w]) {
                    push_face(&cam, &corners, normal, col, sky, &mut things);
                }
                let h = 0.25;
                let head_c = col.lerp_to_gamma(Color32::WHITE, 0.25);
                for (corners, normal) in box_faces([pos[0] - h, pos[1] + 1.35, pos[2] - h], [pos[0] + h, pos[1] + 1.85, pos[2] + h]) {
                    push_face(&cam, &corners, normal, head_c, sky, &mut things);
                }
                tags.push(([pos[0], pos[1] + 2.15, pos[2]], r.state.name.clone(), r.state.hp));
            }
            draw_polys(&painter, &mut walls);
            draw_polys(&painter, &mut things);
            // Name tags
            for (at, name, hp) in tags {
                let to = sub(at, eye);
                let dist = dot(to, to).sqrt().max(0.01);
                // Only for people you can actually see (not through walls).
                let dir = [to[0] / dist, to[1] / dist, to[2] / dist];
                let (_, clear) = arena::trace(eye, dir, &s.world, &[], instance);
                if clear + 0.3 < dist {
                    continue;
                }
                if let Some(sp) = cam.screen(at) {
                    let size = (16.0 - dist * 0.15).clamp(10.0, 16.0);
                    painter.text(sp, Align2::CENTER_BOTTOM, &name, FontId::proportional(size), Color32::WHITE);
                    let bar = Rect::from_center_size(sp + vec2(0.0, 4.0), vec2(40.0, 4.0));
                    painter.rect_filled(bar, CornerRadius::same(2), Color32::from_black_alpha(140));
                    painter.rect_filled(Rect::from_min_size(bar.min, vec2(40.0 * (hp.max(0) as f32 / 100.0), 4.0)), CornerRadius::same(2), Color32::from_rgb(0x6B, 0xD1, 0x7A));
                }
            }
            // Blaster beams
            for (a, b, at, hue) in &s.tracers {
                let fade = 1.0 - ((now - at) / 0.18) as f32;
                let (ca, cb) = (cam.to_cam(*a), cam.to_cam(*b));
                let seg = clip_near(&[ca, cb, cb]);
                if seg.len() >= 2 {
                    let col = theme::hue(*hue).lerp_to_gamma(Color32::WHITE, 0.5).gamma_multiply(fade.max(0.0));
                    painter.line_segment([cam.project(seg[0]), cam.project(seg[1])], Stroke::new(3.0, col));
                }
            }

            // HUD
            let c = rect.center();
            let ch = Color32::from_white_alpha(220);
            painter.line_segment([c - vec2(9.0, 0.0), c - vec2(3.0, 0.0)], Stroke::new(2.0, ch));
            painter.line_segment([c + vec2(3.0, 0.0), c + vec2(9.0, 0.0)], Stroke::new(2.0, ch));
            painter.line_segment([c - vec2(0.0, 9.0), c - vec2(0.0, 3.0)], Stroke::new(2.0, ch));
            painter.line_segment([c + vec2(0.0, 3.0), c + vec2(0.0, 9.0)], Stroke::new(2.0, ch));
            if now - s.hit_marker_at < 0.2 {
                let hm = Color32::from_rgb(0xFF, 0x6B, 0x6B);
                for (dx, dy) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
                    painter.line_segment([c + vec2(dx * 6.0, dy * 6.0), c + vec2(dx * 13.0, dy * 13.0)], Stroke::new(2.5, hm));
                }
            }
            if now - s.hurt_at < 0.35 {
                let a = (1.0 - (now - s.hurt_at) / 0.35) as f32;
                painter.rect_stroke(rect.shrink(6.0), CornerRadius::same(8), Stroke::new(14.0, Color32::from_rgba_unmultiplied(220, 40, 40, (120.0 * a) as u8)), StrokeKind::Inside);
            }
            // Health and score
            let hb = Rect::from_min_size(rect.left_bottom() + vec2(24.0, -50.0), vec2(220.0, 22.0));
            painter.rect_filled(hb, CornerRadius::same(6), Color32::from_black_alpha(150));
            let frac = s.me.hp.max(0) as f32 / 100.0;
            let hp_col = if frac > 0.5 { Color32::from_rgb(0x6B, 0xD1, 0x7A) } else if frac > 0.25 { Color32::from_rgb(0xF2, 0xB3, 0x3D) } else { Color32::from_rgb(0xEF, 0x5B, 0x5B) };
            painter.rect_filled(Rect::from_min_size(hb.min, vec2(hb.width() * frac, hb.height())), CornerRadius::same(6), hp_col);
            painter.text(hb.left_center() + vec2(8.0, 0.0), Align2::LEFT_CENTER, format!("{} health", s.me.hp.max(0)), FontId::proportional(13.0), Color32::WHITE);
            painter.text(hb.left_top() - vec2(0.0, 8.0), Align2::LEFT_BOTTOM, format!("Tags {}   ·   Tagged {}", s.me.kos, s.me.downs), FontId::proportional(14.0), Color32::WHITE);
            // Clock and goal
            let status = match s.state.phase {
                ArenaPhase::Lobby => "Warm-up".to_string(),
                ArenaPhase::Playing => {
                    let t = s.state.time_left.max(0.0) as u32;
                    format!("{}:{:02}  ·  first to {}", t / 60, t % 60, s.state.target)
                }
                ArenaPhase::Over => "Round over".to_string(),
            };
            let top = Rect::from_center_size(rect.center_top() + vec2(0.0, 24.0), vec2(240.0, 30.0));
            painter.rect_filled(top, CornerRadius::same(8), Color32::from_black_alpha(140));
            painter.text(top.center(), Align2::CENTER_CENTER, status, FontId::proportional(15.0), Color32::WHITE);
            // Tag feed
            for (i, (line, _)) in s.feed.iter().rev().take(5).enumerate() {
                painter.text(rect.right_top() + vec2(-18.0, 20.0 + i as f32 * 20.0), Align2::RIGHT_TOP, line, FontId::proportional(13.5), Color32::WHITE);
            }
            if !s.me.alive {
                let left = s.respawn_at.map(|t| (t - now).max(0.0)).unwrap_or(0.0);
                painter.text(c + vec2(0.0, 60.0), Align2::CENTER_CENTER, format!("Tagged! Back in {left:.1}s"), FontId::proportional(22.0), Color32::WHITE);
            }

            // Scoreboard: hold Tab, and between rounds.
            let show_board = tab || matches!(s.state.phase, ArenaPhase::Lobby | ArenaPhase::Over);
            let mut rows: Vec<(String, u16, u16, u8, bool)> =
                s.others.values().map(|r| (r.state.name.clone(), r.state.kos, r.state.downs, r.state.hue, false)).collect();
            rows.push((s.me.name.clone(), s.me.kos, s.me.downs, s.me.hue, true));
            rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.2.cmp(&b.2)));
            if show_board {
                let h = 70.0 + rows.len() as f32 * 26.0 + if is_host && s.state.phase != ArenaPhase::Playing { 50.0 } else { 0.0 };
                let board = Rect::from_center_size(c - vec2(0.0, 30.0), vec2(380.0, h));
                painter.rect_filled(board, CornerRadius::same(14), p.raised.gamma_multiply(0.93));
                let title = match s.state.phase {
                    ArenaPhase::Over => match rows.iter().find(|r| r.4).filter(|_| s.state.winner == instance) {
                        Some(_) => "You win! 🎉".to_string(),
                        None => {
                            let w = s.others.get(&s.state.winner).map(|r| r.state.name.clone());
                            w.map(|n| format!("{n} wins!")).unwrap_or_else(|| "It's a draw!".into())
                        }
                    },
                    ArenaPhase::Lobby => format!("Warm-up · {} here", rows.len()),
                    ArenaPhase::Playing => "Scores".to_string(),
                };
                painter.text(board.center_top() + vec2(0.0, 16.0), Align2::CENTER_TOP, title, FontId::proportional(20.0), p.strong);
                let mut y = board.top() + 56.0;
                for (name, k, d, hue, me) in &rows {
                    painter.circle_filled(pos2(board.left() + 24.0, y + 8.0), 6.0, theme::hue(*hue));
                    let label = if *me { format!("{name} (you)") } else { name.clone() };
                    painter.text(pos2(board.left() + 38.0, y + 8.0), Align2::LEFT_CENTER, label, FontId::proportional(14.5), p.text);
                    painter.text(pos2(board.right() - 24.0, y + 8.0), Align2::RIGHT_CENTER, format!("{k} tags · {d} tagged"), FontId::proportional(13.5), p.muted);
                    y += 26.0;
                }
                if is_host && s.state.phase != ArenaPhase::Playing {
                    let br = Rect::from_center_size(pos2(board.center().x, board.bottom() - 28.0), vec2(170.0, 36.0));
                    let r = ui.interact(br, EId::new("arena_start"), Sense::click());
                    painter.rect_filled(br, CornerRadius::same(10), if r.hovered() { p.accent.gamma_multiply(0.85) } else { p.accent });
                    let label = if s.state.phase == ArenaPhase::Over { "Play again" } else { "Start round" };
                    painter.text(br.center(), Align2::CENTER_CENTER, label, FontId::proportional(15.0), p.on_accent);
                    if r.clicked() {
                        start = true;
                    }
                }
            }
            if !captured {
                let hint = Rect::from_center_size(rect.center() + vec2(0.0, 90.0), vec2(560.0, 54.0));
                painter.rect_filled(hint, CornerRadius::same(12), Color32::from_black_alpha(170));
                painter.text(hint.center() - vec2(0.0, 9.0), Align2::CENTER_CENTER, "Click to play", FontId::proportional(17.0), Color32::WHITE);
                painter.text(
                    hint.center() + vec2(0.0, 12.0),
                    Align2::CENTER_CENTER,
                    "Mouse to look · W A S D to move · Space to jump · click to tag · Tab for scores · Esc to free the mouse",
                    FontId::proportional(11.5),
                    Color32::from_white_alpha(200),
                );
            }
            if host_gone {
                painter.text(c, Align2::CENTER_CENTER, "The host left the game.", FontId::proportional(22.0), Color32::WHITE);
            }
        });

        if want_capture {
            self.capture_mouse(&ctx);
        }
        if start {
            if let Some(s) = &mut self.arena {
                if let Some(m) = &mut s.m {
                    m.start();
                    s.last_match_send = -100.0;
                }
            }
        }
        if back {
            self.show_arena = false;
            self.release_mouse();
        }
        if leave {
            self.leave_arena();
        }
    }

    /// Test aid: a picture of the arena with a few made-up players.
    pub(super) fn preview_arena(&mut self) {
        self.host_arena();
        if let Some(s) = &mut self.arena {
            for (i, (pos, name)) in [([-6.0, 0.0, 6.0], "Mira"), ([3.0, 1.2, -2.0], "Juno"), ([-12.0, 0.0, -2.0], "Theo")].iter().enumerate() {
                let st = PlayerState {
                    id: 900 + i as u64,
                    name: name.to_string(),
                    hue: (i as u8 + 1) * 2,
                    pos: *pos,
                    yaw: 0.0,
                    pitch: 0.0,
                    hp: 100 - i as i16 * 25,
                    alive: true,
                    kos: 3 - i as u16,
                    downs: i as u16,
                    round: 1,
                };
                s.others.insert(900 + i as u64, Remote { prev_pos: st.pos, state: st, heard: 1e9, prev_heard: 1e9 - 0.05 });
            }
            s.body.pos = [-16.0, 0.0, 16.0];
            s.me.yaw = 0.75;
            s.me.pitch = -0.05;
            if let Some(m) = &mut s.m {
                m.start();
                s.state = m.state.clone();
            }
            s.me.round = s.state.round;
        }
    }
}
