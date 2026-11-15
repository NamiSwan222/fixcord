//! Hearth's own colour themes (original palette, warm "fireside" look).

use crate::model::{Status, Theme};
use egui::Color32;

#[derive(Clone, Copy)]
pub struct Palette {
    pub rail: Color32,
    pub side: Color32,
    pub main: Color32,
    pub raised: Color32,
    pub input: Color32,
    pub hover: Color32,
    pub selected: Color32,
    pub divider: Color32,
    pub text: Color32,
    pub strong: Color32,
    pub muted: Color32,
    pub faint: Color32,
    pub accent: Color32,
    pub on_accent: Color32,
    pub mention_bg: Color32,
    pub danger: Color32,
    pub dark: bool,
}

pub const ACCENTS: [(&str, Color32); 5] = [
    ("Ember", Color32::from_rgb(0xFF, 0x8A, 0x4C)),
    ("Lagoon", Color32::from_rgb(0x2E, 0xC4, 0xB6)),
    ("Rose", Color32::from_rgb(0xF2, 0x5F, 0x8C)),
    ("Moss", Color32::from_rgb(0x8F, 0xC9, 0x4A)),
    ("Sky", Color32::from_rgb(0x5A, 0xA9, 0xF2)),
];

pub const HUES: [Color32; 8] = [
    Color32::from_rgb(0xE0, 0x7A, 0x5F),
    Color32::from_rgb(0x3D, 0x9A, 0x8B),
    Color32::from_rgb(0x8E, 0x7C, 0xC3),
    Color32::from_rgb(0xD9, 0xA4, 0x2E),
    Color32::from_rgb(0x4F, 0x86, 0xE0),
    Color32::from_rgb(0xD0, 0x5E, 0x8A),
    Color32::from_rgb(0x5E, 0xAE, 0x4E),
    Color32::from_rgb(0xB8, 0x6E, 0x34),
];

pub fn hue(i: u8) -> Color32 {
    HUES[i as usize % HUES.len()]
}

pub fn status_color(s: Status) -> Color32 {
    match s {
        Status::Online => Color32::from_rgb(0x4C, 0xC3, 0x8A),
        Status::Away => Color32::from_rgb(0xF2, 0xB3, 0x3D),
        Status::Busy => Color32::from_rgb(0xEF, 0x5B, 0x5B),
        Status::Hidden => Color32::from_rgb(0x7D, 0x77, 0x87),
    }
}

pub fn palette(theme: Theme, accent: u8) -> Palette {
    let accent = ACCENTS[accent as usize % ACCENTS.len()].1;
    let a = |c: Color32, alpha: u8| Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), alpha);
    match theme {
        Theme::Ember => Palette {
            rail: Color32::from_rgb(0x14, 0x12, 0x18),
            side: Color32::from_rgb(0x1E, 0x1B, 0x23),
            main: Color32::from_rgb(0x27, 0x23, 0x2D),
            raised: Color32::from_rgb(0x18, 0x16, 0x1C),
            input: Color32::from_rgb(0x33, 0x2E, 0x3A),
            hover: Color32::from_rgb(0x2E, 0x29, 0x35),
            selected: Color32::from_rgb(0x3A, 0x33, 0x42),
            divider: Color32::from_rgb(0x15, 0x13, 0x19),
            text: Color32::from_rgb(0xDD, 0xD6, 0xE3),
            strong: Color32::from_rgb(0xF6, 0xF1, 0xFA),
            muted: Color32::from_rgb(0x9C, 0x94, 0xA8),
            faint: Color32::from_rgb(0x6E, 0x67, 0x78),
            accent,
            on_accent: Color32::from_rgb(0x1A, 0x12, 0x10),
            mention_bg: a(accent, 40),
            danger: Color32::from_rgb(0xEF, 0x5B, 0x5B),
            dark: true,
        },
        Theme::Midnight => Palette {
            rail: Color32::from_rgb(0x08, 0x0B, 0x12),
            side: Color32::from_rgb(0x0E, 0x13, 0x1C),
            main: Color32::from_rgb(0x13, 0x19, 0x24),
            raised: Color32::from_rgb(0x0A, 0x0E, 0x15),
            input: Color32::from_rgb(0x1C, 0x24, 0x31),
            hover: Color32::from_rgb(0x19, 0x20, 0x2C),
            selected: Color32::from_rgb(0x22, 0x2C, 0x3B),
            divider: Color32::from_rgb(0x07, 0x0A, 0x10),
            text: Color32::from_rgb(0xCF, 0xD8, 0xE6),
            strong: Color32::from_rgb(0xF0, 0xF5, 0xFC),
            muted: Color32::from_rgb(0x8A, 0x97, 0xAB),
            faint: Color32::from_rgb(0x5A, 0x66, 0x78),
            accent,
            on_accent: Color32::from_rgb(0x08, 0x0B, 0x12),
            mention_bg: a(accent, 36),
            danger: Color32::from_rgb(0xEF, 0x5B, 0x5B),
            dark: true,
        },
        Theme::Daylight => Palette {
            rail: Color32::from_rgb(0xE6, 0xDF, 0xD4),
            side: Color32::from_rgb(0xF1, 0xEB, 0xE2),
            main: Color32::from_rgb(0xFB, 0xF8, 0xF3),
            raised: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            input: Color32::from_rgb(0xEC, 0xE5, 0xDA),
            hover: Color32::from_rgb(0xE9, 0xE1, 0xD5),
            selected: Color32::from_rgb(0xDF, 0xD5, 0xC6),
            divider: Color32::from_rgb(0xDD, 0xD3, 0xC5),
            text: Color32::from_rgb(0x3A, 0x33, 0x2C),
            strong: Color32::from_rgb(0x1E, 0x19, 0x14),
            muted: Color32::from_rgb(0x7A, 0x6F, 0x63),
            faint: Color32::from_rgb(0xA3, 0x98, 0x8B),
            accent,
            on_accent: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            mention_bg: a(accent, 46),
            danger: Color32::from_rgb(0xD6, 0x45, 0x45),
            dark: false,
        },
    }
}

pub fn apply(ctx: &egui::Context, p: &Palette, scale: f32) {
    let mut v = if p.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = p.main;
    v.window_fill = p.raised;
    v.extreme_bg_color = p.input;
    v.faint_bg_color = p.hover;
    v.override_text_color = Some(p.text);
    v.selection.bg_fill = Color32::from_rgba_unmultiplied(p.accent.r(), p.accent.g(), p.accent.b(), 90);
    v.selection.stroke = egui::Stroke::new(1.0, p.accent);
    v.hyperlink_color = p.accent;
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, p.divider);
    v.widgets.inactive.weak_bg_fill = p.input;
    v.widgets.inactive.bg_fill = p.input;
    v.widgets.hovered.weak_bg_fill = p.selected;
    v.widgets.hovered.bg_fill = p.selected;
    v.widgets.active.weak_bg_fill = p.selected;
    v.window_stroke = egui::Stroke::new(1.0, p.divider);
    ctx.set_visuals(v);
    ctx.set_zoom_factor(scale);
}
