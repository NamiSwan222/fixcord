//! The sign-in screen: choose an account, enter its PIN, or create a new one.

use super::*;

impl HearthApp {
    /// Re-read the accounts on this computer for the sign-in screen.
    pub(super) fn refresh_login(&mut self) {
        let (accounts, has_legacy) = match &self.store {
            Some(s) => (s.list(), s.legacy().is_some()),
            None => (vec![], false),
        };
        let first = accounts.is_empty();
        let legacy_name = if first && has_legacy { self.store.as_ref().and_then(|s| s.legacy()).map(|d| d.me().name.clone()) } else { None };
        self.login = LoginForm {
            mode: if first { LoginMode::Create } else { LoginMode::Choose },
            accounts,
            has_legacy: first && has_legacy,
            name: legacy_name.filter(|n| n != "You").unwrap_or_default(),
            hue: (self.rand() % 8) as u8,
            keep_signed_in: true,
            bring_over: true,
            ..Default::default()
        };
    }

    /// Start using an account's data.
    pub(super) fn sign_in(&mut self, data: Data, remember: bool) {
        let id = data.account.id;
        self.data = data;
        self.save_path = self.store.as_ref().map(|s| s.path_for(id));
        if let Some(s) = &self.store {
            s.set_remembered(if remember && self.data.account.pin_hash.is_none() { Some(id) } else { None });
        }
        self.signed_in = true;
        self.applied_theme = None;
        self.channel_for.clear();
        self.prepare_data();
        self.lan = None;
        self.lan_seen.clear();
        self.peers.clear();
        self.peer_uid.clear();
        if self.data.settings.lan_enabled && self.screenshot.is_none() {
            self.start_lan();
        }
        self.mark_read();
        self.view_changed_at = self.now;
        self.dirty = true;
    }

    /// Save, leave everything, and go back to the sign-in screen.
    pub(super) fn sign_out(&mut self) {
        self.save_now();
        self.leave_voice();
        if self.game.is_some() {
            self.leave_game();
        }
        self.lan = None;
        self.lan_seen.clear();
        self.peers.clear();
        self.peer_uid.clear();
        self.settings_open = false;
        self.save_path = None;
        if let Some(s) = &self.store {
            s.set_remembered(None);
        }
        self.signed_in = false;
        self.refresh_login();
        self.login.mode = LoginMode::Choose;
        self.view_changed_at = self.now;
    }

    fn create_account(&mut self) -> Result<(), String> {
        let name: String = self.login.name.trim().chars().take(32).collect();
        if name.is_empty() {
            return Err("Pick a display name.".into());
        }
        let username = accounts::check_username(&self.login.username, &self.login.accounts)?;
        let pin = self.login.pin.trim().to_string();
        if !pin.is_empty() {
            accounts::check_pin(&pin)?;
            if pin != self.login.pin2.trim() {
                return Err("The two PINs don't match.".into());
            }
        }
        let id = loop {
            let id = self.rand();
            if id != 0 && !self.login.accounts.iter().any(|a| a.id == id) {
                break id;
            }
        };
        let salt = self.rand();
        let store = self.store.as_ref().ok_or("Hearth can't find a place to save accounts.")?;
        let mut data = if self.login.has_legacy && self.login.bring_over {
            store.legacy().unwrap_or_else(|| sample(now_secs(), &name))
        } else {
            sample(now_secs(), &name)
        };
        data.account = Account {
            id,
            username,
            created: now_secs(),
            pin_hash: if pin.is_empty() { None } else { Some(accounts::hash_pin(&pin, salt)) },
            pin_salt: salt,
        };
        if let Some(me) = data.user_mut(ME) {
            me.name = name;
            me.hue = self.login.hue;
        }
        store.save(&data)?;
        let remember = self.login.keep_signed_in;
        self.sign_in(data, remember);
        Ok(())
    }

    pub(super) fn login_screen(&mut self, ui: &mut Ui, p: &Palette) {
        let t = self.ease(self.view_changed_at, 0.3);
        let mut choose: Option<u64> = None;
        let mut to_create = false;
        let mut to_choose = false;
        let mut submit_create = false;
        let mut submit_pin = false;
        egui::CentralPanel::default().frame(Frame::new().fill(p.rail)).show(ui, |ui| {
            ui.multiply_opacity(t);
            let area = ui.max_rect();
            let card = Rect::from_center_size(area.center() + vec2(0.0, (1.0 - t) * 20.0), vec2(440.0, (area.height() - 40.0).min(620.0)));
            ui.scope_builder(UiBuilder::new().max_rect(card), |ui| {
                Frame::new().fill(p.main).corner_radius(18.0).inner_margin(28.0).show(ui, |ui| {
                    ui.set_width(card.width() - 56.0);
                    ui.vertical_centered(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
                        paint_logo(ui, r, p.accent);
                        ui.add_space(6.0);
                        let title = match self.login.mode {
                            LoginMode::Create if self.login.accounts.is_empty() => "Welcome to Hearth",
                            LoginMode::Create => "Create an account",
                            _ => "Welcome back",
                        };
                        ui.label(RichText::new(title).size(24.0).color(p.strong).strong());
                    });
                    ui.add_space(10.0);
                    match self.login.mode {
                        LoginMode::Choose => {
                            ui.label(RichText::new("Choose your account").color(p.muted));
                            ui.add_space(6.0);
                            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                                for a in &self.login.accounts {
                                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 56.0), Sense::click());
                                    let k = hover_anim(ui, EId::new(("acct", a.id)), resp.hovered(), self.data.settings.reduced_motion);
                                    let bg = p.side.lerp_to_gamma(p.hover, k);
                                    ui.painter().rect_filled(rect, CornerRadius::same(10), bg);
                                    paint_avatar(ui, rect.left_center() + vec2(30.0, 0.0), 38.0, &a.name, a.hue, None, bg);
                                    ui.painter().text(rect.left_center() + vec2(58.0, -8.0), Align2::LEFT_CENTER, &a.name, FontId::proportional(16.0), p.strong);
                                    ui.painter().text(rect.left_center() + vec2(58.0, 10.0), Align2::LEFT_CENTER, format!("@{}", a.username), FontId::proportional(12.5), p.muted);
                                    if a.has_pin {
                                        ui.painter().text(rect.right_center() - vec2(16.0, 0.0), Align2::RIGHT_CENTER, "🔒", FontId::proportional(14.0), p.muted);
                                    }
                                    if resp.on_hover_text("Sign in").clicked() {
                                        choose = Some(a.id);
                                    }
                                    ui.add_space(6.0);
                                }
                            });
                            ui.add_space(10.0);
                            ui.checkbox(&mut self.login.keep_signed_in, "Keep me signed in on this computer");
                            ui.add_space(6.0);
                            if ui.add(egui::Button::new(RichText::new("+ Create a new account").color(p.accent)).frame(false)).clicked() {
                                to_create = true;
                            }
                        }
                        LoginMode::Pin(id) => {
                            let name = self.login.accounts.iter().find(|a| a.id == id).map(|a| a.name.clone()).unwrap_or_default();
                            ui.label(RichText::new(format!("Enter the PIN for {name}")).color(p.muted));
                            ui.add_space(6.0);
                            let r = ui.add(
                                egui::TextEdit::singleline(&mut self.login.pin)
                                    .password(true)
                                    .hint_text("PIN")
                                    .char_limit(8)
                                    .desired_width(f32::INFINITY)
                                    .background_color(p.raised),
                            );
                            if !r.has_focus() && self.login.pin.is_empty() {
                                r.request_focus();
                            }
                            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                                submit_pin = true;
                            }
                            ui.add_space(10.0);
                            ui.horizontal(|ui| {
                                if pill_button(ui, "Sign in", p.accent, p.on_accent).clicked() {
                                    submit_pin = true;
                                }
                                if ui.add(egui::Button::new(RichText::new("Back").color(p.muted)).frame(false)).clicked() {
                                    to_choose = true;
                                }
                            });
                        }
                        LoginMode::Create => {
                            ui.label(
                                RichText::new("Your account stays on this computer. Friends on your network always see the same you.")
                                    .color(p.muted),
                            );
                            ui.add_space(10.0);
                            section_label(ui, "Display name", p);
                            ui.add(egui::TextEdit::singleline(&mut self.login.name).hint_text("What friends see").char_limit(32).desired_width(f32::INFINITY).background_color(p.raised));
                            ui.add_space(6.0);
                            section_label(ui, "Username", p);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("@").color(p.muted));
                                ui.add(egui::TextEdit::singleline(&mut self.login.username).hint_text("letters, numbers, . _ -").char_limit(20).desired_width(f32::INFINITY).background_color(p.raised));
                            });
                            ui.add_space(6.0);
                            section_label(ui, "Colour", p);
                            ui.horizontal(|ui| {
                                for i in 0..8u8 {
                                    let (r, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
                                    ui.painter().circle_filled(r.center(), 10.0, theme::hue(i));
                                    if self.login.hue == i {
                                        ui.painter().circle_stroke(r.center(), 12.0, Stroke::new(2.0, p.strong));
                                    }
                                    if resp.clicked() {
                                        self.login.hue = i;
                                    }
                                }
                            });
                            ui.add_space(6.0);
                            section_label(ui, "PIN (optional)", p);
                            ui.horizontal(|ui| {
                                let w = (ui.available_width() - 8.0) / 2.0;
                                ui.add(egui::TextEdit::singleline(&mut self.login.pin).password(true).hint_text("4–8 digits").char_limit(8).desired_width(w).background_color(p.raised));
                                ui.add(egui::TextEdit::singleline(&mut self.login.pin2).password(true).hint_text("Again").char_limit(8).desired_width(w).background_color(p.raised));
                            });
                            ui.label(RichText::new("With a PIN, Hearth asks for it every time it starts.").size(11.5).color(p.faint));
                            ui.add_space(6.0);
                            if self.login.has_legacy {
                                ui.checkbox(&mut self.login.bring_over, "Bring over my messages, friends and settings from before");
                            }
                            ui.checkbox(&mut self.login.keep_signed_in, "Keep me signed in on this computer");
                            ui.add_space(10.0);
                            ui.horizontal(|ui| {
                                if pill_button(ui, "Create account", p.accent, p.on_accent).clicked() {
                                    submit_create = true;
                                }
                                if !self.login.accounts.is_empty()
                                    && ui.add(egui::Button::new(RichText::new("Back").color(p.muted)).frame(false)).clicked()
                                {
                                    to_choose = true;
                                }
                            });
                        }
                    }
                    if let Some(e) = &self.login.error {
                        ui.add_space(8.0);
                        ui.label(RichText::new(e).color(p.danger));
                    }
                });
            });
        });

        if to_create {
            self.login.mode = LoginMode::Create;
            self.login.error = None;
        }
        if to_choose {
            self.login.mode = LoginMode::Choose;
            self.login.pin.clear();
            self.login.error = None;
        }
        if let Some(id) = choose {
            let has_pin = self.login.accounts.iter().any(|a| a.id == id && a.has_pin);
            if has_pin {
                self.login.mode = LoginMode::Pin(id);
                self.login.pin.clear();
                self.login.error = None;
            } else {
                match self.store.as_ref().and_then(|s| s.load(id)) {
                    Some(d) => {
                        let remember = self.login.keep_signed_in;
                        self.sign_in(d, remember);
                    }
                    None => self.login.error = Some("That account couldn't be opened.".into()),
                }
            }
        }
        if submit_pin {
            if let LoginMode::Pin(id) = self.login.mode {
                match self.store.as_ref().and_then(|s| s.load(id)) {
                    Some(d) if accounts::pin_matches(&d, self.login.pin.trim()) => {
                        self.login.pin.clear();
                        self.sign_in(d, false);
                    }
                    Some(_) => {
                        self.login.error = Some("That PIN isn't right.".into());
                        self.login.pin.clear();
                    }
                    None => self.login.error = Some("That account couldn't be opened.".into()),
                }
            }
        }
        if submit_create {
            if let Err(e) = self.create_account() {
                self.login.error = Some(e);
            }
        }
    }
}
