//! GAMES: DASH, LADDER, INVADERS, SIMON, WORDLE and FIST. The rules live in `pocketcw_logic::games`;
//! this draws them and feeds them your keying. Each game keeps its best score in the settings.

use core::fmt::Write;

use pocketcw_logic::{
    dash::{self, Dash},
    decoder::{Decoded, copy_end},
    drill::{MAX_COPY, MAX_GROUP, Mark, Score},
    games::{
        FIELD, Fist, Game, INVADER_COLS, INVADER_LIVES, INVADER_SLOTS, Invaders, LADDER_CALLS, Ladder, Simon, TRIES, Tile, WORD_LEN,
        Wordle, spelled,
    },
    input::Action,
    player::{Keying, MAX_TEXT},
    settings::Settings,
    timing::Interval,
};

use super::footer::{FootLine, INPUT_COLS, INPUT_Y};
use super::dash::{SCENE_TOP, View};
use super::{Play, Screen, Session, Ui, draw_char, push_scrolling, settings::Section};
use crate::fonts::{SMALL, TINY};
use crate::gfx::{CHAR_W, TINY_W, ink, rect, text_cols, tiny};
use crate::lcd::Lcd;
use crate::text::Text;
use crate::theme::{BAD, GOOD, MAIN, Theme, WARN};

const INFO_Y: i32 = 26;
const SUB_Y: i32 = 42;
const RULE_Y: i32 = 66;
const ROWS: usize = 7;
const ROW_Y: i32 = 70;
const ROW_STEP: i32 = 18;
const COPY_X: i32 = 120;
/// Time to start keying after the device sends, before it counts as nothing copied.
const START_MS: u32 = 8000;
const NEXT_MS: u32 = 900;
const PLAY_DELAY_MS: u32 = 400;
/// INVADERS: the field, and how often it moves.
const FIELD_TOP: i32 = 44;
const GROUND_Y: i32 = 198;
const TICK_MS: u32 = 80;
/// DASH frames.
const DASH_TICK_MS: u32 = 33;
const COL_W: i32 = MAIN / INVADER_COLS as i32;
/// WORDLE tiles.
const TILE_W: i32 = 22;
const TILE_X: i32 = 70;

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    /// The device sends: LADDER's call, SIMON's sequence.
    Playing,
    /// Your turn to key.
    Waiting,
    Copying,
    /// A result shows; the next turn starts at the deadline.
    Between,
    /// INVADERS moving.
    Running,
    Paused,
    Over,
}

enum Engine {
    Ladder(Ladder),
    Invaders(Invaders),
    Simon(Simon),
    Wordle(Wordle),
    Fist(Fist),
    Dash(Dash, View),
}

#[derive(Clone, Copy)]
struct Line {
    sent: Text<MAX_GROUP>,
    marks: [Mark; MAX_GROUP],
    len: usize,
    note: Text<14>,
    good: bool,
}

pub(super) struct Arcade {
    game: Game,
    engine: Engine,
    state: State,
    pub(super) deadline: Option<u32>,
    copy: Text<MAX_COPY>,
    lines: [Option<Line>; ROWS],
    /// INVADERS: when the field last moved, and where each slot's invader was drawn.
    last_ms: u32,
    drawn: [Option<(i32, i32)>; INVADER_SLOTS],
    /// WORDLE: words solved in a row this session.
    streak: u16,
    /// The game that just ended set a new best.
    new_best: bool,
    /// DASH hides the character and sends it.
    hear: bool,
}

impl Arcade {
    pub(super) fn game(&self) -> Game {
        self.game
    }

    pub(super) fn badge(&self) -> &'static str {
        match self.state {
            State::Playing => "LISTEN",
            State::Waiting => "YOUR TURN",
            State::Copying => "KEYING",
            State::Running if matches!(self.engine, Engine::Dash(..)) => "RUN!",
            State::Between | State::Running => "",
            State::Paused => "PAUSED",
            State::Over => "GAME OVER",
        }
    }

    pub(super) fn pause(&mut self) {
        if self.state != State::Over {
            self.state = State::Paused;
        }
        self.deadline = None;
        self.copy.clear();
    }

    fn row(&mut self, sent: &str, score: Score, note: Text<14>, good: bool) {
        let line = Line { sent: Text::format(format_args!("{sent}")), marks: score.marks, len: score.len, note, good };
        push_scrolling(&mut self.lines, line);
    }
}

fn engine(game: Game, s: &Settings, pool: &'static [u8], seed: u32, t: &Theme, hear: bool) -> Engine {
    match game {
        Game::Dash => {
            // Only what the bubbles can draw: letters and digits.
            let mut chars = [0u8; 44];
            let mut n = 0;
            for &c in pool.iter().filter(|c| c.is_ascii_alphanumeric()).take(chars.len()) {
                chars[n] = c;
                n += 1;
            }
            let chars: &[u8] = if n == 0 { b"ETANIM" } else { &chars[..n] };
            let mode = if hear { dash::Mode::Hear } else { dash::Mode::See };
            Engine::Dash(Dash::new(chars, mode, seed), View::new(t))
        }
        Game::Ladder => Engine::Ladder(Ladder::new(s.receive_speed(s.wpm).0, seed)),
        Game::Invaders => Engine::Invaders(Invaders::new(pool, seed)),
        Game::Simon => Engine::Simon(Simon::new(pool, seed)),
        Game::Wordle => Engine::Wordle(Wordle::new(seed)),
        Game::Fist => Engine::Fist(Fist::new(seed)),
    }
}

impl Ui {
    fn pool(&self, s: &Settings) -> &'static [u8] {
        s.chars_set.chars(self.letters.level)
    }

    pub(super) fn open_game(&mut self, lcd: &mut Lcd, s: &mut Settings, game: Game) {
        self.open_game_with(lcd, s, game, false);
    }

    fn open_game_with(&mut self, lcd: &mut Lcd, s: &mut Settings, game: Game, hear: bool) {
        let seed = self.next_seed();
        let pool = self.pool(s);
        let arcade = Arcade {
            game,
            engine: engine(game, s, pool, seed, &self.theme, hear),
            state: State::Between,
            deadline: None,
            copy: Text::new(),
            lines: [None; ROWS],
            last_ms: self.now_ms,
            drawn: [None; INVADER_SLOTS],
            streak: 0,
            new_best: false,
            hear,
        };
        self.session = Some(Session::Game(arcade));
        self.open(lcd, s, Screen::Game);
        self.game_turn(lcd, s);
    }

    /// Back from the menu or settings: a paused game goes on; a finished one stays finished.
    pub(super) fn game_resume(&mut self, lcd: &mut Lcd, s: &Settings) {
        self.open(lcd, s, Screen::Game);
        if let Some(Session::Game(a)) = &self.session
            && a.state == State::Paused
        {
            self.game_turn(lcd, s);
        }
    }

    /// A new game of the same kind.
    fn game_restart(&mut self, lcd: &mut Lcd, s: &mut Settings) {
        let Some(Session::Game(a)) = &self.session else { return };
        let game = a.game;
        let streak = a.streak;
        let hear = a.hear;
        self.open_game_with(lcd, s, game, hear);
        if let Some(Session::Game(a)) = &mut self.session {
            a.streak = streak;
        }
    }

    /// The next turn: send the next call or sequence, show the next word, or start the field.
    fn game_turn(&mut self, lcd: &mut Lcd, s: &Settings) {
        let seed = self.next_seed();
        let now = self.now_ms;
        let Some(Session::Game(a)) = &mut self.session else { return };
        a.copy.clear();
        a.deadline = None;
        let send = match &mut a.engine {
            Engine::Ladder(l) => {
                let call = l.next_call();
                Some((call, Keying { wpm: l.wpm, effective: l.wpm, fist: 0 }))
            }
            Engine::Simon(simon) => {
                let (wpm, effective) = s.receive_speed(s.wpm);
                Some((simon.text(), Keying { wpm, effective, fist: 0 }))
            }
            Engine::Fist(f) => {
                f.next_word();
                None
            }
            Engine::Wordle(w) => {
                if w.over() {
                    w.new_word();
                    a.lines = [None; ROWS];
                }
                None
            }
            Engine::Invaders(_) | Engine::Dash(..) => {
                a.state = State::Running;
                a.last_ms = now;
                a.deadline = Some(now.wrapping_add(TICK_MS));
                self.draw(lcd, s);
                return;
            }
        };
        match send {
            Some((text, keying)) => {
                a.state = State::Playing;
                let text = Text::<MAX_TEXT>::format(format_args!("{}", text.as_str()));
                self.play = Some(Play { text, keying, seed, delay_ms: PLAY_DELAY_MS, pitch_hz: s.tone_hz, transmit: false });
            }
            None => a.state = State::Waiting,
        }
        self.draw(lcd, s);
    }

    pub(super) fn draw_game(&mut self, lcd: &mut Lcd, s: &Settings) {
        let Some(Session::Game(a)) = &self.session else { return };
        self.header(lcd, s, a.game.name(), &[]);
        self.draw_game_keys(lcd);
        let t = &self.theme;
        if !matches!(a.engine, Engine::Invaders(_) | Engine::Dash(..)) {
            rect(lcd, 0, RULE_Y, MAIN, 1, t.rule());
        }
        self.draw_game_info(lcd, s);
        self.draw_game_body(lcd, true);
        self.draw_game_now(lcd);
    }

    fn draw_game_keys(&self, lcd: &mut Lcd) {
        let Some(Session::Game(a)) = &self.session else { return };
        let again = match (&a.engine, a.state) {
            (Engine::Dash(..), State::Paused | State::Over) => if a.hear { "HEAR" } else { "SEE" },
            (Engine::Dash(..), State::Running) if a.hear => "AGN",
            (Engine::Ladder(_), State::Playing | State::Waiting | State::Copying) => "AGN",
            (Engine::Simon(_), State::Waiting) => "AGN",
            _ => "",
        };
        let go = if matches!(a.state, State::Paused | State::Over) { "GO" } else { "STOP" };
        self.softkeys(lcd, ["SET", again, go, "BACK"]);
    }

    /// The score line, and under it what the game asks for now.
    fn draw_game_info(&self, lcd: &mut Lcd, s: &Settings) {
        let Some(Session::Game(a)) = &self.session else { return };
        let t = &self.theme;
        let bottom = match a.engine {
            Engine::Invaders(_) => FIELD_TOP,
            Engine::Dash(..) => SCENE_TOP,
            _ => RULE_Y,
        };
        rect(lcd, 0, INFO_Y, MAIN, bottom - INFO_Y, t.bg);
        let best = s.bests[a.game.index()];
        let mut info = Text::<40>::new();
        let mut sub = Text::<40>::new();
        match &a.engine {
            Engine::Dash(d, _) => {
                let _ = write!(info, "SCORE {} LV {} LIVES ", d.score, d.level());
                (0..dash::LIVES).for_each(|i| _ = info.push(if i < d.lives { b'*' } else { b'-' }));
                let _ = write!(info, " {}", if a.hear { "HEAR" } else { "SEE" });
            }
            Engine::Ladder(l) => {
                let _ = write!(info, "CALL {}/{}  {} WPM  {} PTS", l.calls.min(LADDER_CALLS - 1) + 1, LADDER_CALLS, l.wpm, l.points);
                let _ = write!(sub, "BEST {best} PTS  RIGHT SPEEDS IT UP");
            }
            Engine::Invaders(g) => {
                // At most 36 characters, to fit left of the keys.
                let _ = write!(info, "SCORE {} LV {} LIVES ", g.score, g.level());
                (0..INVADER_LIVES).for_each(|i| _ = info.push(if i < g.lives { b'*' } else { b'-' }));
                let _ = write!(info, " BEST {best}");
            }
            Engine::Simon(simon) => {
                let _ = write!(info, "LENGTH {}  BEST {best}", simon.len());
                let _ = write!(sub, "KEY BACK THE WHOLE SEQUENCE");
            }
            Engine::Wordle(w) => {
                let _ = write!(info, "TRY {}/{}  STREAK {}  BEST {best}", (w.count + 1).min(TRIES), TRIES, a.streak);
                self.draw_alphabet(lcd, w);
            }
            Engine::Fist(f) => {
                let _ = write!(info, "WORD {}/10  RUN {}%  BEST {best}%", (f.words + 1).min(10), f.percent());
                if matches!(a.state, State::Waiting | State::Copying) {
                    let word = f.word();
                    let code = spelled::<40>(word);
                    let _ = write!(sub, "KEY  {word}  ");
                    if sub.len() + code.len() <= 38 {
                        let _ = sub.write_str(code.as_str());
                    }
                }
            }
        }
        tiny(lcd, 2, INFO_Y, info.as_str(), t.muted, t.bg);
        if !sub.as_str().is_empty() {
            let fg = if matches!(a.engine, Engine::Fist(_)) { t.fg } else { t.muted };
            tiny(lcd, 2, SUB_Y, sub.as_str(), fg, t.bg);
        }
    }

    /// WORDLE: the alphabet, each letter colored by its best clue so far.
    fn draw_alphabet(&self, lcd: &mut Lcd, w: &Wordle) {
        let t = &self.theme;
        for (i, c) in (b'A'..=b'Z').enumerate() {
            let (fg, bg) = match w.letter(c) {
                None => (t.fg, t.bg),
                Some(Tile::Right) => (ink(GOOD), GOOD),
                Some(Tile::Present) => (ink(WARN), WARN),
                Some(Tile::Absent) => (t.rule(), t.bg),
            };
            draw_char(lcd, &TINY, 2 + i as i32 * (TINY_W + 3), SUB_Y + 2, c, fg, bg);
        }
    }

    /// The rows, the WORDLE grid, or the INVADERS field. `clear` redraws from empty.
    fn draw_game_body(&mut self, lcd: &mut Lcd, clear: bool) {
        let Some(Session::Game(a)) = &mut self.session else { return };
        let t = &self.theme;
        match &mut a.engine {
            Engine::Dash(d, view) => {
                if clear {
                    // The theme may have changed since.
                    *view = View::new(t);
                }
                view.frame(lcd, d, clear);
            }
            Engine::Invaders(g) => {
                if clear {
                    rect(lcd, 0, FIELD_TOP, MAIN, GROUND_Y - FIELD_TOP, t.bg);
                    a.drawn = [None; INVADER_SLOTS];
                    rect(lcd, 0, GROUND_Y, MAIN, 2, t.accent);
                }
                for (slot, inv) in g.slots.iter().enumerate() {
                    let at = inv.map(|inv| {
                        let x = inv.col as i32 * COL_W + (COL_W - CHAR_W) / 2;
                        let y = FIELD_TOP + (inv.y as i32 * (GROUND_Y - FIELD_TOP - 18) / FIELD as i32);
                        (x, y)
                    });
                    if at == a.drawn[slot] {
                        continue;
                    }
                    if let Some((x, y)) = a.drawn[slot] {
                        rect(lcd, x - 2, y, 2 * CHAR_W + 4, 16, t.bg);
                    }
                    if let (Some((x, y)), Some(inv)) = (at, inv) {
                        let w = draw_char(lcd, &SMALL, x, y, inv.c, ink(t.accent), t.accent);
                        rect(lcd, x - 2, y, 2, 16, t.accent);
                        rect(lcd, x + w, y, 2, 16, t.accent);
                    }
                    a.drawn[slot] = at;
                }
            }
            Engine::Wordle(w) => {
                rect(lcd, 0, ROW_Y - 4, MAIN, ROWS as i32 * ROW_STEP + 4, t.bg);
                for g in 0..TRIES {
                    let y = ROW_Y - 4 + g as i32 * (ROW_STEP + 1);
                    for i in 0..WORD_LEN {
                        let x = TILE_X + i as i32 * (TILE_W + 4);
                        if g >= w.count {
                            rect(lcd, x, y, TILE_W, 1, t.rule());
                            rect(lcd, x, y + ROW_STEP - 2, TILE_W, 1, t.rule());
                            continue;
                        }
                        let bg = match w.tiles[g][i] {
                            Tile::Right => GOOD,
                            Tile::Present => WARN,
                            Tile::Absent => t.rule(),
                        };
                        rect(lcd, x, y, TILE_W, ROW_STEP - 1, bg);
                        draw_char(lcd, &SMALL, x + (TILE_W - CHAR_W) / 2, y, w.guesses[g][i], ink(bg), bg);
                    }
                }
            }
            _ => {
                for (k, line) in a.lines.iter().enumerate() {
                    let y = ROW_Y + k as i32 * ROW_STEP;
                    rect(lcd, 0, y, MAIN, ROW_STEP, t.bg);
                    if let Some(line) = line {
                        draw_line(lcd, t, y, line);
                    }
                }
            }
        }
    }

    /// Your keying so far, and the footer.
    fn draw_game_now(&mut self, lcd: &mut Lcd) {
        let Some(Session::Game(a)) = &self.session else { return };
        let t = &self.theme;
        let copy = a.copy.as_str();
        let shown = &copy[copy.len().saturating_sub(INPUT_COLS - 1)..];
        text_cols(lcd, 0, INPUT_Y, shown, INPUT_COLS, t.fg, t.bg);
        if matches!(a.state, State::Waiting | State::Copying) {
            rect(lcd, shown.len() as i32 * CHAR_W + 1, INPUT_Y + 2, CHAR_W - 2, 13, t.accent);
        }
        let foot = match (a.state, &a.engine) {
            (State::Paused, _) => "PAUSED: TOUCH THE KEY OR PRESS GO",
            (State::Over, Engine::Dash(..)) => "GO: RUN AGAIN  F2: SEE OR HEAR",
            (State::Over, _) => "GO OR TOUCH THE KEY: PLAY AGAIN",
            (_, Engine::Dash(..)) if a.hear => "LISTEN, KEY IT TO JUMP  F2: AGAIN",
            (_, Engine::Dash(..)) => "KEY THE LETTER TO JUMP IT",
            (_, Engine::Ladder(_)) => "COPY THE CALL AND KEY IT BACK",
            (_, Engine::Invaders(_)) => "KEY A CHARACTER TO SHOOT IT",
            (_, Engine::Simon(_)) => "KEY BACK THE WHOLE SEQUENCE",
            (_, Engine::Wordle(_)) => "KEY A 5-LETTER WORD  GREEN: RIGHT SPOT",
            (_, Engine::Fist(_)) => "KEY THE WORD WITH YOUR BEST TIMING",
        };
        self.set_footer(lcd, foot);
    }

    pub(super) fn game_played(&mut self, lcd: &mut Lcd, now_ms: u32) {
        let Some(Session::Game(a)) = &mut self.session else { return };
        if a.state == State::Playing {
            a.state = State::Waiting;
            a.deadline = Some(now_ms.wrapping_add(START_MS));
            self.draw_game_now(lcd);
        }
    }

    pub(super) fn game_key(&mut self, lcd: &mut Lcd, down: bool, at_ms: u32, dit_ms: u16) {
        let Some(Session::Game(a)) = &mut self.session else { return };
        let was = a.state;
        match (a.state, down) {
            (State::Waiting, true) => {
                a.state = State::Copying;
                a.deadline = None;
            }
            (State::Copying, true) => a.deadline = None,
            (State::Copying, false) => a.deadline = Some(copy_end(at_ms, dit_ms)),
            _ => {}
        }
        if a.state != was {
            self.draw_game_now(lcd);
        }
    }

    /// FIST: one timed mark or gap of your keying.
    pub(super) fn game_interval(&mut self, iv: &Interval) {
        if let Some(Session::Game(a)) = &mut self.session
            && let Engine::Fist(f) = &mut a.engine
            && a.state == State::Copying
        {
            f.interval(iv.kind, iv.units);
        }
    }

    pub(super) fn game_decoded(&mut self, lcd: &mut Lcd, s: &mut Settings, decoded: Decoded) {
        let Some(Session::Game(a)) = &mut self.session else { return };
        match a.state {
            // The character that ends a pause or a finished game starts it again, and is not copy.
            State::Paused | State::Over if decoded != Decoded::Space => {
                if a.state == State::Over {
                    return self.game_restart(lcd, s);
                }
                return self.game_go(lcd, s);
            }
            State::Running => {
                let Decoded::Char(c) = decoded else { return };
                let hit = match &mut a.engine {
                    Engine::Invaders(g) => g.hit(c).is_some(),
                    Engine::Dash(d, _) => d.key(c) == dash::Key::Cleared,
                    _ => return,
                };
                let _ = decoded.write_to(&mut a.copy);
                if a.copy.len() > INPUT_COLS - 4 {
                    a.copy.clear();
                    let _ = decoded.write_to(&mut a.copy);
                }
                if hit {
                    self.draw_game_body(lcd, false);
                    self.draw_game_info(lcd, s);
                }
                self.draw_game_now(lcd);
            }
            State::Waiting | State::Copying => {
                if decoded.write_to(&mut a.copy) {
                    self.draw_game_now(lcd);
                }
            }
            _ => {}
        }
    }

    pub(super) fn game_timeout(&mut self, lcd: &mut Lcd, s: &mut Settings, now_ms: u32) {
        let Some(Session::Game(a)) = &mut self.session else { return };
        a.deadline = None;
        match a.state {
            State::Between => self.game_turn(lcd, s),
            State::Running if matches!(a.engine, Engine::Dash(..)) => self.dash_tick(lcd, s, now_ms),
            State::Running => self.invaders_tick(lcd, s, now_ms),
            State::Waiting | State::Copying => self.game_copied(lcd, s, now_ms),
            _ => {}
        }
    }

    fn dash_tick(&mut self, lcd: &mut Lcd, s: &mut Settings, now_ms: u32) {
        let seed = self.next_seed();
        let Some(Session::Game(a)) = &mut self.session else { return };
        let Engine::Dash(d, view) = &mut a.engine else { return };
        let events = d.step(now_ms.wrapping_sub(a.last_ms).min(200));
        a.last_ms = now_ms;
        view.frame(lcd, d, false);
        let (over, lives) = (d.over(), d.lives);
        if !over {
            a.deadline = Some(now_ms.wrapping_add(DASH_TICK_MS));
        }
        // HEAR: the new obstacle's character, sent as it comes on screen.
        if a.hear
            && let Some(c) = events.spawned
        {
            self.send_char(s, c, seed);
        }
        if events.crashed {
            self.draw_game_info(lcd, s);
            if !over {
                self.status(lcd, FootLine::format(format_args!("OUCH!  LIVES LEFT {lives}")).as_str());
            }
        }
        if over {
            self.game_over(lcd, s);
        }
    }

    /// One character at the receive speed, as DASH's HEAR sends it.
    fn send_char(&mut self, s: &Settings, c: u8, seed: u32) {
        let (wpm, effective) = s.receive_speed(s.wpm);
        let mut text = Text::<MAX_TEXT>::new();
        let _ = text.push(c);
        self.play = Some(Play { text, keying: Keying { wpm, effective, fist: 0 }, seed, delay_ms: 0, pitch_hz: s.tone_hz, transmit: false });
    }

    fn invaders_tick(&mut self, lcd: &mut Lcd, s: &mut Settings, now_ms: u32) {
        let Some(Session::Game(a)) = &mut self.session else { return };
        let Engine::Invaders(g) = &mut a.engine else { return };
        let step = g.step(now_ms.wrapping_sub(a.last_ms).min(500));
        a.last_ms = now_ms;
        let over = g.over();
        let lives = g.lives;
        if !over {
            a.deadline = Some(now_ms.wrapping_add(TICK_MS));
        }
        self.draw_game_body(lcd, false);
        if step.landed > 0 {
            self.draw_game_info(lcd, s);
            if !over {
                self.status(lcd, FootLine::format(format_args!("ONE LANDED!  LIVES LEFT {lives}")).as_str());
            }
        }
        if over {
            self.game_over(lcd, s);
        }
    }

    /// Your copy ended, or never started in time: score it.
    fn game_copied(&mut self, lcd: &mut Lcd, s: &mut Settings, now_ms: u32) {
        let Some(Session::Game(a)) = &mut self.session else { return };
        let copy = a.copy;
        let copy = copy.as_str();
        let mut over = false;
        let mut next = true;
        match &mut a.engine {
            Engine::Ladder(l) => {
                let call = Text::<MAX_GROUP>::format(format_args!("{}", l.call()));
                let rung = l.copied(copy);
                let note = Text::format(format_args!("+{} {}W", rung.points, rung.wpm));
                over = l.over();
                a.row(call.as_str(), rung.score, note, rung.right);
            }
            Engine::Simon(simon) => {
                let sent = simon.text();
                let (score, right) = simon.copied(copy);
                let note = Text::format(format_args!("{}", if right { "RIGHT" } else { "MISSED" }));
                over = !right || simon.won();
                a.row(sent.as_str(), score, note, right);
            }
            Engine::Fist(f) => {
                let word = Text::<MAX_GROUP>::format(format_args!("{}", f.word()));
                let g = f.copied(copy);
                let note = if g.right {
                    Text::format(format_args!("{}% {}/{}", g.percent, g.good, g.good + g.off + g.bad))
                } else {
                    Text::format(format_args!("MISKEYED"))
                };
                over = f.over();
                a.row(word.as_str(), g.score, note, g.right && g.percent >= 80);
            }
            Engine::Wordle(w) => {
                next = false;
                match w.guess(copy) {
                    None if copy.trim().is_empty() => {}
                    None => self.status(lcd, "KEY A WORD OF 5 LETTERS"),
                    Some(_) => {
                        over = w.over();
                        if over {
                            a.streak = if w.solved() { a.streak + 1 } else { 0 };
                        }
                    }
                }
                let Some(Session::Game(a)) = &mut self.session else { return };
                a.copy.clear();
                if !over {
                    a.state = State::Waiting;
                    self.draw_game_info(lcd, s);
                    self.draw_game_body(lcd, false);
                    self.draw_game_now(lcd);
                    return;
                }
            }
            Engine::Invaders(_) | Engine::Dash(..) => return,
        }
        if over {
            return self.game_over(lcd, s);
        }
        if next {
            let Some(Session::Game(a)) = &mut self.session else { return };
            a.copy.clear();
            a.state = State::Between;
            a.deadline = Some(now_ms.wrapping_add(NEXT_MS));
            self.draw_game_info(lcd, s);
            self.draw_game_body(lcd, false);
            self.draw_game_now(lcd);
        }
    }

    /// The game ended: keep a new best, and say how it went.
    fn game_over(&mut self, lcd: &mut Lcd, s: &mut Settings) {
        let Some(Session::Game(a)) = &mut self.session else { return };
        let (score, line) = match &a.engine {
            Engine::Ladder(l) => (l.points, FootLine::format(format_args!("{} PTS, {} OF {} RIGHT", l.points, l.right, LADDER_CALLS))),
            Engine::Invaders(g) => (g.score, FootLine::format(format_args!("THEY LANDED: {} PTS, LEVEL {}", g.score, g.level()))),
            Engine::Dash(d, _) => (d.score, FootLine::format(format_args!("CRASHED OUT: {} PTS, {} JUMPS", d.score, d.cleared))),
            Engine::Simon(simon) => {
                let longest = if simon.won() { simon.len() } else { simon.len() - 1 };
                (longest as u16, FootLine::format(format_args!("YOU KEYED BACK {longest} IN A ROW")))
            }
            Engine::Wordle(w) if w.solved() => (a.streak, FootLine::format(format_args!("GOT IT IN {}! STREAK {}", w.count, a.streak))),
            Engine::Wordle(w) => (a.streak, FootLine::format(format_args!("THE WORD WAS {}", w.secret()))),
            Engine::Fist(f) => (f.percent() as u16, FootLine::format(format_args!("YOUR FIST: {}% CLEAN TIMING", f.percent()))),
        };
        let best = &mut s.bests[a.game.index()];
        a.new_best = score > *best;
        if a.new_best {
            *best = score;
        }
        a.state = State::Over;
        a.deadline = None;
        let new_best = a.new_best;
        self.stop_players();
        self.draw(lcd, s);
        let mut line = line;
        if new_best {
            let _ = line.write_str("  NEW BEST!");
        }
        self.status(lcd, line.as_str());
    }

    /// GO after a pause: the game goes on with a new turn.
    fn game_go(&mut self, lcd: &mut Lcd, s: &mut Settings) {
        let Some(Session::Game(a)) = &mut self.session else { return };
        // WORDLE keeps its guesses; it waits for the next one.
        if let Engine::Wordle(_) = a.engine {
            a.state = State::Waiting;
            return self.draw(lcd, s);
        }
        self.game_turn(lcd, s);
    }

    pub(super) fn game_act(&mut self, lcd: &mut Lcd, s: &mut Settings, action: Action) {
        let Some(Session::Game(a)) = &mut self.session else { return };
        match action {
            Action::Soft(0) => self.open_settings_at(lcd, s, Section::Receive),
            Action::Soft(1) => match &mut a.engine {
                Engine::Dash(d, _) if matches!(a.state, State::Paused | State::Over) => {
                    a.hear = !a.hear;
                    d.mode = if a.hear { dash::Mode::Hear } else { dash::Mode::See };
                    self.draw_game_keys(lcd);
                    self.draw_game_info(lcd, s);
                    self.draw_game_now(lcd);
                }
                Engine::Dash(d, _) if a.hear && a.state == State::Running => {
                    if let Some(c) = d.next_char() {
                        let seed = self.next_seed();
                        self.stop_players();
                        self.send_char(s, c, seed);
                    }
                }
                Engine::Ladder(l) if matches!(a.state, State::Playing | State::Waiting | State::Copying) => {
                    l.repeated = true;
                    let text = Text::<MAX_TEXT>::format(format_args!("{}", l.call()));
                    let keying = Keying { wpm: l.wpm, effective: l.wpm, fist: 0 };
                    a.state = State::Playing;
                    a.copy.clear();
                    a.deadline = None;
                    self.replay(lcd, s, text, keying);
                }
                Engine::Simon(simon) if a.state == State::Waiting => {
                    let text = Text::<MAX_TEXT>::format(format_args!("{}", simon.text().as_str()));
                    let (wpm, effective) = s.receive_speed(s.wpm);
                    a.state = State::Playing;
                    a.deadline = None;
                    self.replay(lcd, s, text, Keying { wpm, effective, fist: 0 });
                }
                _ => {}
            },
            Action::Soft(2) => match a.state {
                State::Over => self.game_restart(lcd, s),
                State::Paused => self.game_go(lcd, s),
                _ => {
                    a.pause();
                    self.stop_players();
                    self.draw(lcd, s);
                }
            },
            Action::Soft(3) => self.back(lcd, s),
            _ => {}
        }
    }

    fn replay(&mut self, lcd: &mut Lcd, s: &Settings, text: Text<MAX_TEXT>, keying: Keying) {
        self.stop_players();
        let seed = self.next_seed();
        self.play = Some(Play { text, keying, seed, delay_ms: PLAY_DELAY_MS, pitch_hz: s.tone_hz, transmit: false });
        self.draw_game_keys(lcd);
        self.draw_game_now(lcd);
    }
}

/// A row: what was sent, your copy marked against it, and a note at the right end.
fn draw_line(lcd: &mut Lcd, t: &Theme, y: i32, line: &Line) {
    let mut x = 4;
    for c in line.sent.as_str().bytes() {
        x += draw_char(lcd, &SMALL, x, y, c, t.muted, t.bg);
    }
    let mut x = COPY_X;
    let sent = pocketcw_logic::morse::chars(line.sent.as_str().as_bytes()).filter(|&c| c != b' ');
    for (mark, sent) in line.marks[..line.len].iter().zip(sent) {
        let (c, fg, bg) = match *mark {
            Mark::Right => (sent, GOOD, t.bg),
            Mark::Wrong(c) => (c, ink(BAD), BAD),
            Mark::Missed => (b'_', ink(BAD), BAD),
        };
        x += draw_char(lcd, &SMALL, x, y, c, fg, bg);
    }
    let color = if line.good { GOOD } else { t.muted };
    tiny(lcd, MAIN - 4 - line.note.len() as i32 * TINY_W, y + 3, line.note.as_str(), color, t.bg);
}
