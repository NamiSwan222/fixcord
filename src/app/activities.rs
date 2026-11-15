//! Activities: games played with friends on the home network (Snake for now).

use super::*;

/// A Snake game you're hosting or playing.
pub(super) struct GameSession {
    id: u64,
    host_instance: u64,
    host_name: String,
    /// Only on the host: the game itself.
    game: Option<snake::Game>,
    board: Option<Board>,
    /// The board one move earlier, to slide snakes smoothly between moves.
    prev: Option<Board>,
    board_at: f64,
    last_frame: f64,
    last_step: f64,
    last_send: f64,
    last_join: f64,
    last_heard: f64,
    last_phase: Option<SnakePhase>,
    was_alive: bool,
}

/// A game someone on the home network is hosting.
pub(super) struct GameInvite {
    host_instance: u64,
    host_name: String,
    players: usize,
    phase: SnakePhase,
    seen: f64,
}

/// Turns a board position (cell, possibly between two cells) into a point on screen.
fn cell_center(origin: Pos2, cell: f32, x: f32, y: f32) -> Pos2 {
    pos2(origin.x + (x + 0.5) * cell, origin.y + (y + 0.5) * cell)
}

fn new_session(id: u64, host_instance: u64, host_name: String, game: Option<snake::Game>, now: f64) -> GameSession {
    let board = game.as_ref().map(|g| g.board());
    GameSession {
        id,
        host_instance,
        host_name,
        game,
        board,
        prev: None,
        board_at: now,
        last_frame: now,
        last_step: now,
        last_send: -100.0,
        last_join: -100.0,
        last_heard: now,
        last_phase: None,
        was_alive: false,
    }
}

impl HearthApp {
    pub(super) fn lan_send(&self, p: Packet) {
        if let Some(l) = &self.lan {
            l.send(&p);
        }
    }

    pub(super) fn host_snake(&mut self) {
        let id = self.rand() | 1;
        let seed = self.rand();
        let me = self.data.me().clone();
        let mut g = snake::Game::new(seed);
        g.add_player(self.instance, &me.name, me.hue);
        self.game = Some(new_session(id, self.instance, me.name.clone(), Some(g), self.now));
        self.show_game = true;
        self.activities_open = false;
        self.sfx(Sfx::Join);
    }

    /// Test aid (picture of a round in progress): start the hosted round and play a few moves.
    pub(super) fn preview_round(&mut self) {
        if let Some(s) = &mut self.game {
            if let Some(g) = &mut s.game {
                g.start();
                g.update_countdown(snake::COUNTDOWN + 0.1);
                for _ in 0..5 {
                    g.step();
                }
                s.board = Some(g.board());
            }
        }
    }

    pub(super) fn join_snake(&mut self, game_id: u64) {
        let Some(inv) = self.game_invites.get(&game_id) else { return };
        self.game = Some(new_session(game_id, inv.host_instance, inv.host_name.clone(), None, self.now));
        self.show_game = true;
        self.activities_open = false;
        self.sfx(Sfx::Join);
    }

    pub(super) fn leave_game(&mut self) {
        if let Some(s) = self.game.take() {
            if s.game.is_none() {
                self.lan_send(Packet::SnakeLeave { from: self.instance, game: s.id });
            }
            self.sfx(Sfx::Leave);
        }
        self.show_game = false;
    }

    fn steer_snake(&mut self, d: SnakeDir) {
        let instance = self.instance;
        let Some(s) = &mut self.game else { return };
        if let Some(g) = &mut s.game {
            g.steer(instance, d);
        } else {
            let id = s.id;
            self.lan_send(Packet::SnakeInput { from: instance, game: id, dir: d });
        }
    }

    /// Runs the game (on the host), keeps asking to join (players), and plays its sounds.
    pub(super) fn tick_game(&mut self, now: f64) {
        self.game_invites.retain(|_, i| now - i.seen < 5.0);
        let me = self.data.me().clone();
        let instance = self.instance;
        let mut out: Vec<Packet> = vec![];
        let mut sound: Option<Sfx> = None;
        let Some(s) = &mut self.game else { return };
        if let Some(g) = &mut s.game {
            // Host: move the game on.
            g.update_countdown((now - s.last_frame).clamp(0.0, 0.5));
            s.last_frame = now;
            let interval = 1.0 / snake::TICKS_PER_SEC;
            let mut stepped = false;
            if g.phase == SnakePhase::Playing {
                let mut n = 0;
                while now - s.last_step >= interval && n < 3 {
                    g.step();
                    s.last_step += interval;
                    stepped = true;
                    n += 1;
                    if g.phase != SnakePhase::Playing {
                        break;
                    }
                }
                if now - s.last_step > 1.0 {
                    s.last_step = now; // after a long pause, don't race to catch up
                }
            } else {
                s.last_step = now;
            }
            let every = if g.phase == SnakePhase::Countdown { 0.2 } else { 1.0 };
            if stepped || (g.phase != SnakePhase::Playing && now - s.last_send >= every) {
                let b = g.board();
                if stepped {
                    s.prev = s.board.take();
                    s.board_at = now;
                }
                s.board = Some(b.clone());
                s.last_send = now;
                out.push(Packet::SnakeBoard { from: instance, game: s.id, host: s.host_name.clone(), board: b });
            }
            s.last_heard = now;
        } else {
            // Player: ask to join until the host has us on the board.
            let on_board = s.board.as_ref().map(|b| b.snakes.iter().any(|x| x.id == instance)).unwrap_or(false);
            if !on_board && now - s.last_join > 1.0 {
                out.push(Packet::SnakeJoin { from: instance, game: s.id, name: me.name.clone(), hue: me.hue });
                s.last_join = now;
            }
        }
        // Sounds for what changed.
        if let Some(b) = &s.board {
            let alive = b.snakes.iter().any(|x| x.id == instance && x.alive);
            if s.last_phase != Some(b.phase) {
                match b.phase {
                    SnakePhase::Playing => sound = Some(Sfx::LiveStart),
                    SnakePhase::Over if b.winner == Some(instance) => sound = Some(Sfx::Mention),
                    SnakePhase::Over => sound = Some(Sfx::LiveStop),
                    _ => {}
                }
                s.last_phase = Some(b.phase);
            } else if s.was_alive && !alive && b.phase == SnakePhase::Playing {
                sound = Some(Sfx::Leave);
            }
            s.was_alive = alive;
        }
        for p in out {
            self.lan_send(p);
        }
        if let Some(snd) = sound {
            self.sfx(snd);
        }
    }

    pub(super) fn on_game_packet(&mut self, p: Packet, now: f64) {
        match p {
            Packet::SnakeBoard { from, game, host, board } => {
                let too_big = board.snakes.len() > snake::MAX_PLAYERS
                    || board.snakes.iter().any(|s| s.body.len() > (snake::W as usize) * (snake::H as usize) * 2);
                if too_big {
                    return; // not something a Hearth host sends
                }
                self.game_invites.insert(
                    game,
                    GameInvite {
                        host_instance: from,
                        host_name: host.chars().take(32).collect(),
                        players: board.snakes.len(),
                        phase: board.phase,
                        seen: now,
                    },
                );
                if let Some(s) = &mut self.game {
                    if s.id == game && s.game.is_none() && from == s.host_instance {
                        let new_move = s.board.as_ref().map(|b| b.tick != board.tick).unwrap_or(true);
                        if new_move {
                            s.prev = s.board.take();
                            s.board_at = now;
                        }
                        s.board = Some(board);
                        s.last_heard = now;
                    }
                }
            }
            Packet::SnakeJoin { from, game, name, hue } => {
                if let Some(s) = &mut self.game {
                    if s.id == game {
                        if let Some(g) = &mut s.game {
                            if !g.has_player(from) && g.add_player(from, &name, hue % 8) {
                                s.last_send = -100.0; // show them on the board right away
                            }
                        }
                    }
                }
            }
            Packet::SnakeInput { from, game, dir } => {
                if let Some(s) = &mut self.game {
                    if s.id == game {
                        if let Some(g) = &mut s.game {
                            g.steer(from, dir);
                        }
                    }
                }
            }
            Packet::SnakeLeave { from, game } => {
                if let Some(s) = &mut self.game {
                    if s.id == game {
                        if let Some(g) = &mut s.game {
                            g.remove_player(from);
                            s.last_send = -100.0;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// The Activities picker: start a game, or join one on the network.
    pub(super) fn activities_dialog(&mut self, ctx: &egui::Context, p: &Palette) {
        if !self.activities_open {
            return;
        }
        let pop_t = self.ease(self.popup_opened_at, 0.2);
        let mut close = false;
        let mut host = false;
        let mut join: Option<u64> = None;
        let lan_on = self.lan.is_some();
        let mut invites: Vec<(u64, String, usize, SnakePhase)> =
            self.game_invites.iter().map(|(id, i)| (*id, i.host_name.clone(), i.players, i.phase)).collect();
        invites.sort_by(|a, b| a.1.cmp(&b.1));
        let modal = egui::Modal::new(EId::new("activities"))
            .backdrop_color(Color32::from_black_alpha((150.0 * pop_t) as u8))
            .frame(Frame::new().fill(p.main).corner_radius(14.0).inner_margin(20.0))
            .show(ctx, |ui| {
                ui.multiply_opacity(pop_t);
                ui.set_width(440.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Activities").size(20.0).color(p.strong).strong());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if icon_button(ui, "×", "Close", false, p).clicked() {
                            close = true;
                        }
                    });
                });
                ui.label(RichText::new("Play together with friends on your home network.").color(p.muted));
                ui.add_space(12.0);
                Frame::new().fill(p.side).corner_radius(12.0).inner_margin(14.0).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
                        ui.painter().rect_filled(r, CornerRadius::same(14), theme::hue(6));
                        // A little snake drawn from circles.
                        let pts = [(-14.0, 8.0), (-7.0, 8.0), (0.0, 8.0), (0.0, 1.0), (0.0, -6.0), (7.0, -6.0), (14.0, -6.0)];
                        for (k, (x, y)) in pts.iter().enumerate() {
                            let rad = if k == pts.len() - 1 { 5.5 } else { 4.5 };
                            ui.painter().circle_filled(r.center() + vec2(*x, *y), rad, Color32::from_rgb(0xF4, 0xFF, 0xE8));
                        }
                        ui.painter().circle_filled(r.center() + vec2(16.0, -8.0), 1.6, Color32::BLACK);
                        ui.add_space(6.0);
                        ui.vertical(|ui| {
                            ui.label(RichText::new("Snake").size(17.0).color(p.strong).strong());
                            ui.label(
                                RichText::new("1–8 players · Eat to grow, don't crash. Last snake moving wins.").size(12.5).color(p.muted),
                            );
                            ui.add_space(4.0);
                            if lan_on {
                                if pill_button(ui, "Start a game", p.accent, p.on_accent).clicked() {
                                    host = true;
                                }
                            } else {
                                ui.label(
                                    RichText::new("Turn on Home network chat in Settings to play with friends.").size(12.0).color(p.danger),
                                );
                            }
                        });
                    });
                });
                ui.add_space(14.0);
                section_label(ui, "Games on your network", p);
                if invites.is_empty() {
                    ui.label(RichText::new("Nobody is playing right now. Start one and friends get an invite.").size(12.5).color(p.faint));
                }
                for (id, name, players, phase) in &invites {
                    ui.horizontal(|ui| {
                        let state = match phase {
                            SnakePhase::Lobby => "waiting to start",
                            SnakePhase::Countdown | SnakePhase::Playing => "playing a round",
                            SnakePhase::Over => "between rounds",
                        };
                        ui.label(RichText::new(format!("Snake with {name}")).color(p.strong));
                        ui.label(RichText::new(format!("{players}/{} · {state}", snake::MAX_PLAYERS)).size(12.0).color(p.muted));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if pill_button(ui, "Join", p.accent, p.on_accent).clicked() {
                                join = Some(*id);
                            }
                        });
                    });
                }
            });
        if modal.should_close() {
            close = true;
        }
        if close {
            self.activities_open = false;
        }
        if host {
            if self.game.is_some() {
                self.leave_game();
            }
            self.host_snake();
        }
        if let Some(id) = join {
            if self.game.is_some() {
                self.leave_game();
            }
            self.join_snake(id);
        }
    }

    /// "Mira is playing Snake — Join" in the corner, or "Back to your game".
    pub(super) fn game_toast(&mut self, ctx: &egui::Context, p: &Palette) {
        enum Toast {
            Invite(u64, String, usize, SnakePhase),
            Back,
        }
        let toast = if self.game.is_some() {
            if self.show_game { None } else { Some(Toast::Back) }
        } else {
            self.game_invites
                .iter()
                .filter(|(id, _)| !self.hidden_invites.contains(id))
                .map(|(id, i)| Toast::Invite(*id, i.host_name.clone(), i.players, i.phase))
                .next()
        };
        let Some(toast) = toast else { return };
        let screen = ctx.content_rect();
        let mut join: Option<u64> = None;
        let mut hide: Option<u64> = None;
        let mut open = false;
        egui::Area::new(EId::new("game_toast"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2(screen.right() - 360.0, screen.bottom() - 150.0))
            .show(ctx, |ui| {
                Frame::new()
                    .fill(p.raised)
                    .corner_radius(12.0)
                    .stroke(Stroke::new(1.0, p.accent))
                    .inner_margin(12.0)
                    .shadow(egui::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(90) })
                    .show(ui, |ui| {
                        ui.set_width(320.0);
                        match &toast {
                            Toast::Invite(id, name, players, phase) => {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("🎮").size(18.0));
                                    ui.vertical(|ui| {
                                        ui.label(RichText::new(format!("{name} is playing Snake")).color(p.strong).strong());
                                        let more = if matches!(phase, SnakePhase::Playing | SnakePhase::Countdown) {
                                            " · you'll join the next round"
                                        } else {
                                            ""
                                        };
                                        ui.label(
                                            RichText::new(format!("{players}/{} players{more}", snake::MAX_PLAYERS)).size(12.0).color(p.muted),
                                        );
                                    });
                                });
                                ui.horizontal(|ui| {
                                    if pill_button(ui, "Join", p.accent, p.on_accent).clicked() {
                                        join = Some(*id);
                                    }
                                    if ui.add(egui::Button::new(RichText::new("Not now").color(p.muted)).frame(false)).clicked() {
                                        hide = Some(*id);
                                    }
                                });
                            }
                            Toast::Back => {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("🎮").size(18.0));
                                    ui.label(RichText::new("You're in a Snake game").color(p.strong).strong());
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if pill_button(ui, "Open", p.accent, p.on_accent).clicked() {
                                            open = true;
                                        }
                                    });
                                });
                            }
                        }
                    });
            });
        if let Some(id) = join {
            self.join_snake(id);
        }
        if let Some(id) = hide {
            self.hidden_invites.insert(id);
        }
        if open {
            self.show_game = true;
        }
    }

    /// The game screen.
    pub(super) fn game_view(&mut self, ui: &mut Ui, p: &Palette, now: f64) {
        let ctx = ui.ctx().clone();
        // Keys: arrows or W A S D (only while the game is on screen).
        let pressed: Option<SnakeDir> = ctx.input(|i| {
            if i.key_pressed(Key::ArrowUp) || i.key_pressed(Key::W) {
                Some(SnakeDir::Up)
            } else if i.key_pressed(Key::ArrowDown) || i.key_pressed(Key::S) {
                Some(SnakeDir::Down)
            } else if i.key_pressed(Key::ArrowLeft) || i.key_pressed(Key::A) {
                Some(SnakeDir::Left)
            } else if i.key_pressed(Key::ArrowRight) || i.key_pressed(Key::D) {
                Some(SnakeDir::Right)
            } else {
                None
            }
        });
        if let Some(d) = pressed {
            self.steer_snake(d);
        }
        let instance = self.instance;
        let reduced = self.data.settings.reduced_motion;
        let view_t = self.ease(self.view_changed_at, 0.25);
        let Some(s) = &self.game else { return };
        let is_host = s.game.is_some();
        let host_name = s.host_name.clone();
        let board = s.board.clone();
        let prev = s.prev.clone();
        let since = now - s.board_at;
        let host_gone = !is_host && now - s.last_heard > 6.0;
        if board.as_ref().map(|b| matches!(b.phase, SnakePhase::Playing | SnakePhase::Countdown)).unwrap_or(false) {
            ctx.request_repaint(); // smooth sliding between moves
        }

        let mut leave = false;
        let mut back = false;
        let mut start = false;
        egui::Panel::top("game_header")
            .exact_size(HEADER_H)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::new().fill(p.rail).inner_margin(Margin::symmetric(14, 0)))
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(RichText::new("🎮").size(16.0));
                    ui.label(RichText::new("Snake").size(16.0).color(p.strong).strong());
                    let who = if is_host { "You're hosting".to_string() } else { format!("Hosted by {host_name}") };
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

        // Scoreboard
        let mut players: Vec<snake::SnakeView> = board.as_ref().map(|b| b.snakes.clone()).unwrap_or_default();
        players.sort_by(|a, b| b.score.cmp(&a.score).then(a.name.cmp(&b.name)));
        let playing = board.as_ref().map(|b| b.phase == SnakePhase::Playing).unwrap_or(false);
        egui::Panel::right("game_scores")
            .exact_size(MEMBERS_W)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::new().fill(p.side).inner_margin(Margin { left: 14, right: 14, top: 16, bottom: 12 }))
            .show(ui, |ui| {
                section_label(ui, &format!("Players — {}/{}", players.len(), snake::MAX_PLAYERS), p);
                ui.add_space(6.0);
                for pl in &players {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                        let col = if pl.alive || !playing { theme::hue(pl.hue) } else { p.faint };
                        ui.painter().circle_filled(r.center(), 6.0, col);
                        let name = if pl.id == instance { format!("{} (you)", pl.name) } else { pl.name.clone() };
                        ui.label(RichText::new(name).color(if pl.alive || !playing { p.strong } else { p.muted }));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(pl.score.to_string()).color(p.accent).strong());
                            if !pl.alive && playing {
                                ui.label(RichText::new("out").size(11.0).color(p.faint));
                            }
                        });
                    });
                }
                ui.add_space(16.0);
                ui.label(RichText::new("Steer with the arrow keys or W A S D.").size(12.0).color(p.faint));
                ui.label(RichText::new("Eat the glowing dots to grow. Hitting a wall or any snake puts you out.").size(12.0).color(p.faint));
            });

        egui::CentralPanel::default().frame(Frame::new().fill(p.rail).inner_margin(16.0)).show(ui, |ui| {
            ui.multiply_opacity(view_t);
            let area = ui.max_rect();
            let cell = ((area.width() / snake::W as f32).min(area.height() / snake::H as f32)).floor().max(4.0);
            let size = vec2(cell * snake::W as f32, cell * snake::H as f32);
            let rect = Rect::from_center_size(area.center(), size);
            let painter = ui.painter_at(area);
            painter.rect_filled(rect.expand(4.0), CornerRadius::same(12), p.side);
            // Soft checkerboard
            let alt = p.side.lerp_to_gamma(p.input, 0.35);
            for y in 0..snake::H {
                for x in 0..snake::W {
                    if (x + y) % 2 == 0 {
                        let r = Rect::from_min_size(rect.min + vec2(x as f32 * cell, y as f32 * cell), vec2(cell, cell));
                        painter.rect_filled(r, CornerRadius::ZERO, alt);
                    }
                }
            }
            let Some(b) = &board else {
                let msg = if host_gone { "The host left the game." } else { "Joining…" };
                painter.text(rect.center(), Align2::CENTER_CENTER, msg, FontId::proportional(20.0), p.muted);
                return;
            };
            // Food, gently pulsing.
            let pulse = if reduced { 0.0 } else { ((now * 4.0).sin() as f32) * 0.05 };
            for f in b.food.chunks_exact(2) {
                let c = cell_center(rect.min, cell, f[0] as f32, f[1] as f32);
                painter.circle_filled(c, cell * (0.42 + pulse), p.accent.gamma_multiply(0.25));
                painter.circle_filled(c, cell * 0.28, p.accent);
                painter.circle_filled(c + vec2(-cell * 0.08, -cell * 0.08), cell * 0.08, Color32::from_white_alpha(160));
            }
            // Slide between the previous move and this one.
            let t = if b.phase == SnakePhase::Playing && !reduced { ((since * snake::TICKS_PER_SEC) as f32).clamp(0.0, 1.0) } else { 1.0 };
            for sn in &b.snakes {
                if sn.body.len() < 4 {
                    continue;
                }
                let old = prev
                    .as_ref()
                    .filter(|pb| pb.tick + 1 == b.tick)
                    .and_then(|pb| pb.snakes.iter().find(|o| o.id == sn.id))
                    .map(|o| o.body.clone());
                let pts: Vec<Pos2> = sn
                    .body
                    .chunks_exact(2)
                    .enumerate()
                    .map(|(i, c)| {
                        let (cx, cy) = (c[0] as f32, c[1] as f32);
                        let (ox, oy) = match &old {
                            Some(o) if sn.alive && o.len() >= 2 => {
                                let j = (i * 2).min(o.len() - 2);
                                (o[j] as f32, o[j + 1] as f32)
                            }
                            _ => (cx, cy),
                        };
                        // Only slide between neighbouring cells.
                        let (x, y) =
                            if (ox - cx).abs() + (oy - cy).abs() <= 1.0 { (ox + (cx - ox) * t, oy + (cy - oy) * t) } else { (cx, cy) };
                        cell_center(rect.min, cell, x, y)
                    })
                    .collect();
                let base = theme::hue(sn.hue);
                let col = if sn.alive { base } else { base.gamma_multiply(0.3) };
                let dark = col.lerp_to_gamma(Color32::BLACK, 0.25);
                for w in pts.windows(2) {
                    painter.line_segment([w[0], w[1]], Stroke::new(cell * 0.74, col));
                }
                for (i, pt) in pts.iter().enumerate().rev() {
                    let rad = if i == 0 { cell * 0.46 } else { cell * 0.37 };
                    painter.circle_filled(*pt, rad, if i % 2 == 0 { col } else { dark.lerp_to_gamma(col, 0.6) });
                }
                // Eyes look the way the snake is going.
                let head = pts[0];
                let d = pts[0] - pts[1];
                let dir = if d.length() > 0.01 { d.normalized() } else { vec2(1.0, 0.0) };
                let side = vec2(-dir.y, dir.x);
                for k in [-1.0f32, 1.0] {
                    let e = head + dir * cell * 0.15 + side * cell * 0.2 * k;
                    painter.circle_filled(e, cell * 0.13, Color32::WHITE);
                    painter.circle_filled(e + dir * cell * 0.04, cell * 0.065, Color32::from_rgb(20, 20, 30));
                }
                if sn.alive {
                    let label = if sn.id == instance { "you".to_string() } else { sn.name.clone() };
                    painter.text(
                        head - vec2(0.0, cell * 0.85),
                        Align2::CENTER_BOTTOM,
                        label,
                        FontId::proportional((cell * 0.55).clamp(10.0, 14.0)),
                        Color32::from_white_alpha(220),
                    );
                }
            }
            // Messages over the board.
            let card = |lines: &[(&str, f32, Color32)]| {
                let h = lines.iter().map(|l| l.1 + 10.0).sum::<f32>() + 24.0;
                let r = Rect::from_center_size(rect.center(), vec2(360.0, h));
                painter.rect_filled(r, CornerRadius::same(14), p.raised.gamma_multiply(0.94));
                let mut y = r.top() + 16.0;
                for (text, size, color) in lines {
                    painter.text(pos2(r.center().x, y), Align2::CENTER_TOP, *text, FontId::proportional(*size), *color);
                    y += size + 10.0;
                }
                r
            };
            let me_alive = b.snakes.iter().any(|x| x.id == instance && x.alive);
            let on_board = b.snakes.iter().any(|x| x.id == instance);
            let mut button: Option<(Rect, &str)> = None;
            match b.phase {
                SnakePhase::Lobby => {
                    let waiting = format!("Players: {} of {}", b.snakes.len(), snake::MAX_PLAYERS);
                    let who = if is_host { "Friends on your network see an invite." } else { "Waiting for the host to start…" };
                    let r = card(&[("Snake", 26.0, p.strong), (&waiting, 15.0, p.text), (who, 13.0, p.muted)]);
                    if is_host {
                        button = Some((Rect::from_center_size(pos2(r.center().x, r.bottom() + 28.0), vec2(160.0, 38.0)), "Start round"));
                    }
                }
                SnakePhase::Countdown => {
                    let n = b.countdown.ceil().max(1.0) as u32;
                    painter.text(rect.center(), Align2::CENTER_CENTER, n.to_string(), FontId::proportional(cell * 5.0), Color32::from_white_alpha(230));
                }
                SnakePhase::Playing => {
                    let note = if on_board && !me_alive {
                        Some("You're out — watch the others finish")
                    } else if !on_board {
                        Some("Round in progress — you'll join the next one")
                    } else {
                        None
                    };
                    if let Some(n) = note {
                        painter.text(rect.center_top() + vec2(0.0, 20.0), Align2::CENTER_TOP, n, FontId::proportional(15.0), Color32::from_white_alpha(220));
                    }
                }
                SnakePhase::Over => {
                    let title = if b.snakes.len() == 1 {
                        format!("Game over! Score {}", b.snakes[0].score)
                    } else {
                        match b.winner.and_then(|w| b.snakes.iter().find(|x| x.id == w)) {
                            Some(w) if w.id == instance => "You win! 🎉".to_string(),
                            Some(w) => format!("{} wins!", w.name),
                            None => "It's a draw!".to_string(),
                        }
                    };
                    let sub = if is_host { "Play again whenever you're ready." } else { "Waiting for the host to start the next round…" };
                    let r = card(&[(&title, 24.0, p.strong), (sub, 13.0, p.muted)]);
                    if is_host {
                        button = Some((Rect::from_center_size(pos2(r.center().x, r.bottom() + 28.0), vec2(160.0, 38.0)), "Play again"));
                    }
                }
            }
            if let Some((br, label)) = button {
                let resp = ui.interact(br, EId::new("snake_button"), Sense::click());
                painter.rect_filled(br, CornerRadius::same(10), if resp.hovered() { p.accent.gamma_multiply(0.85) } else { p.accent });
                painter.text(br.center(), Align2::CENTER_CENTER, label, FontId::proportional(15.0), p.on_accent);
                if resp.clicked() {
                    start = true;
                }
            }
            if host_gone {
                card(&[("The host left the game.", 20.0, p.strong), ("Press Leave game to go back.", 13.0, p.muted)]);
            }
        });

        if start {
            if let Some(s) = &mut self.game {
                if let Some(g) = &mut s.game {
                    g.start();
                    s.last_send = -100.0;
                    s.prev = None;
                }
            }
        }
        if back {
            self.show_game = false;
        }
        if leave {
            self.leave_game();
        }
    }
}
