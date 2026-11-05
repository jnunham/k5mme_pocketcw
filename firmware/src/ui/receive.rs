//! Receive drills: the device sends a group at the receive speed, you key back what you copied,
//! and the group shows under your copy with each character marked.

use core::fmt::Write;
use core::ops::RangeInclusive;

use pocketcw_logic::{
    decoder::{Decoded, copy_end},
    dial::Target,
    drill::{Drill, Item, Kind, MAX_COPY, MAX_GROUP, Mark, RUNGS},
    input::Action,
    letters::{KOCH, Mastery},
    mania::{self, CHOICES, Mania, ROUND},
    morse,
    pack::WORD_TEXT,
    player::{Keying, MAX_TEXT},
    settings::Settings,
};

use super::footer::{FootLine, INPUT_COLS, INPUT_Y};
use super::{COLS, Play, Screen, Session, Ui, char_cells, draw_char, push_scrolling, settings::Section};
use crate::fonts::{SMALL, TINY};
use crate::gfx::{CHAR_W, TINY_W, ink, rect, text_cols, tiny};
use crate::lcd::Lcd;
use crate::pack::Pack;
use crate::text::Text;
use crate::theme::{BAD, GOOD, MAIN, Theme, WARN};

const INFO_Y: i32 = 26;
const SET_Y: i32 = 40;
const SET_PER_ROW: usize = 38;
/// One more than TINY's height, so a prosign bar on the second row clears the first row.
const SET_STEP: i32 = 13;
const RULE_Y: i32 = 66;
const ROWS: usize = 7;
const ROW_Y: i32 = 70;
const ROW_STEP: i32 = 18;
const COPY_X: i32 = 110;
/// Cells of sent text left of `COPY_X`.
const SENT_CELLS: usize = 10;
/// HEAD items shown, each on two rows: sent, then your copy.
const HEAD_ITEMS: usize = ROWS / 2;
const _: () = assert!(ROW_Y + ROWS as i32 * ROW_STEP <= INPUT_Y);
/// Time after a group ends to start keying it back before it counts as missed.
const START_MS: u32 = 5000;
/// NINJA's time to start keying before the voice says the word.
const THINK_MS: u32 = 1500;
/// Pack words to try for one that fits a group.
const PICK_TRIES: usize = 20;
const NEXT_GROUP_MS: u32 = 900;
/// CHARS: time the hint stays up after its character plays again, before the next one.
const HINT_HOLD_MS: u32 = 1500;
const GROUP_DELAY_MS: u32 = 400;

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Playing,
    /// The group ended; waiting for you to start keying.
    Waiting,
    Copying,
    /// Scored, or for NINJA said; the next group starts at the deadline.
    Between,
    /// CHARS: the hint shows, and the character plays again. MANIA: the right answer to a wrong
    /// pick shows, and plays again.
    Hint,
    Paused,
    /// MANIA: a round ended; its result shows until GO.
    RoundOver,
    /// NINJA or HEAD with no content pack, or no pack word fits.
    NoPack,
}

#[derive(Clone, Copy)]
struct Row {
    sent: Text<MAX_GROUP>,
    marks: [Mark; MAX_GROUP],
    right: u8,
    /// False for a NINJA word you only listened to.
    scored: bool,
    /// CHARS: you asked for a hint, so the row shows the answer and its code.
    hinted: bool,
    /// MANIA: your pick, marked, then the code of what was sent.
    mania: bool,
}

pub(super) struct Receive {
    drill: Drill,
    group: Text<MAX_GROUP>,
    /// The NINJA word as the pack spells it, for the voice: `QTH`, `<KN>`.
    word: Text<WORD_TEXT>,
    copy: Text<MAX_COPY>,
    state: State,
    pub(super) deadline: Option<u32>,
    rows: [Option<Row>; ROWS],
    session_right: u16,
    session_total: u16,
    /// The last group, which got no copy and is not scored yet. A second such group pauses the
    /// drill, and neither counts.
    silent: Option<Text<MAX_GROUP>>,
    mania: Mania,
    /// MANIA: how the last round went, while `RoundOver` shows it.
    result: Option<mania::Result>,
}

impl Receive {
    pub(super) fn badge(&self) -> &'static str {
        match self.state {
            State::Playing => "LISTEN",
            State::Waiting => "YOUR TURN",
            State::Copying => "KEYING",
            State::Between | State::NoPack => "",
            State::Hint if self.drill.kind == Kind::Mania => "WRONG",
            State::Hint => "HINT",
            State::Paused => "PAUSED",
            State::RoundOver => "DONE",
        }
    }

    pub(super) fn new(kind: Kind, level: u8, seed: u32) -> Receive {
        Receive {
            drill: Drill::new(kind, level, seed),
            group: Text::new(),
            word: Text::new(),
            copy: Text::new(),
            state: State::Between,
            deadline: None,
            rows: [None; ROWS],
            session_right: 0,
            session_total: 0,
            silent: None,
            mania: Mania::new(mania::FIRST_LEVEL, seed.rotate_left(16)),
            result: None,
        }
    }

    /// Groups are playing or waiting on you, not paused.
    pub(super) fn running(&self) -> bool {
        !matches!(self.state, State::Paused | State::NoPack | State::RoundOver)
    }

    pub(super) fn kind(&self) -> Kind {
        self.drill.kind
    }

    pub(super) fn pause(&mut self) {
        if self.state != State::NoPack {
            self.state = State::Paused;
        }
        self.deadline = None;
        self.copy.clear();
        self.silent = None;
    }
}

impl Ui {
    pub(super) fn draw_receive(&mut self, lcd: &mut Lcd, s: &Settings) {
        let Some(Session::Receive(rx)) = &self.session else { return };
        let t = &self.theme;
        self.header(lcd, s, rx.drill.kind.name(), &[]);
        self.draw_receive_keys(lcd);
        rect(lcd, 0, RULE_Y, MAIN, 1, t.rule());
        self.draw_receive_info(lcd);
        self.draw_receive_rows(lcd);
        self.draw_receive_now(lcd);
    }

    fn draw_receive_keys(&self, lcd: &mut Lcd) {
        let Some(Session::Receive(rx)) = &self.session else { return };
        let chars = rx.drill.kind == Kind::Chars;
        if rx.drill.kind == Kind::Mania {
            let c = rx.mania.choices();
            let keys = match rx.state {
                State::Playing | State::Waiting | State::Copying => {
                    [choice_label(&c[0]), choice_label(&c[1]), choice_label(&c[2]), choice_label(&c[3])]
                }
                State::RoundOver | State::Paused => ["SET", "", "GO", "BACK"],
                _ => ["SET", "", "STOP", "BACK"],
            };
            return self.softkeys(lcd, keys);
        }
        let (again, pause) = match rx.state {
            State::NoPack => ("", ""),
            State::Paused => ("", "GO"),
            _ if chars => ("HINT", "STOP"),
            _ => ("AGN", "STOP"),
        };
        self.softkeys(lcd, ["SET", again, pause, "BACK"]);
    }

    /// Koch level and progress, and the character set colored by your accuracy.
    pub(super) fn draw_receive_info(&self, lcd: &mut Lcd) {
        let Some(Session::Receive(rx)) = &self.session else { return };
        let t = &self.theme;
        rect(lcd, 0, INFO_Y, MAIN, RULE_Y - INFO_Y, t.bg);
        let mut info = Text::<40>::new();
        let mut info_x = 2;
        let drill = &rx.drill;
        let set = match drill.kind {
            Kind::Koch => {
                // The level is a dial target, inverted while focused.
                let level = Text::<10>::format(format_args!("LEVEL {}", drill.level));
                let (fg, bg) = if self.focused() == Some(Target::Level) { (ink(t.accent), t.accent) } else { (t.muted, t.bg) };
                tiny(lcd, info_x, INFO_Y, level.as_str(), fg, bg);
                info_x += level.len() as i32 * TINY_W;
                let (right, total) = drill.window();
                let percent = if total > 0 { right as u32 * 100 / total as u32 } else { 0 };
                let _ = write!(info, "  {total}/50 CHARS  {percent}%  NEED 90%");
                &KOCH[..drill.level as usize]
            }
            Kind::Chars => {
                let _ = write!(info, "SET {}  F2 HINTS  ? REPEATS", drill.char_set_name());
                drill.char_set()
            }
            Kind::Mania => {
                let m = &rx.mania;
                match (rx.state, rx.result) {
                    (State::RoundOver, Some(r)) => {
                        let _ = write!(info, "ROUND {}/{}  ", r.right, ROUND);
                        let _ = match r.stars {
                            0 => write!(info, "NEED 16 TO PASS"),
                            stars => {
                                (0..stars).for_each(|_| _ = info.push(b'*'));
                                if r.unlocked { write!(info, "  LEVEL {} OPEN", m.level()) } else { write!(info, "  PASSED") }
                            }
                        };
                    }
                    _ => {
                        let level = Text::<10>::format(format_args!("LEVEL {}", m.level()));
                        let (fg, bg) = if self.focused() == Some(Target::Level) { (ink(t.accent), t.accent) } else { (t.muted, t.bg) };
                        tiny(lcd, info_x, INFO_Y, level.as_str(), fg, bg);
                        info_x += level.len() as i32 * TINY_W;
                        let (asked, right) = m.progress();
                        let _ = write!(info, "  {asked}/{ROUND}  {right} RIGHT  NEW ");
                        m.new_chars().iter().for_each(|c| _ = info.write_str(morse::text(c)));
                    }
                }
                m.set()
            }
            Kind::Ninja | Kind::Head if rx.state == State::NoPack => {
                let _ = write!(info, "CONTENT PACK MISSING: INSTALL IT");
                &KOCH[..]
            }
            Kind::Ninja => {
                let _ = write!(info, "NINJA DRILL  LISTEN, OR KEY IT BACK");
                &KOCH[..]
            }
            Kind::Head => {
                let _ = write!(info, "RUNG {} OF {}  ", drill.rung(), RUNGS.len());
                let _ = match drill.item() {
                    Item::Words { count, min, max } => {
                        let words = if count == 1 { "WORD" } else { "WORDS" };
                        let _ = write!(info, "{count} {words} OF {min}");
                        if max > min { write!(info, " TO {max} CHARS") } else { write!(info, " CHARS") }
                    }
                    Item::Sentence => write!(info, "A QSO LINE"),
                };
                &KOCH[..]
            }
            _ => {
                let _ = write!(info, "{} DRILL  KEY BACK WHAT YOU HEAR", drill.kind.name());
                &KOCH[..]
            }
        };
        tiny(lcd, info_x, INFO_Y, info.as_str(), t.muted, t.bg);
        let mut cell = 0;
        for (i, &c) in set.iter().enumerate() {
            let cells = char_cells(c);
            if cell % SET_PER_ROW + cells > SET_PER_ROW {
                cell += SET_PER_ROW - cell % SET_PER_ROW;
            }
            let x = 2 + (cell % SET_PER_ROW) as i32 * TINY_W;
            let y = SET_Y + (cell / SET_PER_ROW) as i32 * SET_STEP;
            cell += cells;
            // Weak characters show inverted, so they stand out without color.
            let (fg, bg) = match self.letters.mastery(pocketcw_logic::letters::index(c).unwrap_or(0)) {
                None => (t.muted, t.bg),
                Some(Mastery::Known) => (GOOD, t.bg),
                Some(Mastery::Unsure) => (WARN, t.bg),
                Some(Mastery::Weak) => (ink(BAD), BAD),
            };
            let w = draw_char(lcd, &TINY, x, y, c, fg, bg);
            let new = match drill.kind {
                Kind::Koch => i + 1 == set.len(),
                Kind::Mania => i + rx.mania.new_chars().len() >= set.len(),
                _ => false,
            };
            if new {
                rect(lcd, x, y + 11, w - 1, 1, t.accent);
            }
        }
    }

    fn draw_receive_rows(&self, lcd: &mut Lcd) {
        let Some(Session::Receive(rx)) = &self.session else { return };
        let t = &self.theme;
        if rx.drill.kind == Kind::Head {
            rect(lcd, 0, ROW_Y, MAIN, ROWS as i32 * ROW_STEP, t.bg);
            let skip = rx.rows.iter().flatten().count().saturating_sub(HEAD_ITEMS);
            for (k, row) in rx.rows.iter().flatten().skip(skip).enumerate() {
                let y = ROW_Y + 2 * k as i32 * ROW_STEP;
                let sent = row.sent.as_str().bytes();
                draw_sent(lcd, t, y, sent.clone());
                draw_marks(lcd, t, y + ROW_STEP, 4, sent.filter(|&c| c != b' '), &row.marks, row.right);
            }
            return;
        }
        for (k, row) in rx.rows.iter().enumerate() {
            let y = ROW_Y + k as i32 * ROW_STEP;
            rect(lcd, 0, y, MAIN, ROW_STEP, t.bg);
            let Some(row) = row else { continue };
            if row.mania {
                draw_mania_row(lcd, t, y, row);
            } else if row.hinted {
                draw_hinted(lcd, t, y, row.sent.as_str().bytes());
            } else if row.scored {
                draw_marked(lcd, t, y, COPY_X, row.sent.as_str().bytes(), &row.marks, row.right);
            } else {
                draw_sent(lcd, t, y, row.sent.as_str().bytes());
            }
        }
    }

    /// Your copy so far, what the drill waits for, and the session score.
    fn draw_receive_now(&mut self, lcd: &mut Lcd) {
        let Some(Session::Receive(rx)) = &self.session else { return };
        let t = &self.theme;
        let copy = rx.copy.as_str();
        let shown = &copy[copy.len().saturating_sub(INPUT_COLS - 1)..];
        text_cols(lcd, 0, INPUT_Y, shown, INPUT_COLS, t.fg, t.bg);
        if rx.state == State::Waiting || rx.state == State::Copying {
            rect(lcd, shown.len() as i32 * CHAR_W + 1, INPUT_Y + 2, CHAR_W - 2, 13, t.accent);
        }
        let hint = match rx.drill.kind {
            Kind::Ninja => "LISTEN, OR KEY IT BACK  ? REPEATS",
            Kind::Head => "KEY IT BACK AFTER IT ENDS  ? REPEATS",
            Kind::Chars => "KEY IT BACK  F2 FOR A HINT",
            Kind::Mania => "PICK F1-F4, OR KEY IT  ? REPEATS",
            _ => "KEY BACK WHAT YOU HEARD  ? REPEATS",
        };
        let foot = match rx.state {
            State::Paused => FootLine::format(format_args!("PAUSED: TOUCH THE KEY OR PRESS GO")),
            State::RoundOver => FootLine::format(format_args!("GO: NEXT ROUND  DIAL: PICK A LEVEL")),
            State::Hint => {
                let c = rx.group.as_str().as_bytes().first().copied().unwrap_or(b' ');
                let what = if rx.drill.kind == Kind::Mania { "IT WAS" } else { "HINT:" };
                FootLine::format(format_args!("{what} {}  {}", morse::text(&c), morse::code(c).unwrap_or("")))
            }
            State::Waiting => FootLine::format(format_args!("{hint}")),
            _ => session_line(rx.session_right, rx.session_total, hint),
        };
        self.set_footer(lcd, foot.as_str());
    }

    /// Starts the next group, or the same one again with `again`.
    fn play_group(&mut self, lcd: &mut Lcd, s: &Settings, again: bool) {
        let seed = self.next_seed();
        let Some(Session::Receive(rx)) = &mut self.session else { return };
        if !again {
            rx.drill.set_chars(s.chars_set);
            let pack = self.pack.as_ref().ok();
            let group = match (rx.drill.kind, rx.drill.item()) {
                (Kind::Ninja, _) => pack.and_then(|pack| ninja_word(&mut rx.drill, pack, lcd, s.ninja_words)).map(|(word, group)| {
                    rx.word = word;
                    group
                }),
                (Kind::Head, Item::Words { count, min, max }) => {
                    pack.and_then(|pack| head_words(&mut rx.drill, pack, lcd, count, min as usize..=max as usize))
                }
                (Kind::Mania, _) => {
                    let mut group = Text::new();
                    let _ = group.push(rx.mania.next(&self.letters));
                    Some(group)
                }
                _ => Some(rx.drill.next(&self.letters)),
            };
            let Some(group) = group else {
                rx.state = State::NoPack;
                rx.deadline = None;
                self.draw_receive_keys(lcd);
                self.draw_receive_info(lcd);
                self.draw_receive_now(lcd);
                return;
            };
            rx.group = group;
        }
        rx.copy.clear();
        rx.state = State::Playing;
        rx.deadline = None;
        let (wpm, effective) = s.receive_speed(s.wpm);
        let text = Text::<MAX_TEXT>::format(format_args!("{}", rx.group.as_str()));
        let keying = Keying { wpm, effective, fist: 0 };
        let mania = rx.drill.kind == Kind::Mania;
        self.play = Some(Play { text, keying, seed, delay_ms: GROUP_DELAY_MS, pitch_hz: s.tone_hz, transmit: false });
        self.draw_receive_now(lcd);
        if mania {
            self.draw_receive_keys(lcd);
            self.draw_receive_info(lcd);
        }
    }

    pub(super) fn open_receive(&mut self, lcd: &mut Lcd, s: &Settings, kind: Kind) {
        let seed = self.next_seed();
        let mut rx = Receive::new(kind, self.letters.level, seed);
        if self.koch_window.0 == self.letters.level {
            rx.drill.resume(self.koch_window.1);
        }
        rx.drill.set_rung(s.head_rung);
        rx.mania.set_level(s.mania_level);
        self.session = Some(Session::Receive(rx));
        self.open(lcd, s, Screen::Receive);
        self.play_group(lcd, s, false);
    }

    /// Back from settings: the next group plays at the new speed.
    pub(super) fn receive_resume(&mut self, lcd: &mut Lcd, s: &Settings) {
        self.open(lcd, s, Screen::Receive);
        self.play_group(lcd, s, false);
        self.draw_receive_keys(lcd);
    }

    pub(super) fn receive_played(&mut self, lcd: &mut Lcd, now_ms: u32) {
        let Some(Session::Receive(rx)) = &mut self.session else { return };
        if rx.state == State::Hint {
            rx.state = State::Between;
            rx.deadline = Some(now_ms.wrapping_add(HINT_HOLD_MS));
            return;
        }
        if rx.state == State::Playing {
            rx.state = State::Waiting;
            let wait = if rx.drill.kind == Kind::Ninja { THINK_MS } else { START_MS };
            // MANIA waits for your answer as long as you like.
            rx.deadline = (rx.drill.kind != Kind::Mania).then(|| now_ms.wrapping_add(wait));
            self.draw_receive_now(lcd);
        }
    }

    pub(super) fn receive_key(&mut self, lcd: &mut Lcd, down: bool, at_ms: u32, dit_ms: u16) {
        let Some(Session::Receive(rx)) = &mut self.session else { return };
        if matches!(rx.state, State::Between | State::Hint | State::Paused | State::NoPack | State::RoundOver) {
            return;
        }
        let was = rx.state;
        if down {
            if rx.state == State::Waiting {
                rx.state = State::Copying;
            }
            if rx.state == State::Copying {
                rx.deadline = None;
            }
        } else if rx.state == State::Copying {
            rx.deadline = Some(copy_end(at_ms, dit_ms));
        }
        if rx.state != was {
            self.draw_receive_now(lcd);
        }
    }

    pub(super) fn receive_decoded(&mut self, lcd: &mut Lcd, s: &mut Settings, decoded: Decoded) {
        let Some(Session::Receive(rx)) = &mut self.session else { return };
        // The character that ends a pause starts the next group, and is not copy.
        if matches!(rx.state, State::Paused | State::RoundOver) {
            if decoded != Decoded::Space {
                self.play_group(lcd, s, false);
                self.draw(lcd, s);
            }
            return;
        }
        // MANIA: a keyed character is your answer, and `?` plays it again.
        if rx.drill.kind == Kind::Mania {
            if let Decoded::Char(c) = decoded
                && matches!(rx.state, State::Playing | State::Waiting | State::Copying)
            {
                if c == b'?' {
                    self.stop_players();
                    self.play_group(lcd, s, true);
                } else {
                    self.mania_answer(lcd, s, c);
                }
            }
            return;
        }
        // Head copy holds the item in your head until it ends.
        let head_playing = rx.drill.kind == Kind::Head && rx.state == State::Playing;
        if !matches!(rx.state, State::Copying | State::Waiting | State::Playing) || head_playing {
            return;
        }
        if decoded.write_to(&mut rx.copy) {
            self.draw_receive_now(lcd);
        }
    }

    /// The copy ended or timed out: score it. Or the pause between groups ended: play the next.
    pub(super) fn receive_timeout(&mut self, lcd: &mut Lcd, s: &mut Settings, now_ms: u32) {
        let Some(Session::Receive(rx)) = &mut self.session else { return };
        rx.deadline = None;
        if rx.state == State::Between {
            if rx.drill.kind == Kind::Mania {
                return self.mania_next(lcd, s);
            }
            return self.play_group(lcd, s, false);
        }
        if !matches!(rx.state, State::Waiting | State::Copying) {
            return;
        }
        if rx.copy.as_str().trim() == "?" {
            return self.play_group(lcd, s, true);
        }
        let ninja = rx.drill.kind == Kind::Ninja;
        if ninja {
            self.say = Some(rx.word);
        }
        if ninja && rx.state == State::Waiting && rx.copy.as_str().trim().is_empty() {
            push_scrolling(&mut rx.rows, Row { sent: rx.group, marks: [Mark::Missed; MAX_GROUP], right: 0, scored: false, hinted: false, mania: false });
            rx.copy.clear();
            rx.state = State::Between;
            rx.deadline = Some(now_ms.wrapping_add(NEXT_GROUP_MS));
            self.draw_receive_rows(lcd);
            self.draw_receive_now(lcd);
            return;
        }
        if rx.state == State::Waiting && rx.copy.as_str().trim().is_empty() {
            push_scrolling(&mut rx.rows, Row { sent: rx.group, marks: [Mark::Missed; MAX_GROUP], right: 0, scored: false, hinted: false, mania: false });
            if rx.silent.is_some() {
                rx.pause();
                self.draw(lcd, s);
                self.status(lcd, "NO COPY FOR 2 GROUPS: PAUSED");
                return;
            }
            rx.silent = Some(rx.group);
            rx.state = State::Between;
            rx.deadline = Some(now_ms.wrapping_add(NEXT_GROUP_MS));
            self.draw_receive_rows(lcd);
            self.draw_receive_now(lcd);
            return;
        }
        // A lone silent group counts as missed once you copy the next one. Its row is the last.
        let missed = rx.silent.take().map(|sent| rx.drill.score(sent.as_str(), "", &mut self.letters));
        if let Some(missed) = missed
            && let Some(row) = rx.rows.iter_mut().flatten().last()
        {
            *row = Row { marks: missed.marks, right: missed.right, scored: true, ..*row };
        }
        let score = rx.drill.score(rx.group.as_str(), rx.copy.as_str(), &mut self.letters);
        self.letters.level = rx.drill.level;
        self.koch_window = (rx.drill.level, rx.drill.window());
        s.head_rung = rx.drill.rung();
        for scored in missed.iter().chain([&score]) {
            self.history.copied(scored.right, scored.len as u8);
            rx.session_right += scored.right as u16;
            rx.session_total += scored.len as u16;
        }
        let level_up = score.level_up || missed.is_some_and(|missed| missed.level_up);
        push_scrolling(&mut rx.rows, Row { sent: rx.group, marks: score.marks, right: score.right, scored: true, hinted: false, mania: false });
        rx.copy.clear();
        rx.state = State::Between;
        rx.deadline = Some(now_ms.wrapping_add(NEXT_GROUP_MS));
        self.draw_receive_rows(lcd);
        self.draw_receive_now(lcd);
        if level_up {
            self.draw_receive_info(lcd);
            let mut line = Text::<COLS>::new();
            if let Some(Session::Receive(rx)) = &self.session {
                let _ = write!(line, "LEVEL UP: ADDED {}", morse::text(&KOCH[rx.drill.level as usize - 1]));
            }
            self.status(lcd, line.as_str());
        } else if let Some(Session::Receive(rx)) = &self.session
            && matches!(rx.drill.kind, Kind::Koch | Kind::Head)
        {
            self.draw_receive_info(lcd);
        }
    }

    /// The voice started saying the NINJA word at `now_ms` and lasts `ms`: the next word waits
    /// until it ends. With `ms` 0 nothing was said, and the next word keeps its deadline.
    pub(super) fn receive_said(&mut self, ms: u32, now_ms: u32) {
        let Some(Session::Receive(rx)) = &mut self.session else { return };
        if rx.state == State::Between && ms > 0 {
            rx.deadline = Some(now_ms.wrapping_add(ms + NEXT_GROUP_MS));
        }
    }

    /// CHARS: shows the character and its code, plays it again, and scores it as missed. A silent
    /// group before it counts as missed too, as it does when you copy.
    fn receive_hint(&mut self, lcd: &mut Lcd, s: &Settings) {
        self.stop_players();
        let seed = self.next_seed();
        let Some(Session::Receive(rx)) = &mut self.session else { return };
        let missed = rx.silent.take().map(|sent| rx.drill.score(sent.as_str(), "", &mut self.letters));
        if let Some(missed) = missed
            && let Some(row) = rx.rows.iter_mut().flatten().last()
        {
            *row = Row { marks: missed.marks, right: missed.right, scored: true, ..*row };
        }
        let score = rx.drill.score(rx.group.as_str(), "", &mut self.letters);
        for scored in missed.iter().chain([&score]) {
            self.history.copied(scored.right, scored.len as u8);
            rx.session_right += scored.right as u16;
            rx.session_total += scored.len as u16;
        }
        let sent_char = rx.group.as_str().as_bytes().first().copied().unwrap_or(b' ');
        push_scrolling(&mut rx.rows, Row { sent: rx.group, marks: score.marks, right: 0, scored: true, hinted: true, mania: false });
        rx.copy.clear();
        rx.state = State::Hint;
        rx.deadline = None;
        let (wpm, effective) = s.receive_speed(s.wpm);
        let text = Text::<MAX_TEXT>::format(format_args!("{}", rx.group.as_str()));
        let keying = Keying { wpm, effective, fist: 0 };
        self.play = Some(Play { text, keying, seed, delay_ms: GROUP_DELAY_MS, pitch_hz: s.tone_hz, transmit: false });
        self.draw_receive_keys(lcd);
        self.draw_receive_info(lcd);
        self.draw_receive_rows(lcd);
        self.draw_receive_now(lcd);
        // In the bright status color too, so it stands out and stays a while.
        let c = sent_char;
        self.status(lcd, FootLine::format(format_args!("HINT: {}  {}", morse::text(&c), morse::code(c).unwrap_or(""))).as_str());
    }

    /// MANIA: your answer `c`, picked or keyed. Right goes on to the next character; wrong shows
    /// and plays what it was.
    fn mania_answer(&mut self, lcd: &mut Lcd, s: &mut Settings, c: u8) {
        self.stop_players();
        let seed = self.next_seed();
        let Some(Session::Receive(rx)) = &mut self.session else { return };
        let right = rx.mania.answered(c, &mut self.letters);
        self.history.copied(right as u8, 1);
        rx.session_right += right as u16;
        rx.session_total += 1;
        let mut marks = [Mark::Missed; MAX_GROUP];
        marks[0] = if right { Mark::Right } else { Mark::Wrong(c) };
        push_scrolling(&mut rx.rows, Row { sent: rx.group, marks, right: right as u8, scored: true, hinted: false, mania: true });
        rx.copy.clear();
        rx.deadline = None;
        if right {
            self.draw_receive_rows(lcd);
            return self.mania_next(lcd, s);
        }
        rx.state = State::Hint;
        let (wpm, effective) = s.receive_speed(s.wpm);
        let text = Text::<MAX_TEXT>::format(format_args!("{}", rx.group.as_str()));
        let keying = Keying { wpm, effective, fist: 0 };
        self.play = Some(Play { text, keying, seed, delay_ms: GROUP_DELAY_MS, pitch_hz: s.tone_hz, transmit: false });
        self.draw_receive_keys(lcd);
        self.draw_receive_info(lcd);
        self.draw_receive_rows(lcd);
        self.draw_receive_now(lcd);
    }

    /// MANIA: the next character, or the round's result once it has `ROUND` answers.
    fn mania_next(&mut self, lcd: &mut Lcd, s: &mut Settings) {
        let Some(Session::Receive(rx)) = &mut self.session else { return };
        if !rx.mania.round_over() {
            return self.play_group(lcd, s, false);
        }
        let result = rx.mania.finish_round();
        s.mania_level = rx.mania.level();
        rx.result = Some(result);
        rx.state = State::RoundOver;
        rx.deadline = None;
        let line = match (result.stars, result.unlocked) {
            (0, _) => FootLine::format(format_args!("{}/{} RIGHT: 16 PASSES, TRY AGAIN", result.right, ROUND)),
            (_, true) => FootLine::format(format_args!("PASSED! LEVEL {} ADDS {}", rx.mania.level(), NewChars(rx.mania.new_chars()))),
            (_, false) => FootLine::format(format_args!("PASSED THE LAST LEVEL: {}/{}", result.right, ROUND)),
        };
        self.draw(lcd, s);
        self.status(lcd, line.as_str());
    }

    /// The dial stepped the Koch level; the caller redraws it.
    pub(super) fn step_level(&mut self, s: &mut Settings, delta: i8) {
        let Some(Session::Receive(rx)) = &mut self.session else { return };
        if rx.drill.kind == Kind::Mania {
            rx.mania.set_level((rx.mania.level() as i32 + delta as i32).clamp(0, u8::MAX as i32) as u8);
            s.mania_level = rx.mania.level();
            rx.result = None;
            return;
        }
        rx.drill.set_level((rx.drill.level as i32 + delta as i32).clamp(0, u8::MAX as i32) as u8);
        self.letters.level = rx.drill.level;
        self.koch_window = (rx.drill.level, rx.drill.window());
    }

    pub(super) fn receive_act(&mut self, lcd: &mut Lcd, s: &mut Settings, action: Action) {
        let Some(Session::Receive(rx)) = &mut self.session else { return };
        let mania = rx.drill.kind == Kind::Mania;
        match action {
            Action::Soft(k) if mania && matches!(rx.state, State::Playing | State::Waiting | State::Copying) => {
                let c = rx.mania.choices()[k as usize % CHOICES];
                if c != 0 {
                    self.mania_answer(lcd, s, c);
                }
            }
            Action::Soft(2) if mania && rx.state == State::RoundOver => {
                rx.result = None;
                self.play_group(lcd, s, false);
                self.draw(lcd, s);
            }
            Action::Soft(0) if rx.drill.kind == Kind::Ninja => self.open_settings_at(lcd, s, Section::Drills),
            Action::Soft(0) => self.open_settings_at(lcd, s, Section::Receive),
            Action::Soft(1) if rx.drill.kind == Kind::Chars => {
                if matches!(rx.state, State::Playing | State::Waiting | State::Copying) {
                    self.receive_hint(lcd, s);
                }
            }
            Action::Soft(1) if matches!(rx.state, State::Playing | State::Waiting | State::Copying) => {
                self.stop_players();
                self.play_group(lcd, s, true);
            }
            Action::Soft(2) if rx.state == State::NoPack => {}
            Action::Soft(2) => {
                if rx.state == State::Paused {
                    self.play_group(lcd, s, false);
                } else {
                    rx.pause();
                    self.stop_players();
                }
                self.draw(lcd, s);
            }
            Action::Soft(3) => self.back(lcd, s),
            _ => {}
        }
    }
}

/// A NINJA word among the first `limit`: one the voice can say that fits left of `COPY_X`, as the pack
/// spells it for the voice and as the group to send.
fn ninja_word(drill: &mut Drill, pack: &Pack, lcd: &mut Lcd, limit: u16) -> Option<(Text<WORD_TEXT>, Text<MAX_GROUP>)> {
    let cells = |group: &Text<MAX_GROUP>| group.as_str().bytes().map(char_cells).sum::<usize>();
    pack_word(drill, pack, lcd, limit, |group, voiced| voiced && cells(group) <= SENT_CELLS)
}

/// `count` pack words of `len` characters each, between spaces.
fn head_words(drill: &mut Drill, pack: &Pack, lcd: &mut Lcd, count: u8, len: RangeInclusive<usize>) -> Option<Text<MAX_GROUP>> {
    let mut group = Text::new();
    for i in 0..count {
        let (_, word) = pack_word(drill, pack, lcd, u16::MAX, |word, _| len.contains(&word.len()))?;
        if i > 0 {
            group.push(b' ');
        }
        word.as_str().bytes().for_each(|c| _ = group.push(c));
    }
    Some(group)
}

/// A pack word among the first `limit` that `fits` takes, given the word as a group and whether
/// the voice can say it. Returns it as the pack spells it and as a group. Random picks come first, then
/// a scan on from the last pick, so a rare length still ends. None only when no word fits.
fn pack_word(
    drill: &mut Drill,
    pack: &Pack,
    lcd: &mut Lcd,
    limit: u16,
    fits: impl Fn(&Text<MAX_GROUP>, bool) -> bool,
) -> Option<(Text<WORD_TEXT>, Text<MAX_GROUP>)> {
    let (first, count) = pack.words();
    let span = count.min(limit);
    let mut try_word = |index: u16| {
        let word = pack.word(lcd, index)?;
        let mut group = Text::new();
        let whole = morse::chars(word.text()).all(|c| group.push(c));
        let text = Text::format(format_args!("{}", core::str::from_utf8(word.text()).ok()?));
        (whole && fits(&group, word.clip.is_some() || pack.voiced)).then_some((text, group))
    };
    let mut last = first;
    for _ in 0..PICK_TRIES {
        last = drill.pick_word(first, count, limit);
        if let Some(found) = try_word(last) {
            return Some(found);
        }
    }
    (1..span).find_map(|k| try_word(first + (last - first + k) % span))
}

/// One scored row at `y`: what was sent, then your copy marked against it from `copy_x`.
pub(super) fn draw_marked(lcd: &mut Lcd, t: &Theme, y: i32, copy_x: i32, sent: impl Iterator<Item = u8> + Clone, marks: &[Mark], right: u8) {
    draw_sent(lcd, t, y, sent.clone());
    draw_marks(lcd, t, y, copy_x, sent, marks, right);
}

/// A CHARS row you asked a hint for: the character, then from `COPY_X` the character and its code
/// in the warning color, and a score of none right.
fn draw_hinted(lcd: &mut Lcd, t: &Theme, y: i32, sent: impl Iterator<Item = u8> + Clone) {
    draw_sent(lcd, t, y, sent.clone());
    let mut x = COPY_X;
    for c in sent.clone() {
        x += draw_char(lcd, &SMALL, x, y, c, ink(WARN), WARN);
        x += CHAR_W;
        for d in morse::code(c).unwrap_or("").bytes() {
            x += draw_char(lcd, &SMALL, x, y, d, WARN, t.bg);
        }
    }
    let score = Text::<12>::format(format_args!("HINT 0/{}", sent.count()));
    tiny(lcd, MAIN - 6 - score.len() as i32 * TINY_W, y + 3, score.as_str(), t.muted, t.bg);
}

/// A MANIA row: what was sent, your pick marked against it, then the code of what was sent, in
/// the warning color when you picked wrong.
fn draw_mania_row(lcd: &mut Lcd, t: &Theme, y: i32, row: &Row) {
    draw_marked(lcd, t, y, COPY_X, row.sent.as_str().bytes(), &row.marks, row.right);
    let sent = row.sent.as_str().as_bytes().first().copied().unwrap_or(b' ');
    let color = if row.right > 0 { t.muted } else { WARN };
    let mut x = COPY_X + 3 * CHAR_W;
    for d in morse::code(sent).unwrap_or("").bytes() {
        x += draw_char(lcd, &SMALL, x, y, d, color, t.bg);
    }
}

/// A MANIA choice as its soft key shows it; 0 is an unused key.
fn choice_label(c: &u8) -> &str {
    if *c == 0 { "" } else { morse::text(c) }
}

/// Characters written as text, prosigns as `<AR>`.
struct NewChars(&'static [u8]);

impl core::fmt::Display for NewChars {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        self.0.iter().enumerate().try_for_each(|(i, c)| if i > 0 { write!(f, " {}", morse::text(c)) } else { f.write_str(morse::text(c)) })
    }
}

fn draw_sent(lcd: &mut Lcd, t: &Theme, y: i32, sent: impl Iterator<Item = u8>) {
    let mut x = 4;
    for c in sent {
        x += draw_char(lcd, &SMALL, x, y, c, t.muted, t.bg);
    }
}

/// Your copy marked against `sent` from `x`, and the score at the right end. Errors show
/// inverted, so they stand out without color.
fn draw_marks(lcd: &mut Lcd, t: &Theme, y: i32, mut x: i32, sent: impl Iterator<Item = u8> + Clone, marks: &[Mark], right: u8) {
    let len = sent.clone().count();
    for (mark, sent_char) in marks.iter().zip(sent) {
        let (c, fg, bg) = match *mark {
            Mark::Right => (sent_char, GOOD, t.bg),
            Mark::Wrong(c) => (c, ink(BAD), BAD),
            Mark::Missed => (b'_', ink(BAD), BAD),
        };
        x += draw_char(lcd, &SMALL, x, y, c, fg, bg);
    }
    let score = Text::<6>::format(format_args!("{right}/{len}"));
    tiny(lcd, MAIN - 6 - score.len() as i32 * TINY_W, y + 3, score.as_str(), t.muted, t.bg);
}

/// The footer: the session's percent right, or `hint` before anything is scored.
pub(super) fn session_line(right: u16, total: u16, hint: &str) -> FootLine {
    if total > 0 {
        FootLine::format(format_args!("SESSION {}%  {} CHARS", right as u32 * 100 / total as u32, total))
    } else {
        FootLine::format(format_args!("{hint}"))
    }
}
