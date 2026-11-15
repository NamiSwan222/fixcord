//! "Arena": a 3D first-person game for friends on the home network.
//!
//! Everyone runs around an original arena (the Yard) and tags others with a blaster. Each copy of
//! Hearth moves its own player and tells the others where it is; a shot says who it hit, and the
//! one who was hit takes the damage and says when they're knocked out. The host only keeps the
//! match: when it starts, the time left and who won. This file holds the world, the movement and
//! the hit tests — no drawing and no networking — so they can be tested on their own.

use serde::{Deserialize, Serialize};

pub type V3 = [f32; 3];

/// Half the arena's width (metres): the floor goes from -SIZE to +SIZE.
pub const SIZE: f32 = 24.0;
pub const WALL_H: f32 = 4.0;
pub const EYE: f32 = 1.6;
pub const HEIGHT: f32 = 1.8;
pub const RADIUS: f32 = 0.4;
/// Head starts this high above the feet (for headshots).
pub const HEAD: f32 = 1.45;
pub const SPEED: f32 = 6.5;
pub const JUMP: f32 = 6.2;
pub const GRAVITY: f32 = 18.0;
/// How high a step you can walk up without jumping.
pub const STEP: f32 = 0.45;
pub const FIRE_EVERY: f32 = 0.25;
pub const DAMAGE: i16 = 25;
pub const HEADSHOT: i16 = 50;
pub const RANGE: f32 = 70.0;
pub const RESPAWN: f32 = 2.5;
pub const TARGET_KOS: u16 = 15;
pub const MATCH_SECS: f32 = 300.0;
/// Position updates per second sent by each player.
pub const SEND_HZ: f32 = 20.0;
pub const MAX_PLAYERS: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Phase {
    Lobby,
    Playing,
    Over,
}

/// The host's view of the match.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MatchState {
    pub phase: Phase,
    pub round: u32,
    pub time_left: f32,
    pub target: u16,
    /// Winner of the last round (0 = nobody yet / a draw).
    pub winner: u64,
    /// How many are playing (for invites).
    #[serde(default)]
    pub players: u8,
}

/// One player, as they report themselves.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayerState {
    pub id: u64,
    pub name: String,
    pub hue: u8,
    pub pos: V3,
    pub yaw: f32,
    pub pitch: f32,
    pub hp: i16,
    pub alive: bool,
    pub kos: u16,
    pub downs: u16,
    pub round: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shot {
    pub from: V3,
    pub to: V3,
    pub hit: u64,
    pub dmg: i16,
}

/// A solid box in the world.
#[derive(Clone, Copy, Debug)]
pub struct Block {
    pub min: V3,
    pub max: V3,
    /// Colour index (0 crate, 1 stone, 2 platform, 3 wall).
    pub kind: u8,
}

fn b(min: V3, max: V3, kind: u8) -> Block {
    Block { min, max, kind }
}

/// The Yard: crates, pillars, low walls and a raised middle with steps.
pub fn world() -> Vec<Block> {
    let s = SIZE;
    let mut w = vec![
        // Outer walls
        b([-s - 1.0, 0.0, -s - 1.0], [s + 1.0, WALL_H, -s], 3),
        b([-s - 1.0, 0.0, s], [s + 1.0, WALL_H, s + 1.0], 3),
        b([-s - 1.0, 0.0, -s], [-s, WALL_H, s], 3),
        b([s, 0.0, -s], [s + 1.0, WALL_H, s], 3),
        // Raised middle with a step on each side
        b([-4.0, 0.0, -4.0], [4.0, 1.2, 4.0], 2),
        b([-2.0, 0.0, 4.0], [2.0, 0.4, 5.2], 2),
        b([-2.0, 0.4, 4.0], [2.0, 0.8, 4.6], 2),
        b([-2.0, 0.0, -5.2], [2.0, 0.4, -4.0], 2),
        b([-2.0, 0.4, -4.6], [2.0, 0.8, -4.0], 2),
        b([4.0, 0.0, -2.0], [5.2, 0.4, 2.0], 2),
        b([4.0, 0.4, -2.0], [4.6, 0.8, 2.0], 2),
        b([-5.2, 0.0, -2.0], [-4.0, 0.4, 2.0], 2),
        b([-4.6, 0.4, -2.0], [-4.0, 0.8, 2.0], 2),
        // Pillars
        b([-12.5, 0.0, -12.5], [-11.0, 3.2, -11.0], 1),
        b([11.0, 0.0, -12.5], [12.5, 3.2, -11.0], 1),
        b([-12.5, 0.0, 11.0], [-11.0, 3.2, 12.5], 1),
        b([11.0, 0.0, 11.0], [12.5, 3.2, 12.5], 1),
        // Low walls to duck behind
        b([-18.0, 0.0, -1.0], [-14.0, 1.1, 0.0], 1),
        b([14.0, 0.0, 0.0], [18.0, 1.1, 1.0], 1),
        b([-1.0, 0.0, 14.0], [0.0, 1.1, 18.0], 1),
        b([0.0, 0.0, -18.0], [1.0, 1.1, -14.0], 1),
    ];
    // Crates
    for (x, z, h) in [(-7.0, 10.0, 1.0), (-6.0, 11.0, 1.0), (7.0, -10.0, 1.0), (8.0, -9.0, 2.0), (17.0, 15.0, 1.0), (-17.0, -15.0, 1.0), (16.0, -17.0, 2.0), (-16.0, 17.0, 2.0), (9.0, 8.0, 1.0), (-9.0, -8.0, 1.0)] {
        w.push(b([x - 0.5, 0.0, z - 0.5], [x + 0.5, h, z + 0.5], 0));
    }
    w
}

/// Places to (re)appear, around the edges.
pub fn spawns() -> Vec<V3> {
    let e = SIZE - 3.0;
    vec![
        [-e, 0.0, -e],
        [e, 0.0, e],
        [e, 0.0, -e],
        [-e, 0.0, e],
        [0.0, 0.0, -e],
        [0.0, 0.0, e],
        [-e, 0.0, 0.0],
        [e, 0.0, 0.0],
    ]
}

/// The spawn point farthest from everyone else (so you don't appear next to someone).
pub fn pick_spawn(others: &[V3], salt: u64) -> V3 {
    let list = spawns();
    if others.is_empty() {
        return list[(salt % list.len() as u64) as usize];
    }
    *list
        .iter()
        .max_by(|a, b| {
            let d = |p: &V3| others.iter().map(|o| (p[0] - o[0]).powi(2) + (p[2] - o[2]).powi(2)).fold(f32::MAX, f32::min);
            d(a).partial_cmp(&d(b)).unwrap()
        })
        .unwrap()
}

/// The direction you're looking.
pub fn look_dir(yaw: f32, pitch: f32) -> V3 {
    [yaw.sin() * pitch.cos(), pitch.sin(), -yaw.cos() * pitch.cos()]
}

/// Your own player's movement state.
#[derive(Clone, Debug)]
pub struct Body {
    pub pos: V3,
    pub vy: f32,
    pub on_ground: bool,
}

/// What the keys say this frame.
#[derive(Clone, Copy, Default, Debug)]
pub struct Input {
    pub forward: f32,
    pub right: f32,
    pub jump: bool,
}

fn overlaps(pos: V3, blk: &Block) -> bool {
    pos[0] + RADIUS > blk.min[0]
        && pos[0] - RADIUS < blk.max[0]
        && pos[2] + RADIUS > blk.min[2]
        && pos[2] - RADIUS < blk.max[2]
        && pos[1] + HEIGHT > blk.min[1]
        && pos[1] < blk.max[1]
}

fn blocked(pos: V3, world: &[Block]) -> bool {
    world.iter().any(|blk| overlaps(pos, blk))
}

/// Move with walls, steps, jumping and gravity.
pub fn step(body: &mut Body, input: Input, yaw: f32, dt: f32, world: &[Block]) {
    let dt = dt.min(0.05);
    let (s, c) = (yaw.sin(), yaw.cos());
    let mut fx = input.forward;
    let mut rx = input.right;
    let len = (fx * fx + rx * rx).sqrt();
    if len > 1.0 {
        fx /= len;
        rx /= len;
    }
    let vx = (s * fx + c * rx) * SPEED;
    let vz = (-c * fx + s * rx) * SPEED;
    for (axis, v) in [(0usize, vx), (2usize, vz)] {
        if v == 0.0 {
            continue;
        }
        let mut next = body.pos;
        next[axis] += v * dt;
        if !blocked(next, world) {
            body.pos = next;
        } else if body.on_ground {
            // Walk up a low step.
            let mut up = next;
            up[1] += STEP;
            if !blocked(up, world) {
                body.pos = up;
            }
        }
    }
    if input.jump && body.on_ground {
        body.vy = JUMP;
        body.on_ground = false;
    }
    body.vy -= GRAVITY * dt;
    let mut next = body.pos;
    next[1] += body.vy * dt;
    if blocked(next, world) || next[1] < 0.0 {
        if body.vy < 0.0 {
            // Land on the floor or on top of whatever is below.
            let ground = world
                .iter()
                .filter(|blk| {
                    body.pos[0] + RADIUS > blk.min[0]
                        && body.pos[0] - RADIUS < blk.max[0]
                        && body.pos[2] + RADIUS > blk.min[2]
                        && body.pos[2] - RADIUS < blk.max[2]
                        && blk.max[1] <= body.pos[1] + 0.01
                })
                .map(|blk| blk.max[1])
                .fold(0.0f32, f32::max);
            body.pos[1] = ground;
            body.on_ground = true;
        }
        body.vy = 0.0;
    } else {
        body.pos = next;
        // Walking off an edge: start falling.
        let mut below = body.pos;
        below[1] -= 0.02;
        body.on_ground = body.pos[1] <= 0.0 || blocked(below, world);
    }
    let lim = SIZE - RADIUS;
    body.pos[0] = body.pos[0].clamp(-lim, lim);
    body.pos[2] = body.pos[2].clamp(-lim, lim);
}

/// Distance along a ray to a box (None: missed).
fn ray_box(o: V3, d: V3, min: V3, max: V3) -> Option<f32> {
    let mut t0 = 0.0f32;
    let mut t1 = f32::MAX;
    for i in 0..3 {
        if d[i].abs() < 1e-6 {
            if o[i] < min[i] || o[i] > max[i] {
                return None;
            }
        } else {
            let a = (min[i] - o[i]) / d[i];
            let b = (max[i] - o[i]) / d[i];
            let (a, b) = if a < b { (a, b) } else { (b, a) };
            t0 = t0.max(a);
            t1 = t1.min(b);
            if t0 > t1 {
                return None;
            }
        }
    }
    Some(t0)
}

/// Fire from `eye` along `dir`: who is hit (and where), or how far the shot flew.
pub fn trace(eye: V3, dir: V3, world: &[Block], players: &[(u64, V3)], me: u64) -> (Option<(u64, bool)>, f32) {
    let wall = world.iter().filter_map(|blk| ray_box(eye, dir, blk.min, blk.max)).fold(RANGE, f32::min);
    let wall = wall.min(-eye[1] / dir[1].min(-1e-6)).min(RANGE);
    let mut best: Option<(u64, bool, f32)> = None;
    for (id, p) in players {
        if *id == me {
            continue;
        }
        let min = [p[0] - RADIUS, p[1], p[2] - RADIUS];
        let max = [p[0] + RADIUS, p[1] + HEIGHT, p[2] + RADIUS];
        if let Some(t) = ray_box(eye, dir, min, max) {
            if t < wall && best.map(|b| t < b.2).unwrap_or(true) {
                let hit_y = eye[1] + dir[1] * t;
                best = Some((*id, hit_y >= p[1] + HEAD, t));
            }
        }
    }
    match best {
        Some((id, head, t)) => (Some((id, head)), t),
        None => (None, wall),
    }
}

/// The host's match clock and winner.
#[derive(Clone, Debug)]
pub struct Match {
    pub state: MatchState,
}

impl Match {
    pub fn new() -> Match {
        Match { state: MatchState { phase: Phase::Lobby, round: 0, time_left: MATCH_SECS, target: TARGET_KOS, winner: 0, players: 1 } }
    }

    pub fn start(&mut self) {
        self.state.phase = Phase::Playing;
        self.state.round += 1;
        self.state.time_left = MATCH_SECS;
        self.state.winner = 0;
    }

    /// Move the clock on; ends the round at the target or when time runs out.
    pub fn update(&mut self, dt: f32, players: &[PlayerState]) {
        if self.state.phase != Phase::Playing {
            return;
        }
        self.state.time_left = (self.state.time_left - dt).max(0.0);
        let current: Vec<&PlayerState> = players.iter().filter(|p| p.round == self.state.round).collect();
        let reached = current.iter().any(|p| p.kos >= self.state.target);
        if reached || self.state.time_left <= 0.0 {
            self.state.phase = Phase::Over;
            let best = current.iter().map(|p| p.kos).max().unwrap_or(0);
            let top: Vec<u64> = current.iter().filter(|p| p.kos == best).map(|p| p.id).collect();
            self.state.winner = if best > 0 && top.len() == 1 { top[0] } else { 0 };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn standing(pos: V3) -> Body {
        Body { pos, vy: 0.0, on_ground: true }
    }

    #[test]
    fn walking_forward_moves_the_way_you_look() {
        let w = world();
        let mut body = standing([-20.0, 0.0, 20.0]);
        for _ in 0..30 {
            step(&mut body, Input { forward: 1.0, ..Default::default() }, 0.0, 1.0 / 60.0, &w);
        }
        assert!(body.pos[2] < 20.0 - 2.5, "moved toward -z: {:?}", body.pos);
        assert!((body.pos[0] + 20.0).abs() < 0.01);
    }

    #[test]
    fn walls_stop_you_and_the_floor_holds_you() {
        let w = world();
        let mut body = standing([0.0, 0.0, 20.0]);
        for _ in 0..600 {
            step(&mut body, Input { forward: -1.0, ..Default::default() }, 0.0, 1.0 / 60.0, &w);
        }
        assert!(body.pos[2] <= SIZE - RADIUS + 1e-3, "inside the arena: {:?}", body.pos);
        assert!(body.pos[1].abs() < 1e-3, "still on the floor");
    }

    #[test]
    fn jumping_goes_up_and_comes_back_down() {
        let w = world();
        let mut body = standing([-20.0, 0.0, -20.0]);
        step(&mut body, Input { jump: true, ..Default::default() }, 0.0, 1.0 / 60.0, &w);
        let mut top: f32 = 0.0;
        for _ in 0..120 {
            step(&mut body, Input::default(), 0.0, 1.0 / 60.0, &w);
            top = top.max(body.pos[1]);
        }
        assert!(top > 0.8, "jumped {top} m");
        assert!(body.on_ground && body.pos[1].abs() < 1e-3);
    }

    #[test]
    fn steps_lead_up_to_the_raised_middle() {
        let w = world();
        // Walk north (-z) from in front of the south steps onto the platform.
        let mut body = standing([0.0, 0.0, 7.0]);
        // Walk until well onto the platform (z = 2), not across it and down the far side.
        for _ in 0..240 {
            if body.pos[2] < 2.0 {
                break;
            }
            step(&mut body, Input { forward: 1.0, ..Default::default() }, 0.0, 1.0 / 60.0, &w);
        }
        for _ in 0..30 {
            step(&mut body, Input::default(), 0.0, 1.0 / 60.0, &w); // settle
        }
        assert!(body.pos[2] < 2.5, "reached the platform: {:?}", body.pos);
        assert!((body.pos[1] - 1.2).abs() < 0.02, "standing on top of it: {:?}", body.pos);
    }

    #[test]
    fn shots_hit_players_but_not_through_walls() {
        let w = world();
        let eye = [-20.0, EYE, 20.0];
        // A player straight ahead (toward -z), in the open.
        let target = (7u64, [-20.0, 0.0, 15.0]);
        let (hit, _) = trace(eye, look_dir(0.0, 0.0), &w, &[target], 1);
        assert_eq!(hit, Some((7, true)), "eye height hits the head");
        let (hit, _) = trace(eye, look_dir(0.0, -0.2), &w, &[target], 1);
        assert_eq!(hit.map(|h| h.1), Some(false), "lower is a body hit");
        // Behind a pillar: the pillar at (11..12.5, 11..12.5) is between these two.
        let eye2 = [11.75, EYE, 20.0];
        let behind = (8u64, [11.75, 0.0, 5.0]);
        let (hit, dist) = trace(eye2, look_dir(0.0, 0.0), &w, &[behind], 1);
        assert_eq!(hit, None);
        assert!(dist < 9.0);
    }

    #[test]
    fn the_round_ends_at_the_target_with_a_winner() {
        let mut m = Match::new();
        m.start();
        let p = |id, kos| PlayerState {
            id,
            name: String::new(),
            hue: 0,
            pos: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            hp: 100,
            alive: true,
            kos,
            downs: 0,
            round: 1,
        };
        m.update(1.0, &[p(1, 3), p(2, 5)]);
        assert_eq!(m.state.phase, Phase::Playing);
        m.update(1.0, &[p(1, 3), p(2, TARGET_KOS)]);
        assert_eq!(m.state.phase, Phase::Over);
        assert_eq!(m.state.winner, 2);
    }

    #[test]
    fn spawns_are_inside_and_away_from_others() {
        for s in spawns() {
            assert!(s[0].abs() < SIZE && s[2].abs() < SIZE);
            assert!(!blocked(s, &world()), "spawn {s:?} is clear");
        }
        let near = [[-21.0, 0.0, -21.0]];
        let s = pick_spawn(&near, 0);
        assert!((s[0] + 21.0).abs() + (s[2] + 21.0).abs() > 30.0);
    }
}
