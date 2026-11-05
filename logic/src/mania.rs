//! MANIA: a listening game in the spirit of Morse Mania. The device plays one character and you
//! pick it from up to four choices, or key it. Characters come in mirror pairs, such as A `.-`
//! and N `-.`, a level at a time, and a round of `ROUND` answers with enough right unlocks the
//! next level.

use crate::letters::{self, Letters};
use crate::morse;
use crate::rng::Rng;

/// Every character in the order MANIA teaches them.
const ORDER: &[u8] = b"ETIMANSOUGRKWDHBJFQLYPXVCZ1627384950";
/// Where each level's characters end in `ORDER`: level 1 is E and T, level 2 adds I and M.
const LEVEL_END: [u8; 20] = [2, 4, 6, 8, 10, 12, 14, 15, 17, 19, 21, 23, 24, 25, 26, 28, 30, 32, 34, 36];
const _: () = assert!(LEVEL_END[LEVEL_END.len() - 1] as usize == ORDER.len());
pub const FIRST_LEVEL: u8 = 1;
pub const LEVELS: u8 = LEVEL_END.len() as u8;
/// Answers in a round.
pub const ROUND: u8 = 20;
/// Right answers in a round for one, two and three stars. One star passes.
const STARS: [u8; 3] = [16, 18, 20];
pub const CHOICES: usize = 4;
/// How much more often a level's new characters come up than the others.
const NEW_WEIGHT: u32 = 3;

/// How a round went.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Result {
    pub right: u8,
    /// 0 to 3; one or more passes.
    pub stars: u8,
    /// This round unlocked the next level.
    pub unlocked: bool,
}

pub struct Mania {
    level: u8,
    rng: Rng,
    answer: u8,
    choices: [u8; CHOICES],
    asked: u8,
    right: u8,
}

impl Mania {
    pub fn new(level: u8, seed: u32) -> Mania {
        Mania { level: level.clamp(FIRST_LEVEL, LEVELS), rng: Rng::new(seed), answer: 0, choices: [0; CHOICES], asked: 0, right: 0 }
    }

    pub fn level(&self) -> u8 {
        self.level
    }

    /// Moves to `level` and starts a new round there.
    pub fn set_level(&mut self, level: u8) {
        self.level = level.clamp(FIRST_LEVEL, LEVELS);
        self.start_round();
    }

    pub fn start_round(&mut self) {
        self.asked = 0;
        self.right = 0;
    }

    /// Every character up to this level.
    pub fn set(&self) -> &'static [u8] {
        &ORDER[..LEVEL_END[self.level as usize - 1] as usize]
    }

    /// The characters this level adds.
    pub fn new_chars(&self) -> &'static [u8] {
        let start = if self.level > FIRST_LEVEL { LEVEL_END[self.level as usize - 2] as usize } else { 0 };
        &self.set()[start..]
    }

    /// Answers so far this round, and how many were right.
    pub fn progress(&self) -> (u8, u8) {
        (self.asked, self.right)
    }

    /// The character playing now.
    pub fn answer(&self) -> u8 {
        self.answer
    }

    /// The choices for the character playing now, in key order; 0 marks an unused key.
    pub fn choices(&self) -> [u8; CHOICES] {
        self.choices
    }

    /// Picks the next character, weighted toward weak ones and this level's new ones, and not
    /// the last one again. Lays out its choices: the answer, its mirror when the level has it,
    /// and others from the level, shuffled.
    pub fn next(&mut self, letters: &Letters) -> u8 {
        let set = self.set();
        let new_from = set.len() - self.new_chars().len();
        let last = self.answer;
        let weight = |i: usize| {
            if set[i] == last && set.len() > 1 {
                return 0;
            }
            let base = letters::index(set[i]).map_or(2, |k| letters.weight(k));
            base * if i >= new_from { NEW_WEIGHT } else { 1 }
        };
        let total: u32 = (0..set.len()).map(weight).sum();
        let mut r = self.rng.below(total);
        let mut answer = set[set.len() - 1];
        for (i, &c) in set.iter().enumerate() {
            if r < weight(i) {
                answer = c;
                break;
            }
            r -= weight(i);
        }
        self.answer = answer;
        self.lay_out_choices(set);
        answer
    }

    fn lay_out_choices(&mut self, set: &[u8]) {
        let n = set.len().min(CHOICES);
        let mut choices = [0u8; CHOICES];
        choices[0] = self.answer;
        let mut count = 1;
        if let Some(m) = mirror(self.answer).filter(|m| set.contains(m))
            && count < n
        {
            choices[count] = m;
            count += 1;
        }
        while count < n {
            let c = set[self.rng.below(set.len() as u32) as usize];
            if !choices[..count].contains(&c) {
                choices[count] = c;
                count += 1;
            }
        }
        for i in (1..n).rev() {
            let j = self.rng.below(i as u32 + 1) as usize;
            choices.swap(i, j);
        }
        self.choices = choices;
    }

    /// Your answer `c`: picked or keyed. Records it for the letter stats. True when right.
    pub fn answered(&mut self, c: u8, letters: &mut Letters) -> bool {
        let c = c.to_ascii_uppercase();
        let right = c == self.answer;
        letters.record(self.answer, Some(c).filter(|&c| letters::index(c).is_some()));
        self.asked = self.asked.saturating_add(1);
        self.right += right as u8;
        right
    }

    pub fn round_over(&self) -> bool {
        self.asked >= ROUND
    }

    /// How the round went; a pass moves up a level unless this is the last one.
    pub fn finish_round(&mut self) -> Result {
        let stars = STARS.iter().filter(|&&s| self.right >= s).count() as u8;
        let unlocked = stars > 0 && self.level < LEVELS;
        let result = Result { right: self.right, stars, unlocked };
        if unlocked {
            self.level += 1;
        }
        self.start_round();
        result
    }
}

/// The character with dits and dahs swapped: A `.-` for N `-.`.
pub fn mirror(c: u8) -> Option<u8> {
    let code = morse::code(c)?;
    let mut flipped = [0u8; 8];
    for (out, d) in flipped.iter_mut().zip(code.bytes()) {
        *out = if d == b'.' { b'-' } else { b'.' };
    }
    morse::decode(core::str::from_utf8(&flipped[..code.len().min(8)]).ok()?)
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    #[test]
    fn levels_add_mirror_pairs() {
        let mut m = Mania::new(1, 1);
        assert_eq!(m.set(), b"ET");
        m.set_level(3);
        assert_eq!((m.set(), m.new_chars()), (&b"ETIMAN"[..], &b"AN"[..]));
        m.set_level(LEVELS);
        assert_eq!(m.set().len(), 36);
        assert_eq!(m.new_chars(), b"50");
        for pair in LEVEL_END.windows(2) {
            let new = &ORDER[pair[0] as usize..pair[1] as usize];
            if new.len() == 2 {
                assert_eq!(mirror(new[0]), Some(new[1]), "{}", core::str::from_utf8(new).unwrap());
            }
        }
        m.set_level(0);
        assert_eq!(m.level(), FIRST_LEVEL);
        m.set_level(200);
        assert_eq!(m.level(), LEVELS);
    }

    #[test]
    fn choices_hold_the_answer_its_mirror_and_no_repeats() {
        let letters = Letters::default();
        for level in FIRST_LEVEL..=LEVELS {
            let mut m = Mania::new(level, 7 + level as u32);
            let mut last = 0;
            for _ in 0..300 {
                let answer = m.next(&letters);
                let set = m.set();
                assert!(set.contains(&answer));
                if set.len() > 1 {
                    assert_ne!(answer, last, "the same character twice in a row");
                }
                last = answer;
                let choices = m.choices();
                let used: std::vec::Vec<u8> = choices.iter().copied().filter(|&c| c != 0).collect();
                assert_eq!(used.len(), set.len().min(CHOICES));
                assert!(used.contains(&answer));
                assert!(used.iter().all(|c| set.contains(c)));
                let mut sorted = used.clone();
                sorted.sort();
                sorted.dedup();
                assert_eq!(sorted.len(), used.len(), "no choice twice");
                if let Some(mirror) = mirror(answer).filter(|c| set.contains(c)) {
                    assert!(used.contains(&mirror), "{} without its mirror", answer as char);
                }
            }
        }
    }

    #[test]
    fn the_answer_moves_between_keys() {
        let mut m = Mania::new(5, 3);
        let letters = Letters::default();
        let mut slots = [0; CHOICES];
        for _ in 0..400 {
            let answer = m.next(&letters);
            slots[m.choices().iter().position(|&c| c == answer).unwrap()] += 1;
        }
        assert!(slots.iter().all(|&n| n > 50), "{slots:?}");
    }

    #[test]
    fn a_round_passes_at_sixteen_and_moves_up_a_level() {
        let mut letters = Letters::default();
        let mut m = Mania::new(2, 5);
        for i in 0..ROUND {
            let answer = m.next(&letters);
            let guess = if i < 15 { answer } else { b'X' };
            assert_eq!(m.answered(guess, &mut letters), i < 15);
        }
        assert!(m.round_over());
        assert_eq!(m.finish_round(), Result { right: 15, stars: 0, unlocked: false });
        assert_eq!((m.level(), m.progress()), (2, (0, 0)));
        for i in 0..ROUND {
            let answer = m.next(&letters);
            m.answered(if i < 18 { answer.to_ascii_lowercase() } else { 0 }, &mut letters);
        }
        assert_eq!(m.finish_round(), Result { right: 18, stars: 2, unlocked: true });
        assert_eq!(m.level(), 3);
        m.set_level(LEVELS);
        for _ in 0..ROUND {
            let answer = m.next(&letters);
            m.answered(answer, &mut letters);
        }
        assert_eq!(m.finish_round(), Result { right: 20, stars: 3, unlocked: false });
        assert_eq!(m.level(), LEVELS, "the last level stays");
    }

    #[test]
    fn answers_feed_the_letter_stats() {
        let mut letters = Letters::default();
        let mut m = Mania::new(1, 9);
        let answer = m.next(&letters);
        m.answered(answer, &mut letters);
        let other = if answer == b'E' { b'T' } else { b'E' };
        let answer = m.next(&letters);
        assert_eq!(answer, other, "not the same one twice");
        m.answered(b'?', &mut letters);
        let i = letters::index(answer).unwrap();
        assert_eq!(letters.accuracy(i), Some((0, 1)));
    }
}
