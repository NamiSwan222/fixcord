//! Multiplayer Snake, played on the home network as a Hearth "activity".
//!
//! One computer (the host) runs the game; everyone sends their key presses to it and it sends
//! the whole board back several times a second. This file holds the rules only — no drawing and
//! no networking — so they can be tested on their own.

use serde::{Deserialize, Serialize};

/// Board size in cells (estimated to fit a chat window nicely).
pub const W: i16 = 40;
pub const H: i16 = 26;
/// Moves per second.
pub const TICKS_PER_SEC: f64 = 9.0;
/// Most players in one game.
pub const MAX_PLAYERS: usize = 8;
/// Length of a new snake.
const START_LEN: usize = 4;
/// Seconds of "3, 2, 1" before a round starts.
pub const COUNTDOWN: f64 = 3.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Dir {
    fn delta(self) -> (i16, i16) {
        match self {
            Dir::Up => (0, -1),
            Dir::Down => (0, 1),
            Dir::Left => (-1, 0),
            Dir::Right => (1, 0),
        }
    }
    fn opposite(self) -> Dir {
        match self {
            Dir::Up => Dir::Down,
            Dir::Down => Dir::Up,
            Dir::Left => Dir::Right,
            Dir::Right => Dir::Left,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Phase {
    /// Waiting for players; the host starts the round.
    Lobby,
    /// Counting down; snakes are placed but don't move yet.
    Countdown,
    Playing,
    /// Round over; the host can play again.
    Over,
}

/// One snake as everyone sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnakeView {
    pub id: u64,
    pub name: String,
    pub hue: u8,
    /// Head first: x0, y0, x1, y1, …
    pub body: Vec<i16>,
    pub alive: bool,
    pub score: u32,
}

/// The whole board, sent from the host to every player.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Board {
    pub tick: u64,
    pub phase: Phase,
    /// Seconds left in the countdown.
    #[serde(default)]
    pub countdown: f32,
    /// x, y pairs.
    pub food: Vec<i16>,
    pub snakes: Vec<SnakeView>,
    /// Who won the last round (None: nobody / a draw).
    #[serde(default)]
    pub winner: Option<u64>,
}

struct Snake {
    id: u64,
    name: String,
    hue: u8,
    body: Vec<(i16, i16)>, // head first
    dir: Dir,
    /// Turns pressed since the last move (so quick double turns aren't lost).
    queue: Vec<Dir>,
    alive: bool,
    score: u32,
    grow: u32,
}

pub struct Game {
    snakes: Vec<Snake>,
    food: Vec<(i16, i16)>,
    pub phase: Phase,
    pub tick: u64,
    countdown_left: f64,
    rng: u64,
    winner: Option<u64>,
    solo: bool,
}

impl Game {
    pub fn new(seed: u64) -> Game {
        Game { snakes: vec![], food: vec![], phase: Phase::Lobby, tick: 0, countdown_left: 0.0, rng: seed | 1, winner: None, solo: false }
    }

    fn rand(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    pub fn players(&self) -> usize {
        self.snakes.len()
    }

    pub fn has_player(&self, id: u64) -> bool {
        self.snakes.iter().any(|s| s.id == id)
    }

    /// Join (only between rounds). Returns false when full or a round is running.
    pub fn add_player(&mut self, id: u64, name: &str, hue: u8) -> bool {
        if self.has_player(id) {
            return true;
        }
        if self.snakes.len() >= MAX_PLAYERS || matches!(self.phase, Phase::Countdown | Phase::Playing) {
            return false;
        }
        self.snakes.push(Snake {
            id,
            name: name.chars().take(24).collect(),
            hue,
            body: vec![],
            dir: Dir::Right,
            queue: vec![],
            alive: false,
            score: 0,
            grow: 0,
        });
        true
    }

    pub fn remove_player(&mut self, id: u64) {
        self.snakes.retain(|s| s.id != id);
    }

    /// Place everyone and count down.
    pub fn start(&mut self) {
        if self.snakes.is_empty() {
            return;
        }
        let n = self.snakes.len();
        self.solo = n == 1;
        for (i, s) in self.snakes.iter_mut().enumerate() {
            // Players start in rows spread over the board, alternating sides and directions.
            let row = (H as usize * (i / 2 * 2 + 1) / (n.div_ceil(2) * 2 + 1)).clamp(2, H as usize - 3) as i16;
            let row = if i % 2 == 1 { (row + 1).min(H - 3) } else { row };
            let (x, dir) = if i % 2 == 0 { (6, Dir::Right) } else { (W - 7, Dir::Left) };
            let (dx, _) = dir.delta();
            s.body = (0..START_LEN as i16).map(|k| (x - dx * k, row)).collect();
            s.dir = dir;
            s.queue.clear();
            s.alive = true;
            s.score = 0;
            s.grow = 0;
        }
        self.food.clear();
        let want = 1 + n;
        for _ in 0..want {
            self.add_food();
        }
        self.winner = None;
        self.phase = Phase::Countdown;
        self.countdown_left = COUNTDOWN;
    }

    fn occupied(&self, c: (i16, i16)) -> bool {
        self.snakes.iter().any(|s| s.alive && s.body.contains(&c)) || self.food.contains(&c)
    }

    fn add_food(&mut self) {
        for _ in 0..500 {
            let x = (self.rand() % W as u64) as i16;
            let y = (self.rand() % H as u64) as i16;
            if !self.occupied((x, y)) {
                self.food.push((x, y));
                return;
            }
        }
    }

    /// A player pressed a direction.
    pub fn steer(&mut self, id: u64, d: Dir) {
        if let Some(s) = self.snakes.iter_mut().find(|s| s.id == id) {
            let last = *s.queue.last().unwrap_or(&s.dir);
            if d != last && d != last.opposite() && s.queue.len() < 3 {
                s.queue.push(d);
            }
        }
    }

    /// Advance the countdown by `dt` seconds (call every frame on the host).
    pub fn update_countdown(&mut self, dt: f64) {
        if self.phase == Phase::Countdown {
            self.countdown_left -= dt;
            if self.countdown_left <= 0.0 {
                self.phase = Phase::Playing;
            }
        }
    }

    /// One move for every snake.
    pub fn step(&mut self) {
        if self.phase != Phase::Playing {
            return;
        }
        self.tick += 1;
        // Turn and find each new head.
        let mut heads: Vec<Option<(i16, i16)>> = vec![];
        for s in self.snakes.iter_mut() {
            if !s.alive {
                heads.push(None);
                continue;
            }
            if !s.queue.is_empty() {
                s.dir = s.queue.remove(0);
            }
            let (dx, dy) = s.dir.delta();
            let h = s.body[0];
            heads.push(Some((h.0 + dx, h.1 + dy)));
        }
        // Move tails first (a snake may follow its own or another's tail).
        let eats: Vec<bool> = heads.iter().map(|h| h.map(|c| self.food.contains(&c)).unwrap_or(false)).collect();
        for (i, s) in self.snakes.iter_mut().enumerate() {
            if s.alive && !eats[i] && s.grow == 0 {
                s.body.pop();
            } else if s.alive && s.grow > 0 {
                s.grow -= 1;
            }
        }
        // Collisions.
        let mut dies = vec![false; self.snakes.len()];
        for (i, h) in heads.iter().enumerate() {
            let Some(c) = *h else { continue };
            if c.0 < 0 || c.1 < 0 || c.0 >= W || c.1 >= H {
                dies[i] = true;
                continue;
            }
            if self.snakes.iter().any(|s| s.alive && s.body.contains(&c)) {
                dies[i] = true;
                continue;
            }
            if heads.iter().enumerate().any(|(j, o)| j != i && *o == Some(c)) {
                dies[i] = true; // head to head
            }
        }
        for (i, s) in self.snakes.iter_mut().enumerate() {
            let Some(c) = heads[i] else { continue };
            if dies[i] {
                s.alive = false;
            } else {
                s.body.insert(0, c);
            }
        }
        // Food.
        for (i, s) in self.snakes.iter_mut().enumerate() {
            if s.alive && eats[i] {
                s.score += 1;
            }
        }
        let eaten: Vec<(i16, i16)> =
            heads.iter().enumerate().filter(|(i, _)| eats[*i] && !dies[*i]).filter_map(|(_, h)| *h).collect();
        self.food.retain(|f| !eaten.contains(f));
        while self.food.len() < 1 + self.snakes.len() {
            let before = self.food.len();
            self.add_food();
            if self.food.len() == before {
                break; // board full
            }
        }
        // Round over?
        let alive: Vec<u64> = self.snakes.iter().filter(|s| s.alive).map(|s| s.id).collect();
        let over = if self.solo { alive.is_empty() } else { alive.len() <= 1 };
        if over {
            self.phase = Phase::Over;
            self.winner = if self.solo {
                None
            } else if alive.len() == 1 {
                Some(alive[0])
            } else {
                // Everyone out at once: the highest score wins, a tie is a draw.
                let best = self.snakes.iter().map(|s| s.score).max().unwrap_or(0);
                let top: Vec<u64> = self.snakes.iter().filter(|s| s.score == best).map(|s| s.id).collect();
                if top.len() == 1 { Some(top[0]) } else { None }
            };
        }
    }

    pub fn board(&self) -> Board {
        Board {
            tick: self.tick,
            phase: self.phase,
            countdown: self.countdown_left.max(0.0) as f32,
            food: self.food.iter().flat_map(|(x, y)| [*x, *y]).collect(),
            snakes: self
                .snakes
                .iter()
                .map(|s| SnakeView {
                    id: s.id,
                    name: s.name.clone(),
                    hue: s.hue,
                    body: s.body.iter().flat_map(|(x, y)| [*x, *y]).collect(),
                    alive: s.alive,
                    score: s.score,
                })
                .collect(),
            winner: self.winner,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(g: &Game, id: u64) -> (i16, i16) {
        let s = g.snakes.iter().find(|s| s.id == id).unwrap();
        s.body[0]
    }

    fn started(players: &[u64]) -> Game {
        let mut g = Game::new(42);
        for (i, id) in players.iter().enumerate() {
            assert!(g.add_player(*id, &format!("P{id}"), i as u8));
        }
        g.start();
        g.update_countdown(COUNTDOWN + 0.1);
        assert_eq!(g.phase, Phase::Playing);
        g
    }

    #[test]
    fn snakes_move_and_turn_but_never_reverse() {
        let mut g = started(&[1]);
        let (x, y) = head(&g, 1);
        g.step();
        assert_eq!(head(&g, 1), (x + 1, y));
        g.steer(1, Dir::Left); // straight back: ignored
        g.step();
        assert_eq!(head(&g, 1), (x + 2, y));
        g.steer(1, Dir::Down);
        g.step();
        assert_eq!(head(&g, 1), (x + 2, y + 1));
    }

    #[test]
    fn eating_food_grows_the_snake_and_scores() {
        let mut g = started(&[1]);
        let (x, y) = head(&g, 1);
        g.food = vec![(x + 1, y)];
        let len = g.snakes[0].body.len();
        g.step();
        assert_eq!(g.snakes[0].score, 1);
        assert_eq!(g.snakes[0].body.len(), len + 1);
        assert_eq!(g.food.len(), 2, "food is put back (one per player plus one)");
    }

    #[test]
    fn hitting_a_wall_ends_a_solo_round() {
        let mut g = started(&[1]);
        for _ in 0..(W as usize) {
            g.step();
        }
        assert!(!g.snakes[0].alive);
        assert_eq!(g.phase, Phase::Over);
    }

    #[test]
    fn last_snake_alive_wins() {
        let mut g = started(&[1, 2]);
        // Player 2 turns into the wall quickly; player 1 keeps going.
        g.steer(2, Dir::Up);
        for _ in 0..(H as usize) {
            g.step();
            g.steer(1, if g.tick % 2 == 0 { Dir::Down } else { Dir::Right });
            if g.phase == Phase::Over {
                break;
            }
        }
        assert_eq!(g.phase, Phase::Over);
        assert_eq!(g.board().winner, Some(1));
    }

    #[test]
    fn nobody_joins_mid_round_and_board_round_trips() {
        let mut g = started(&[1, 2]);
        assert!(!g.add_player(3, "late", 2));
        let b = g.board();
        let json = serde_json::to_string(&b).unwrap();
        assert!(json.len() < 4000, "a board fits easily in one network packet ({} bytes)", json.len());
        let back: Board = serde_json::from_str(&json).unwrap();
        assert_eq!(back, b);
    }

    /// Even eight long snakes fit in one network message (the receiver reads up to 64 KB).
    #[test]
    fn a_full_board_fits_in_one_network_message() {
        let ids: Vec<u64> = (1..=8).map(|i| u64::MAX - i).collect();
        let mut g = started(&ids);
        for s in g.snakes.iter_mut() {
            s.name = "A long player name here".into();
            let (x, y) = s.body[0];
            s.body = (0..150).map(|k| (x, (y + k) % H)).collect();
        }
        let p = crate::net::Packet::SnakeBoard { from: u64::MAX, game: u64::MAX, host: "Host".into(), board: g.board() };
        let json = serde_json::to_vec(&p).unwrap();
        assert!(json.len() < 60_000, "{} bytes", json.len());
        let back: crate::net::Packet = serde_json::from_slice(&json).unwrap();
        assert!(matches!(back, crate::net::Packet::SnakeBoard { .. }));
    }

    #[test]
    fn eight_players_start_apart() {
        let ids: Vec<u64> = (1..=8).collect();
        let g = started(&ids);
        let mut cells = std::collections::HashSet::new();
        for s in &g.snakes {
            for c in &s.body {
                assert!(c.0 >= 0 && c.1 >= 0 && c.0 < W && c.1 < H, "inside the board");
                assert!(cells.insert(*c), "no two snakes start on the same cell");
            }
        }
    }
}
