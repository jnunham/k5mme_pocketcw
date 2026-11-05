//! GAMES: five small games that train copy and sending. Each keeps the rules here, with no
//! hardware; the screens in `firmware/src/ui/games.rs` draw them and feed them your keying.
//!
//! - LADDER: callsigns speed up when you copy them and slow down when you miss, after RufzXP.
//! - INVADERS: characters fall; key one to shoot it before it lands.
//! - SIMON: a sequence that grows by one character each time you key it back right.
//! - WORDLE: guess the five-letter word by keying it, with the colored clues.
//! - FIST: key the word shown, scored on the timing of every element and gap.

use core::fmt::Write;

use crate::drill::{self, MAX_GROUP, Score};
use crate::morse;
use crate::qso::home;
use crate::rng::Rng;
use crate::text::Text;
use crate::timing::Quality;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Game {
    Ladder,
    Invaders,
    Simon,
    Wordle,
    Fist,
    /// The side-scroller, in `dash`.
    Dash,
}

/// A game's best score is stored at its index here; new games go at the end.
pub const GAMES: [Game; 6] = [Game::Ladder, Game::Invaders, Game::Simon, Game::Wordle, Game::Fist, Game::Dash];

impl Game {
    pub fn name(self) -> &'static str {
        match self {
            Game::Ladder => "LADDER",
            Game::Invaders => "INVADERS",
            Game::Simon => "SIMON",
            Game::Wordle => "WORDLE",
            Game::Fist => "FIST",
            Game::Dash => "DASH",
        }
    }

    pub fn index(self) -> usize {
        GAMES.iter().position(|&g| g == self).unwrap_or(0)
    }

    /// What its best score counts, for the menu and the result line.
    pub fn best_unit(self) -> &'static str {
        match self {
            Game::Ladder | Game::Invaders | Game::Dash => "PTS",
            Game::Simon => "LONG",
            Game::Wordle => "IN A ROW",
            Game::Fist => "%",
        }
    }
}

/// Scores `copied` against `sent` like the drills, without touching the letter stats.
fn compare(sent: &str, copied: &str) -> Score {
    drill::compare(sent, copied)
}

// LADDER ------------------------------------------------------------------------------------

pub const LADDER_CALLS: u8 = 25;
const LADDER_STEP_WPM: u8 = 2;
pub const LADDER_MIN_WPM: u8 = 10;
pub const LADDER_MAX_WPM: u8 = 60;

/// How one LADDER call went.
#[derive(Clone, Copy, Debug)]
pub struct Rung {
    pub score: Score,
    pub right: bool,
    pub points: u16,
    /// The speed the call was sent at.
    pub wpm: u8,
}

pub struct Ladder {
    rng: Rng,
    pub wpm: u8,
    pub points: u16,
    pub calls: u8,
    pub right: u8,
    call: Text<MAX_GROUP>,
    /// You asked to hear this call again: it scores half.
    pub repeated: bool,
}

impl Ladder {
    pub fn new(wpm: u8, seed: u32) -> Ladder {
        let wpm = wpm.clamp(LADDER_MIN_WPM, LADDER_MAX_WPM);
        Ladder { rng: Rng::new(seed), wpm, points: 0, calls: 0, right: 0, call: Text::new(), repeated: false }
    }

    pub fn next_call(&mut self) -> Text<MAX_GROUP> {
        let call = home(&mut self.rng).call(&mut self.rng);
        self.call = Text::format(format_args!("{}", call.as_str()));
        self.repeated = false;
        self.call
    }

    pub fn call(&self) -> &str {
        self.call.as_str()
    }

    /// Scores your copy: a right call earns its length times the speed, and speeds up; a miss
    /// slows down.
    pub fn copied(&mut self, copied: &str) -> Rung {
        let score = compare(self.call.as_str(), copied);
        let right = score.right as usize == score.len && score.extra == 0;
        let wpm = self.wpm;
        let mut points = if right { score.len as u16 * wpm as u16 } else { 0 };
        if self.repeated {
            points /= 2;
        }
        self.points = self.points.saturating_add(points);
        self.calls += 1;
        self.right += right as u8;
        self.wpm = if right {
            (wpm + LADDER_STEP_WPM).min(LADDER_MAX_WPM)
        } else {
            wpm.saturating_sub(LADDER_STEP_WPM).max(LADDER_MIN_WPM)
        };
        Rung { score, right, points, wpm }
    }

    pub fn over(&self) -> bool {
        self.calls >= LADDER_CALLS
    }
}

// INVADERS ----------------------------------------------------------------------------------

pub const INVADER_SLOTS: usize = 8;
/// Columns invaders fall in.
pub const INVADER_COLS: u8 = 8;
/// Height of the field in the units of `Invader::y`; an invader lands at this.
pub const FIELD: u16 = 1000;
pub const INVADER_LIVES: u8 = 3;
const POINTS_PER_HIT: u16 = 10;
/// Points that raise the level by one.
const POINTS_PER_LEVEL: u16 = 100;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Invader {
    pub c: u8,
    pub col: u8,
    /// 0 at the top, `FIELD` on the ground.
    pub y: u16,
}

/// What one `Invaders::step` did.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Step {
    pub landed: u8,
    pub spawned: bool,
}

pub struct Invaders {
    rng: Rng,
    pool: &'static [u8],
    pub slots: [Option<Invader>; INVADER_SLOTS],
    pub score: u16,
    pub lives: u8,
    /// Time until the next invader appears.
    spawn_in_ms: u32,
    /// Fall progress below one unit, in unit-milliseconds, so slow speeds still move.
    carry: u32,
}

impl Invaders {
    /// Invaders made of the characters of `pool`.
    pub fn new(pool: &'static [u8], seed: u32) -> Invaders {
        Invaders { rng: Rng::new(seed), pool, slots: [None; INVADER_SLOTS], score: 0, lives: INVADER_LIVES, spawn_in_ms: 500, carry: 0 }
    }

    pub fn level(&self) -> u16 {
        1 + self.score / POINTS_PER_LEVEL
    }

    /// Fall speed in field units per second: the field takes 14 s at level 1, and each level is
    /// 12% faster.
    fn speed(&self) -> u32 {
        let base = FIELD as u32 * 1000 / 14_000;
        base * (100 + 12 * (self.level() as u32 - 1)) / 100
    }

    fn spawn_every_ms(&self) -> u32 {
        2600u32.saturating_sub(160 * (self.level() as u32 - 1)).max(700)
    }

    /// Moves time on by `ms`: invaders fall, those that land cost a life, and a new one may
    /// appear.
    pub fn step(&mut self, ms: u32) -> Step {
        let mut step = Step::default();
        if self.over() {
            return step;
        }
        self.carry += self.speed() * ms;
        let fall = (self.carry / 1000) as u16;
        self.carry %= 1000;
        for slot in &mut self.slots {
            if let Some(inv) = slot {
                inv.y = inv.y.saturating_add(fall);
                if inv.y >= FIELD {
                    *slot = None;
                    step.landed += 1;
                }
            }
        }
        self.lives = self.lives.saturating_sub(step.landed);
        if self.spawn_in_ms <= ms {
            step.spawned = self.spawn();
            self.spawn_in_ms = self.spawn_every_ms();
        } else {
            self.spawn_in_ms -= ms;
        }
        step
    }

    /// A new invader at the top of a free column, a character not already falling when it can.
    fn spawn(&mut self) -> bool {
        let Some(slot) = self.slots.iter().position(Option::is_none) else { return false };
        let busy = |slots: &[Option<Invader>], col: u8| slots.iter().flatten().any(|i| i.col == col && i.y < FIELD / 4);
        let mut col = self.rng.below(INVADER_COLS as u32) as u8;
        for _ in 0..INVADER_COLS {
            if !busy(&self.slots, col) {
                break;
            }
            col = (col + 1) % INVADER_COLS;
        }
        let mut c = self.pool[self.rng.below(self.pool.len() as u32) as usize];
        for _ in 0..4 {
            if !self.slots.iter().flatten().any(|i| i.c == c) {
                break;
            }
            c = self.pool[self.rng.below(self.pool.len() as u32) as usize];
        }
        self.slots[slot] = Some(Invader { c, col, y: 0 });
        true
    }

    /// You keyed `c`: the lowest invader of that character is hit. Returns its slot.
    pub fn hit(&mut self, c: u8) -> Option<usize> {
        let c = c.to_ascii_uppercase();
        let slot = (0..INVADER_SLOTS).filter(|&i| self.slots[i].is_some_and(|inv| inv.c == c)).max_by_key(|&i| self.slots[i].map(|inv| inv.y))?;
        self.slots[slot] = None;
        self.score = self.score.saturating_add(POINTS_PER_HIT);
        Some(slot)
    }

    pub fn over(&self) -> bool {
        self.lives == 0
    }
}

// SIMON -------------------------------------------------------------------------------------

pub const SIMON_MAX: usize = 20;

pub struct Simon {
    rng: Rng,
    pool: &'static [u8],
    seq: [u8; SIMON_MAX],
    len: usize,
}

impl Simon {
    /// A sequence of the characters of `pool`, one long.
    pub fn new(pool: &'static [u8], seed: u32) -> Simon {
        let mut simon = Simon { rng: Rng::new(seed), pool, seq: [0; SIMON_MAX], len: 0 };
        simon.grow();
        simon
    }

    fn grow(&mut self) {
        if self.len < SIMON_MAX {
            self.seq[self.len] = self.pool[self.rng.below(self.pool.len() as u32) as usize];
            self.len += 1;
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The sequence to send, with no spaces.
    pub fn text(&self) -> Text<MAX_GROUP> {
        let mut text = Text::new();
        for &c in &self.seq[..self.len] {
            text.push(c);
        }
        text
    }

    /// Checks your copy. Right grows the sequence by one; the score says how it went.
    pub fn copied(&mut self, copied: &str) -> (Score, bool) {
        let score = compare(self.text().as_str(), copied);
        let right = score.right as usize == score.len && score.extra == 0;
        if right {
            self.grow();
        }
        (score, right)
    }

    /// You keyed back every one of the longest sequence.
    pub fn won(&self) -> bool {
        self.len == SIMON_MAX
    }
}

// WORDLE ------------------------------------------------------------------------------------

pub const WORD_LEN: usize = 5;
pub const TRIES: usize = 6;

/// Five-letter words: radio words first, then common English.
const WORDLE_WORDS: [&str; 120] = [
    "RADIO", "POWER", "NOISE", "SOLAR", "WATTS", "RELAY", "BANDS", "CALLS", "TUNER", "TOWER", "LOOPS", "SPARK", "PILOT",
    "GRIDS", "PEAKS", "WAVES", "PARKS", "LINKS", "MORSE", "CODES", "SPEED", "TONES", "PITCH", "KEYER", "METER", "FISTS",
    "ABOUT", "ABOVE", "AFTER", "AGAIN", "ALONG", "APPLE", "BEACH", "BEGIN", "BLACK", "BOARD", "BRAIN", "BREAD", "BRING",
    "BUILD", "CARRY", "CHAIR", "CHEST", "CLEAN", "CLEAR", "CLOCK", "CLOUD", "COAST", "COUNT", "CROWN", "DANCE", "DREAM",
    "DRINK", "EARTH", "EIGHT", "EMPTY", "ENJOY", "EVERY", "FIELD", "FIGHT", "FLAME", "FLOOR", "FOCUS", "FORCE", "FRESH",
    "FRONT", "FRUIT", "GHOST", "GIANT", "GLASS", "GRAND", "GRASS", "GREAT", "GREEN", "HAPPY", "HEART", "HEAVY", "HORSE",
    "HOUSE", "LAUGH", "LEARN", "LIGHT", "LUCKY", "MAGIC", "MONEY", "MONTH", "MUSIC", "NIGHT", "NORTH", "OCEAN", "PAINT",
    "PAPER", "PEACE", "PHONE", "PLANT", "PLATE", "POINT", "QUICK", "QUIET", "RIVER", "ROUND", "SCALE", "SHARP", "SHINE",
    "SMILE", "SOLID", "SOUTH", "SPACE", "STAND", "STONE", "STORM", "SUGAR", "TABLE", "TIGER", "TRAIN", "WATER", "WHEEL",
    "WORLD", "YOUNG", "ZEBRA",
];

/// One letter of a guess.
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub enum Tile {
    /// Not in the word, or not again.
    Absent,
    /// In the word, somewhere else.
    Present,
    /// In this place.
    Right,
}

pub struct Wordle {
    rng: Rng,
    secret: [u8; WORD_LEN],
    pub guesses: [[u8; WORD_LEN]; TRIES],
    pub tiles: [[Tile; WORD_LEN]; TRIES],
    pub count: usize,
}

impl Wordle {
    pub fn new(seed: u32) -> Wordle {
        let mut w = Wordle { rng: Rng::new(seed), secret: [0; WORD_LEN], guesses: [[0; WORD_LEN]; TRIES], tiles: [[Tile::Absent; WORD_LEN]; TRIES], count: 0 };
        w.new_word();
        w
    }

    /// A new secret word, and no guesses.
    pub fn new_word(&mut self) {
        let five = || WORDLE_WORDS.iter().filter(|w| w.len() == WORD_LEN);
        let n = five().count() as u32;
        let word = five().nth(self.rng.below(n) as usize).copied().unwrap_or("MORSE");
        self.secret.copy_from_slice(word.as_bytes());
        self.count = 0;
    }

    pub fn secret(&self) -> &str {
        core::str::from_utf8(&self.secret).unwrap_or("")
    }

    /// Your keyed guess, spaces dropped. None when it is not five letters.
    pub fn guess(&mut self, keyed: &str) -> Option<[Tile; WORD_LEN]> {
        let mut guess = [0u8; WORD_LEN];
        let mut n = 0;
        for c in keyed.bytes().filter(|c| *c != b' ') {
            let c = c.to_ascii_uppercase();
            if n == WORD_LEN || !c.is_ascii_uppercase() {
                return None;
            }
            guess[n] = c;
            n += 1;
        }
        if n != WORD_LEN || self.over() {
            return None;
        }
        let tiles = tiles(&self.secret, &guess);
        self.guesses[self.count] = guess;
        self.tiles[self.count] = tiles;
        self.count += 1;
        Some(tiles)
    }

    pub fn solved(&self) -> bool {
        self.count > 0 && self.tiles[self.count - 1] == [Tile::Right; WORD_LEN]
    }

    pub fn over(&self) -> bool {
        self.solved() || self.count == TRIES
    }

    /// The best clue so far for letter `c`, for the alphabet under the guesses.
    pub fn letter(&self, c: u8) -> Option<Tile> {
        (0..self.count).flat_map(|g| (0..WORD_LEN).map(move |i| (g, i))).filter(|&(g, i)| self.guesses[g][i] == c).map(|(g, i)| self.tiles[g][i]).max()
    }
}

/// Wordle's clues: right places first, then each other letter is present only as many times as
/// the word has it left over.
fn tiles(secret: &[u8; WORD_LEN], guess: &[u8; WORD_LEN]) -> [Tile; WORD_LEN] {
    let mut tiles = [Tile::Absent; WORD_LEN];
    let mut left = [0u8; 26];
    for i in 0..WORD_LEN {
        if guess[i] == secret[i] {
            tiles[i] = Tile::Right;
        } else {
            left[(secret[i] - b'A') as usize] += 1;
        }
    }
    for i in 0..WORD_LEN {
        let k = (guess[i] - b'A') as usize;
        if tiles[i] != Tile::Right && left[k] > 0 {
            tiles[i] = Tile::Present;
            left[k] -= 1;
        }
    }
    tiles
}

// FIST --------------------------------------------------------------------------------------

pub const FIST_WORDS: u8 = 10;

/// How one FIST word went.
#[derive(Clone, Copy, Debug)]
pub struct Graded {
    pub score: Score,
    /// You keyed the word right.
    pub right: bool,
    pub good: u8,
    pub off: u8,
    pub bad: u8,
    /// Good intervals in percent; 0 for a wrong word.
    pub percent: u8,
}

pub struct Fist {
    rng: Rng,
    word: Text<MAX_GROUP>,
    counts: [u8; 3],
    pub words: u8,
    /// Percent points over the words so far.
    sum: u16,
}

impl Fist {
    pub fn new(seed: u32) -> Fist {
        Fist { rng: Rng::new(seed), word: Text::new(), counts: [0; 3], words: 0, sum: 0 }
    }

    /// The next word to key: a word heard on the air, or every third one a callsign.
    pub fn next_word(&mut self) -> Text<MAX_GROUP> {
        self.counts = [0; 3];
        self.word = if self.rng.below(3) == 0 {
            let call = home(&mut self.rng).call(&mut self.rng);
            Text::format(format_args!("{}", call.as_str()))
        } else {
            let mut word = Text::new();
            let _ = write!(word, "{}", drill::word(&mut self.rng));
            word
        };
        self.word
    }

    pub fn word(&self) -> &str {
        self.word.as_str()
    }

    /// One timed mark or gap of your keying. Word gaps don't count: they only end the word.
    pub fn interval(&mut self, kind: crate::timing::Kind, units: f32) {
        if kind == crate::timing::Kind::WordGap {
            return;
        }
        let i = match Quality::of(kind, units) {
            Quality::Good => 0,
            Quality::Off => 1,
            Quality::Bad => 2,
        };
        self.counts[i] = self.counts[i].saturating_add(1);
    }

    /// Your keying of the word ended, decoded as `copied`.
    pub fn copied(&mut self, copied: &str) -> Graded {
        let score = compare(self.word.as_str(), copied);
        let right = score.right as usize == score.len && score.extra == 0;
        let [good, off, bad] = self.counts;
        let total = good as u16 + off as u16 + bad as u16;
        let percent = if right && total > 0 { (good as u16 * 100 / total) as u8 } else { 0 };
        self.words += 1;
        self.sum += percent as u16;
        self.counts = [0; 3];
        Graded { score, right, good, off, bad, percent }
    }

    /// The run's score so far: the mean percent of the words keyed.
    pub fn percent(&self) -> u8 {
        if self.words == 0 { 0 } else { (self.sum / self.words as u16) as u8 }
    }

    pub fn over(&self) -> bool {
        self.words >= FIST_WORDS
    }
}

/// Writes the code of each character of `text` after it, for the screens' answers.
pub fn spelled<const N: usize>(text: &str) -> Text<N> {
    let mut out = Text::new();
    for c in morse::chars(text.as_bytes()) {
        let _ = write!(out, "{} ", morse::code(c).unwrap_or("?"));
    }
    out
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::timing::Kind;

    #[test]
    fn ladder_speeds_up_on_right_and_down_on_wrong() {
        let mut l = Ladder::new(20, 3);
        let call = l.next_call();
        let rung = l.copied(call.as_str());
        assert!(rung.right);
        assert_eq!((rung.points, l.wpm), (call.len() as u16 * 20, 22));
        l.next_call();
        let rung = l.copied("X");
        assert!(!rung.right);
        assert_eq!((rung.points, l.wpm, l.calls, l.right), (0, 20, 2, 1));
        let call = l.next_call();
        l.repeated = true;
        assert_eq!(l.copied(call.as_str()).points, call.len() as u16 * 20 / 2, "a repeat scores half");
        let mut slow = Ladder::new(5, 1);
        assert_eq!(slow.wpm, LADDER_MIN_WPM);
        slow.next_call();
        slow.copied("");
        assert_eq!(slow.wpm, LADDER_MIN_WPM);
        while !l.over() {
            let call = l.next_call();
            l.copied(call.as_str());
        }
        assert_eq!(l.calls, LADDER_CALLS);
        assert!(l.wpm <= LADDER_MAX_WPM);
    }

    #[test]
    fn invaders_fall_land_and_die_to_hits() {
        let mut g = Invaders::new(b"ET", 5);
        assert!(g.step(600).spawned, "the first comes after half a second");
        let first = g.slots.iter().flatten().next().copied().unwrap();
        assert_eq!(first.y, 0, "it appears at the top");
        g.step(1000);
        let moved = g.slots.iter().flatten().find(|i| i.c == first.c && i.col == first.col).unwrap();
        assert_eq!(moved.y, (FIELD as u32 * 1000 / 14_000) as u16, "and falls the field in 14 s");
        let c = first.c;
        assert!(g.hit(b'Q').is_none());
        assert!(g.hit(c.to_ascii_lowercase()).is_some());
        assert_eq!((g.score, g.slots.iter().flatten().count()), (10, 0));
        // Unhit, invaders land and cost lives until the game ends.
        let mut landed = 0;
        for _ in 0..2000 {
            landed += g.step(100).landed as u32;
            if g.over() {
                break;
            }
        }
        assert!(g.over());
        assert_eq!(landed, INVADER_LIVES as u32);
        assert_eq!(g.step(100), Step::default(), "nothing moves once it's over");
    }

    #[test]
    fn invaders_hit_the_lowest_and_speed_up_with_the_score() {
        let mut g = Invaders::new(b"E", 1);
        g.slots[0] = Some(Invader { c: b'E', col: 0, y: 100 });
        g.slots[3] = Some(Invader { c: b'E', col: 1, y: 700 });
        assert_eq!(g.hit(b'E'), Some(3));
        let slow = g.speed();
        g.score = 500;
        assert!(g.speed() > slow && g.spawn_every_ms() < 2600);
        g.score = 60_000;
        assert_eq!(g.spawn_every_ms(), 700);
        let mut full = Invaders::new(b"ABCDEFGHIJ", 2);
        for _ in 0..200 {
            full.step(1000);
            full.lives = INVADER_LIVES;
        }
        assert!(full.slots.iter().flatten().all(|i| i.col < INVADER_COLS));
    }

    #[test]
    fn simon_grows_on_right_and_stops_on_wrong() {
        let mut s = Simon::new(b"KMRS", 9);
        assert_eq!(s.len(), 1);
        for n in 1..5 {
            let text = s.text();
            assert_eq!(text.len(), n);
            assert!(s.copied(text.as_str()).1);
        }
        assert_eq!(s.len(), 5);
        let (score, right) = s.copied("KKKKKK");
        assert!(!right && score.len == 5);
        assert_eq!(s.len(), 5, "a miss doesn't grow it");
        while !s.won() {
            let text = s.text();
            s.copied(text.as_str());
        }
        assert_eq!(s.text().len(), SIMON_MAX);
    }

    #[test]
    fn wordle_clues_count_repeated_letters() {
        use Tile::*;
        assert_eq!(tiles(b"MORSE", b"MORSE"), [Right; 5]);
        assert_eq!(tiles(b"MORSE", b"SMORE"), [Present, Present, Present, Present, Right]);
        assert_eq!(tiles(b"APPLE", b"PAPAL"), [Present, Present, Right, Absent, Present]);
        assert_eq!(tiles(b"SPEED", b"EERIE"), [Present, Present, Absent, Absent, Absent]);
        let mut w = Wordle::new(4);
        assert_eq!(w.secret().len(), WORD_LEN);
        assert!(w.guess("TOO LONGG").is_none());
        assert!(w.guess("ABC").is_none());
        assert!(w.guess("AB1DE").is_none());
        assert_eq!(w.count, 0);
        let secret: std::string::String = w.secret().into();
        let wrong = if secret == "ZZZZZ" { "YYYYY" } else { "ZZZZZ" };
        w.guess(wrong).unwrap();
        assert_eq!(w.letter(b'Z').is_some(), wrong == "ZZZZZ");
        w.guess(&secret.to_lowercase()).unwrap();
        assert!(w.solved() && w.over());
        assert!(w.guess(&secret).is_none(), "no guesses after it's solved");
        w.new_word();
        for _ in 0..TRIES {
            w.guess("QQQQQ");
        }
        assert!(w.over() && (w.secret() == "QQQQQ") == w.solved());
    }

    #[test]
    fn every_wordle_word_has_five_letters() {
        let five = WORDLE_WORDS.iter().filter(|w| w.len() == WORD_LEN && w.bytes().all(|c| c.is_ascii_uppercase())).count();
        assert!(five >= 100, "{five}");
    }

    #[test]
    fn fist_grades_timing_of_right_words_only() {
        let mut f = Fist::new(2);
        let word = f.next_word();
        assert!(!word.as_str().is_empty());
        for _ in 0..6 {
            f.interval(Kind::Dit, 1.0);
        }
        f.interval(Kind::Dah, 2.0);
        f.interval(Kind::CharGap, 3.0);
        f.interval(Kind::WordGap, 9.0);
        let g = f.copied(word.as_str());
        assert!(g.right);
        assert_eq!((g.good, g.off + g.bad, g.percent), (7, 1, 87));
        f.next_word();
        f.interval(Kind::Dit, 1.0);
        let g = f.copied("ZZZZ");
        assert_eq!((g.right, g.percent), (false, 0));
        assert_eq!(f.percent(), 43);
        while !f.over() {
            f.next_word();
            f.copied("");
        }
        assert_eq!(f.words, FIST_WORDS);
    }

    #[test]
    fn spelled_writes_codes() {
        assert_eq!(spelled::<32>("SOS").as_str(), "... --- ... ");
    }
}
