//! DASH on screen: a sky with a sun, mountains that scroll slowly behind, a perspective grid
//! floor that scrolls fast, and the sprites. The screen takes about a microsecond a pixel, and
//! a whole scene is 44,000, so a frame sends only what changed: each floor line where it was and
//! where it is, the pixel or two at the top of each column of mountains when they move, and each
//! sprite where it was and where it is. Sprites stay above the floor, so the floor lines never
//! cover them. A redrawn pixel is the topmost sprite's, or the scenery's.

use embedded_graphics::{
    draw_target::DrawTarget,
    pixelcolor::{Rgb565, RgbColor},
    prelude::{Point, Size},
    primitives::Rectangle,
};
use pocketcw_logic::dash::{
    Dash, HERO_X, Kind, MAX_OBSTACLES, Mode,
    scene::{self, FIRST_LINE_ROW, MAX_SEGMENTS},
};

use super::dash_art::{CRATE, HERO_JUMP, HERO_RUN1, HERO_RUN2, ROCK, TOWER};
use crate::gfx::{self, mix, rect};
use crate::lcd::Lcd;
use crate::theme::{BAD, GOOD, MAIN, Theme};

pub(super) const SCENE_TOP: i32 = 40;
const GROUND_Y: i32 = 160;
const SCENE_BOTTOM: i32 = 202;
const FLOOR_ROWS: usize = (SCENE_BOTTOM - GROUND_Y) as usize;
const SUN_X: i32 = 200;
const SUN_Y: i32 = 112;
const SUN_R: i32 = 32;
const SKY_ROWS: usize = (GROUND_Y - SCENE_TOP) as usize;
const SPRITES: usize = 1 + 2 * MAX_OBSTACLES;
const BUBBLE_W: i32 = 14;
const BUBBLE_H: i32 = 18;
/// Rows of the floor that are whole grid lines, counted down from the horizon; they never move.
const FLOOR_LINES: [i32; 7] = [0, 2, 5, 9, 14, 20, 27];

#[derive(Clone, Copy, PartialEq, Eq)]
struct Rect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl Rect {
    fn union(self, o: Rect) -> Rect {
        let (x, y) = (self.x.min(o.x), self.y.min(o.y));
        Rect { x, y, w: (self.x + self.w).max(o.x + o.w) - x, h: (self.y + self.h).max(o.y + o.h) - y }
    }

    fn overlaps(self, o: Rect) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }

    /// Clipped to the scene.
    fn clip(self) -> Rect {
        let (x0, y0) = (self.x.max(0), self.y.max(SCENE_TOP));
        let (x1, y1) = ((self.x + self.w).min(MAIN), (self.y + self.h).min(SCENE_BOTTOM));
        Rect { x: x0, y: y0, w: (x1 - x0).max(0), h: (y1 - y0).max(0) }
    }
}

/// What a sprite shows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Art {
    /// Run frame 0 or 1, or 2 in the air.
    Hero(u8),
    Thing(Kind),
    /// The character over an obstacle, and how it went: waiting, cleared or crashed.
    Bubble(u8, Fate),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Fate {
    Waiting,
    Cleared,
    Crashed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Sprite {
    r: Rect,
    art: Art,
}

/// The scenery's colors from the theme, and what each sprite slot last drew.
pub(super) struct View {
    sky: [Rgb565; SKY_ROWS],
    sun_top: Rgb565,
    sun_bottom: Rgb565,
    mountain: Rgb565,
    ridge: Rgb565,
    ground: Rgb565,
    grid: Rgb565,
    grid_far: Rgb565,
    /// Outline, body, light, dark, accent.
    hero: [Rgb565; 5],
    thing: [Rgb565; 5],
    bubble: Rgb565,
    letter: Rgb565,
    drawn: [Option<Sprite>; SPRITES],
    /// The mountains' scroll when last drawn, and the top row of each column then.
    far: i32,
    tops: [i16; MAIN as usize],
    /// The floor's moving line segments as last drawn, per row.
    segments: [[(i16, u8); MAX_SEGMENTS]; FLOOR_ROWS],
    counts: [u8; FLOOR_ROWS],
}

impl View {
    pub(super) fn new(t: &Theme) -> View {
        let horizon = mix(t.accent, t.bg, 45);
        let span = (GROUND_Y - SCENE_TOP) as u32;
        let mut sky = [t.bg; SKY_ROWS];
        for (i, c) in sky.iter_mut().enumerate() {
            *c = mix(t.bg, horizon, (i as u32 * i as u32) * 100 / (span * span));
        }
        let shade = |c: Rgb565, p: u32| mix(c, Rgb565::BLACK, p);
        let body = mix(t.fg, t.bg, 45);
        View {
            sky,
            sun_top: t.sun(),
            sun_bottom: t.accent,
            mountain: mix(t.fg, t.bg, 82),
            ridge: mix(t.accent, t.bg, 35),
            ground: mix(t.bg, t.fg, 10),
            grid: t.accent,
            grid_far: mix(t.accent, t.bg, 55),
            hero: [shade(t.fg, 70), t.echo, Rgb565::WHITE, shade(t.echo, 40), t.accent],
            thing: [shade(t.fg, 75), body, mix(body, Rgb565::WHITE, 50), shade(body, 35), t.accent],
            bubble: t.fg,
            letter: t.bg,
            drawn: [None; SPRITES],
            far: -1,
            tops: [GROUND_Y as i16; MAIN as usize],
            segments: [[(0, 0); MAX_SEGMENTS]; FLOOR_ROWS],
            counts: [0; FLOOR_ROWS],
        }
    }

    fn set_tops(&mut self, far: i32) {
        for (x, top) in self.tops.iter_mut().enumerate() {
            *top = (GROUND_Y - scene::mountain_height(x as i32 + far)) as i16;
        }
        self.far = far;
    }

    /// A floor row's moving lines for `phase`.
    fn row_segments(row: usize, phase: i32) -> ([(i16, u8); MAX_SEGMENTS], u8) {
        let mut segs = [(0, 0); MAX_SEGMENTS];
        let n = if FLOOR_LINES.contains(&(row as i32)) { 0 } else { scene::segments(row as i32, phase, MAIN, &mut segs) };
        (segs, n as u8)
    }

    /// The scenery at (x, y), as `tops` and `segments` hold it.
    fn scenery(&self, x: i32, y: i32) -> Rgb565 {
        if y < GROUND_Y {
            let top = self.tops[x as usize] as i32;
            if y > top {
                return self.mountain;
            }
            if y == top {
                return self.ridge;
            }
            let (dx, dy) = (x - SUN_X, y - SUN_Y);
            if dx * dx + dy * dy <= SUN_R * SUN_R {
                // The lower half has bands cut out, wider toward the bottom.
                let gap = dy > 0 && dy % 7 >= 7 - dy / 5;
                if !gap {
                    return mix(self.sun_top, self.sun_bottom, ((dy + SUN_R) * 100 / (2 * SUN_R)) as u32);
                }
            }
            return self.sky[(y - SCENE_TOP) as usize];
        }
        let row = y - GROUND_Y;
        if FLOOR_LINES.contains(&row) {
            return if (1..=8).contains(&row) { self.grid_far } else { self.grid };
        }
        let r = row as usize;
        let on = self.segments[r][..self.counts[r] as usize].iter().any(|&(s, l)| x >= s as i32 && x < s as i32 + l as i32);
        if on { self.grid } else { self.ground }
    }

    /// Sprite `s`'s pixel at (x, y), or None where it's clear.
    fn sprite(&self, s: &Sprite, x: i32, y: i32) -> Option<Rgb565> {
        let (lx, ly) = (x - s.r.x, y - s.r.y);
        let (art, colors) = match s.art {
            Art::Hero(frame) => ([HERO_RUN1, HERO_RUN2, HERO_JUMP][frame.min(2) as usize], &self.hero),
            Art::Thing(kind) => (thing_art(kind), &self.thing),
            Art::Bubble(c, fate) => {
                let corner = (lx == 0 || lx == BUBBLE_W - 1) && (ly == 0 || ly == BUBBLE_H - 1);
                if corner {
                    return None;
                }
                let (gx, gy) = ((lx - 2) / 2, (ly - 2) / 2);
                let on = lx >= 2 && ly >= 2 && gx < 5 && gy < 7 && gfx::glyph(c as char)[gy as usize] & (0b10000 >> gx) != 0;
                let bg = match fate {
                    Fate::Waiting => self.bubble,
                    Fate::Cleared => GOOD,
                    Fate::Crashed => BAD,
                };
                let fg = if fate == Fate::Waiting { self.letter } else { gfx::ink(bg) };
                return Some(if on { fg } else { bg });
            }
        };
        let code = *art.get((ly / 2) as usize)?.get((lx / 2) as usize)?;
        Some(match code {
            b'o' => colors[0],
            b'b' => colors[1],
            b'w' => colors[2],
            b'd' => colors[3],
            b'a' => colors[4],
            _ => return None,
        })
    }

    /// Redraws `area` of the scene with `sprites` over it.
    fn draw(&self, lcd: &mut Lcd, area: Rect, sprites: &[Option<Sprite>; SPRITES]) {
        let r = area.clip();
        if r.w == 0 || r.h == 0 {
            return;
        }
        // Only the sprites in the area; the first is on top, so the runner covers the rest.
        let mut here = [None; SPRITES];
        for (slot, s) in here.iter_mut().zip(sprites.iter()) {
            *slot = s.filter(|s| s.r.overlaps(r));
        }
        let pixel = |x: i32, y: i32| {
            here.iter().flatten().find_map(|s| if s.r.overlaps(Rect { x, y, w: 1, h: 1 }) { self.sprite(s, x, y) } else { None }).unwrap_or_else(|| self.scenery(x, y))
        };
        let colors = (r.y..r.y + r.h).flat_map(|y| (r.x..r.x + r.w).map(move |x| (x, y))).map(|(x, y)| pixel(x, y));
        let _ = lcd.fill_contiguous(&Rectangle::new(Point::new(r.x, r.y), Size::new(r.w as u32, r.h as u32)), colors);
    }

    /// Draws the run as it stands: everything with `full`, or what changed since the last frame.
    pub(super) fn frame(&mut self, lcd: &mut Lcd, d: &Dash, full: bool) {
        let sprites = sprites(d);
        let phase = scene::phase(d.scroll);
        // The mountains move an eighth as fast as the obstacles.
        let far = (d.scroll / 8) as i32;
        if full || self.far < 0 {
            self.set_tops(far);
            for row in 0..FLOOR_ROWS {
                (self.segments[row], self.counts[row]) = Self::row_segments(row, phase);
            }
            let all = Rect { x: 0, y: SCENE_TOP, w: MAIN, h: SCENE_BOTTOM - SCENE_TOP };
            self.draw(lcd, all, &sprites);
        } else {
            self.move_floor(lcd, phase);
            if far != self.far {
                let was = self.tops;
                self.set_tops(far);
                for (x, (&old, &new)) in was.iter().zip(self.tops.iter()).enumerate() {
                    if old != new {
                        let (top, bottom) = (old.min(new) as i32, old.max(new) as i32);
                        self.draw(lcd, Rect { x: x as i32, y: top, w: 1, h: bottom - top + 1 }, &sprites);
                    }
                }
            }
            for i in 0..SPRITES {
                if sprites[i] == self.drawn[i] {
                    continue;
                }
                let area = match (sprites[i], self.drawn[i]) {
                    (Some(now), Some(was)) => now.r.union(was.r),
                    (Some(now), None) => now.r,
                    (None, Some(was)) => was.r,
                    (None, None) => continue,
                };
                self.draw(lcd, area, &sprites);
            }
        }
        self.drawn = sprites;
    }

    /// Moves the floor's lines to `phase`: in each row whose lines moved, the old ones go and
    /// the new ones come.
    fn move_floor(&mut self, lcd: &mut Lcd, phase: i32) {
        for row in FIRST_LINE_ROW as usize..FLOOR_ROWS {
            let (new, n) = Self::row_segments(row, phase);
            let old = self.segments[row];
            let m = self.counts[row];
            let (new_used, old_used) = (&new[..n as usize], &old[..m as usize]);
            if new_used == old_used {
                continue;
            }
            let y = GROUND_Y + row as i32;
            for &(s, l) in old_used.iter().filter(|seg| !new_used.contains(seg)) {
                rect(lcd, s as i32, y, l as i32, 1, self.ground);
            }
            for &(s, l) in new_used.iter().filter(|seg| !old_used.contains(seg)) {
                rect(lcd, s as i32, y, l as i32, 1, self.grid);
            }
            self.segments[row] = new;
            self.counts[row] = n;
        }
    }
}

fn thing_art(kind: Kind) -> &'static [&'static [u8]] {
    match kind {
        Kind::Crate => CRATE,
        Kind::Rock => ROCK,
        Kind::Tower => TOWER,
    }
}

/// Where everything is now: the runner, then each obstacle and its bubble.
fn sprites(d: &Dash) -> [Option<Sprite>; SPRITES] {
    let mut out = [None; SPRITES];
    // The runner blinks while hurt.
    let blink = d.hurt_ms > 0 && d.hurt_ms / 100 % 2 == 1;
    if !blink {
        let y = GROUND_Y - 32 - d.hero_height();
        out[0] = Some(Sprite { r: Rect { x: HERO_X, y, w: 32, h: 32 }, art: Art::Hero(d.hero_frame()) });
    }
    for (i, o) in d.obstacles.iter().enumerate() {
        let Some(o) = o else { continue };
        let art = thing_art(o.kind);
        let (w, h) = (2 * art[0].len() as i32, 2 * art.len() as i32);
        // Standing on the horizon line, not in the floor, so the floor lines never cover them.
        let top = GROUND_Y - h;
        out[1 + 2 * i] = Some(Sprite { r: Rect { x: o.x, y: top, w, h }, art: Art::Thing(o.kind) });
        let fate = match (o.cleared, o.crashed) {
            (true, _) => Fate::Cleared,
            (_, true) => Fate::Crashed,
            _ => Fate::Waiting,
        };
        // HEAR shows the character only once it's decided.
        let c = if d.mode == Mode::Hear && fate == Fate::Waiting { b'?' } else { o.c };
        let bubble = Rect { x: o.x + w / 2 - BUBBLE_W / 2, y: top - BUBBLE_H - 4, w: BUBBLE_W, h: BUBBLE_H };
        out[2 + 2 * i] = Some(Sprite { r: bubble, art: Art::Bubble(c, fate) });
    }
    out
}
