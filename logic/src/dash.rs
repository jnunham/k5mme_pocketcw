//! DASH: a side-scroller. Your runner heads right while obstacles come at it, each carrying a
//! character. Key the character and the runner jumps the obstacle; miss it and you lose a life.
//! In HEAR mode the character is hidden and sent in Morse as the obstacle appears. Positions are
//! screen pixels, as `firmware/src/ui/dash.rs` draws them.

use crate::rng::Rng;

/// Width of the field the obstacles cross.
pub const FIELD_W: i32 = 272;
/// The runner's left edge and width.
pub const HERO_X: i32 = 30;
pub const HERO_W: i32 = 32;
/// Height of a jump at its top.
pub const JUMP_H: i32 = 60;
pub const MAX_OBSTACLES: usize = 4;
pub const LIVES: u8 = 3;
/// How long after a crash the runner can't crash again, flashing.
pub const HURT_MS: u32 = 1500;
const START_SPEED: u32 = 70;
const MAX_SPEED: u32 = 220;
/// Speed gained per obstacle cleared, in pixels a second.
const SPEED_STEP: u32 = 4;
/// Pixels before an obstacle where the jump starts.
const JUMP_LEAD: i32 = 30;
const MAX_POOL: usize = 44;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Crate,
    Rock,
    Tower,
}

impl Kind {
    /// Width on screen.
    pub fn width(self) -> i32 {
        match self {
            Kind::Crate => 28,
            Kind::Rock => 32,
            Kind::Tower => 20,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Obstacle {
    pub x: i32,
    pub kind: Kind,
    pub c: u8,
    /// You keyed it: the runner jumps it.
    pub cleared: bool,
    /// The runner ran into it; it passes through.
    pub crashed: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// The character shows over the obstacle.
    See,
    /// The character is sent in Morse, and shows only once you clear it.
    Hear,
}

/// What a step of time did.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Events {
    /// A new obstacle with this character came on screen.
    pub spawned: Option<u8>,
    pub crashed: bool,
}

/// What a keyed character did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    /// It matched the next obstacle; the runner will jump it.
    Cleared,
    /// It matched nothing ahead.
    Wrong,
    /// No obstacle was waiting for a key.
    Nothing,
}

pub struct Dash {
    rng: Rng,
    pool: [u8; MAX_POOL],
    pool_len: usize,
    pub mode: Mode,
    pub obstacles: [Option<Obstacle>; MAX_OBSTACLES],
    pub score: u16,
    pub lives: u8,
    pub cleared: u16,
    /// Pixels travelled, for the scrolling scenery and the running animation.
    pub scroll: u32,
    /// Movement below a pixel, in pixel-milliseconds.
    carry: u32,
    spawn_in: i32,
    pub hurt_ms: u32,
}

impl Dash {
    /// A run whose obstacles carry characters from `pool`; characters the screen can't draw
    /// are left out by the caller.
    pub fn new(pool: &[u8], mode: Mode, seed: u32) -> Dash {
        let mut buf = [b'E'; MAX_POOL];
        let n = pool.len().clamp(1, MAX_POOL);
        buf[..pool.len().min(MAX_POOL)].copy_from_slice(&pool[..pool.len().min(MAX_POOL)]);
        Dash {
            rng: Rng::new(seed),
            pool: buf,
            pool_len: n,
            mode,
            obstacles: [None; MAX_OBSTACLES],
            score: 0,
            lives: LIVES,
            cleared: 0,
            scroll: 0,
            carry: 0,
            spawn_in: 40,
            hurt_ms: 0,
        }
    }

    /// Pixels a second; faster with every obstacle cleared.
    pub fn speed(&self) -> u32 {
        (START_SPEED + SPEED_STEP * self.cleared as u32).min(MAX_SPEED)
    }

    /// 1 and up, for the screen.
    pub fn level(&self) -> u32 {
        1 + (self.speed() - START_SPEED) / 20
    }

    pub fn over(&self) -> bool {
        self.lives == 0
    }

    /// Moves the run on by `ms`.
    pub fn step(&mut self, ms: u32) -> Events {
        let mut events = Events::default();
        if self.over() {
            return events;
        }
        self.carry += self.speed() * ms;
        let dist = (self.carry / 1000) as i32;
        self.carry %= 1000;
        self.scroll = self.scroll.wrapping_add(dist as u32);
        self.hurt_ms = self.hurt_ms.saturating_sub(ms);
        for slot in &mut self.obstacles {
            let Some(o) = slot else { continue };
            o.x -= dist;
            if o.x + o.kind.width() < 0 {
                *slot = None;
                continue;
            }
            let touching = o.x <= HERO_X + HERO_W - 6 && o.x + o.kind.width() >= HERO_X + 6;
            if touching && !o.cleared && !o.crashed {
                o.crashed = true;
                if self.hurt_ms == 0 {
                    self.lives = self.lives.saturating_sub(1);
                    self.hurt_ms = HURT_MS;
                    events.crashed = true;
                }
            }
        }
        self.spawn_in -= dist;
        if self.spawn_in <= 0
            && let Some(slot) = self.obstacles.iter().position(Option::is_none)
        {
            let c = self.pool[self.rng.below(self.pool_len as u32) as usize];
            let kind = [Kind::Crate, Kind::Rock, Kind::Tower][self.rng.below(3) as usize];
            self.obstacles[slot] = Some(Obstacle { x: FIELD_W, kind, c, cleared: false, crashed: false });
            events.spawned = Some(c);
            self.spawn_in = self.gap();
        }
        events
    }

    /// Pixels to the next obstacle: time enough to read or hear one and key it.
    fn gap(&mut self) -> i32 {
        let react_ms = match self.mode {
            Mode::See => 1300,
            Mode::Hear => 2200,
        };
        let min = (self.speed() * react_ms / 1000) as i32;
        min.max(90) + self.rng.below(60) as i32
    }

    /// The obstacle a key goes to: the nearest ahead not yet cleared or crashed into.
    fn next(&self) -> Option<usize> {
        (0..MAX_OBSTACLES)
            .filter(|&i| self.obstacles[i].is_some_and(|o| !o.cleared && !o.crashed && o.x + o.kind.width() > HERO_X))
            .min_by_key(|&i| self.obstacles[i].map(|o| o.x))
    }

    /// The character of the next obstacle, for HEAR's AGAIN.
    pub fn next_char(&self) -> Option<u8> {
        self.next().and_then(|i| self.obstacles[i]).map(|o| o.c)
    }

    /// You keyed `c`. Right clears the next obstacle and scores 10, plus a bonus the farther
    /// away it still is.
    pub fn key(&mut self, c: u8) -> Key {
        let Some(i) = self.next() else { return Key::Nothing };
        let Some(o) = &mut self.obstacles[i] else { return Key::Nothing };
        if o.c != c.to_ascii_uppercase() {
            return Key::Wrong;
        }
        o.cleared = true;
        let early = ((o.x - HERO_X - HERO_W).max(0) / 20) as u16;
        self.score = self.score.saturating_add(10 + early);
        self.cleared += 1;
        Key::Cleared
    }

    /// How high the runner is: an arc over each cleared obstacle as it passes under.
    pub fn hero_height(&self) -> i32 {
        self.obstacles
            .iter()
            .flatten()
            .filter(|o| o.cleared)
            .map(|o| {
                let start = HERO_X + HERO_W + JUMP_LEAD;
                let end = HERO_X - o.kind.width() - 4;
                if o.x > start || o.x < end {
                    return 0;
                }
                // t from 0 at take-off to 1000 at landing.
                let t = ((start - o.x) * 1000 / (start - end)) as i64;
                (4 * JUMP_H as i64 * t * (1000 - t) / 1_000_000) as i32
            })
            .max()
            .unwrap_or(0)
    }

    /// 0 or 1 while running, 2 in the air.
    pub fn hero_frame(&self) -> u8 {
        if self.hero_height() > 0 { 2 } else { (self.scroll / 10 % 2) as u8 }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    fn run_until_spawn(d: &mut Dash) -> u8 {
        for _ in 0..1000 {
            if let Some(c) = d.step(20).spawned {
                return c;
            }
        }
        panic!("nothing spawned");
    }

    #[test]
    fn a_keyed_obstacle_is_jumped_and_an_unkeyed_one_costs_a_life() {
        let mut d = Dash::new(b"KM", Mode::See, 3);
        let c = run_until_spawn(&mut d);
        assert_eq!(d.next_char(), Some(c));
        let wrong = if c == b'K' { b'M' } else { b'K' };
        assert_eq!(d.key(wrong), Key::Wrong);
        assert_eq!(d.key(c.to_ascii_lowercase()), Key::Cleared);
        assert!(d.score >= 10);
        let mut peak = 0;
        let mut crashed = false;
        while d.obstacles.iter().flatten().any(|o| o.cleared) {
            crashed |= d.step(20).crashed;
            peak = peak.max(d.hero_height());
        }
        assert!(!crashed, "a cleared obstacle never hits");
        assert!(peak > JUMP_H * 9 / 10 && peak <= JUMP_H, "{peak}");
        assert_eq!(d.lives, LIVES);
        // The next one, not keyed, hits.
        run_until_spawn(&mut d);
        let before = d.lives;
        let mut steps = 0;
        while !d.step(20).crashed {
            steps += 1;
            assert!(steps < 1000, "it never hit");
        }
        assert_eq!(d.lives, before - 1);
        assert_eq!(d.hurt_ms, HURT_MS);
    }

    #[test]
    fn the_runner_is_in_the_air_whenever_a_cleared_obstacle_is_under_it() {
        let mut d = Dash::new(b"E", Mode::See, 1);
        for kind in [Kind::Crate, Kind::Rock, Kind::Tower] {
            for x in -40..120 {
                d.obstacles = [Some(Obstacle { x, kind, c: b'E', cleared: true, crashed: false }), None, None, None];
                let under = x <= HERO_X + HERO_W - 6 && x + kind.width() >= HERO_X + 6;
                if under {
                    assert!(d.hero_height() >= 20, "{kind:?} at {x}: {}", d.hero_height());
                }
            }
        }
    }

    #[test]
    fn keys_go_to_the_nearest_obstacle_ahead() {
        let mut d = Dash::new(b"AB", Mode::Hear, 1);
        d.obstacles[0] = Some(Obstacle { x: 200, kind: Kind::Rock, c: b'A', cleared: false, crashed: false });
        d.obstacles[2] = Some(Obstacle { x: 120, kind: Kind::Crate, c: b'B', cleared: false, crashed: false });
        assert_eq!(d.next_char(), Some(b'B'));
        assert_eq!(d.key(b'A'), Key::Wrong, "the nearer one comes first");
        assert_eq!(d.key(b'B'), Key::Cleared);
        assert_eq!(d.key(b'A'), Key::Cleared);
        assert_eq!(d.key(b'A'), Key::Nothing);
    }

    #[test]
    fn it_speeds_up_and_ends_after_three_lives() {
        let mut d = Dash::new(b"QZ", Mode::See, 7);
        let slow = d.speed();
        d.cleared = 10;
        assert!(d.speed() > slow && d.level() > 1);
        d.cleared = 1000;
        assert_eq!(d.speed(), MAX_SPEED);
        d.cleared = 0;
        for _ in 0..20_000 {
            d.step(20);
            if d.over() {
                break;
            }
        }
        assert!(d.over());
        assert_eq!(d.step(20), Events::default());
    }

    #[test]
    fn obstacles_keep_their_distance() {
        for mode in [Mode::See, Mode::Hear] {
            let mut d = Dash::new(b"ABCDEFG", mode, 11);
            for _ in 0..3000 {
                d.step(20);
                let mut xs: std::vec::Vec<i32> = d.obstacles.iter().flatten().map(|o| o.x).collect();
                xs.sort();
                assert!(xs.windows(2).all(|w| w[1] - w[0] >= 90), "{mode:?} {xs:?}");
                d.lives = LIVES;
                if let Some(c) = d.next_char() {
                    d.key(c);
                }
            }
        }
    }
}
