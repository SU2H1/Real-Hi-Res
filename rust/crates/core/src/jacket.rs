//! The cover that turns over like a record sleeve (2.5, `docs/player-spec.md`,
//! "2.5", item 2): the port of Python's `realhires/jacket.py`.
//!
//! On Now Playing the album's songs are the back of the jacket: they are drawn
//! inside the cover's own square, over the cover mirrored and blurred
//! (`covers::back_picture()`), and a swipe, a dot or the arrow keys turn the
//! jacket about its vertical axis.  This module is what both desktop apps share:
//! the timing and the geometry of the turn (`turn_frame()`), what a drag does
//! (`drag_progress()`, `drag_finishes()`) and the measures of the back
//! (`back_layout()`).  Checked against `tests/python_jacket.json`
//! (`rust/tools/gen_jacket_vectors.py`).

use crate::pyfmt::round_int;

/// A whole turn, eased in and out.
pub const FLIP_MS: i64 = 360;
/// The back's text fades in when the turn has ended.
pub const FADE_MS: i64 = 120;
/// How much a face is darkened when it is edge-on.
pub const DARKEN: f64 = 0.35;
/// How much shorter the far edge is drawn, of the height, when edge-on.
pub const PERSPECTIVE: f64 = 0.06;
/// A drag this long (or a quarter of the cover's width) finishes the turn.
pub const SWIPE_PX: i64 = 60;
/// A sideways scroll this far turns the jacket.
pub const SCROLL_PX: i64 = 30;
/// The row under the pointer: white at this strength.
pub const HOVER_TINT: f64 = 0.12;
/// The back's secondary texts: white at this strength.
pub const SECONDARY: f64 = 0.70;
pub const SIDES: [&str; 2] = ["cover", "songs"];
/// Tests: the turn and the fade take no time.
pub const NO_ANIMATION_ENV: &str = "REALHIRES_NO_ANIMATION";

/// How long a whole turn takes: `FLIP_MS`, or 0 with `$REALHIRES_NO_ANIMATION`.
pub fn flip_ms() -> i64 {
    match std::env::var(NO_ANIMATION_ENV) {
        Ok(v) if !matches!(v.trim(), "" | "0") => 0,
        _ => FLIP_MS,
    }
}

pub fn fade_ms() -> i64 {
    if flip_ms() == 0 {
        0
    } else {
        FADE_MS
    }
}

/// Ease in-out (cubic): 0 -> 0, 0.5 -> 0.5, 1 -> 1, slow at both ends.
pub fn ease(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (2.0 - 2.0 * t).powi(3) / 2.0
    }
}

/// One moment of the turn.  `other`: the face being turned to is the one seen
/// (past the half-way point); `width`: the face's width as a part of the cover's
/// (1 = facing, 0 = edge-on); `far_right`: its right edge is the far one, drawn
/// `far` of the height (the near edge keeps the whole height, so the jacket never
/// leaves its square); `dark`: how much black lies over it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TurnFrame {
    pub other: bool,
    pub width: f64,
    pub far_right: bool,
    pub far: f64,
    pub dark: f64,
}

/// The jacket `progress` (0..1) of the way through a turn about its vertical
/// axis.  `direction` > 0: the pointer went to the right (the left edge comes
/// forward and crosses to the right), < 0 the other way.
pub fn turn_frame(progress: f64, direction: i32) -> TurnFrame {
    let p = progress.clamp(0.0, 1.0);
    let angle = std::f64::consts::PI * p;
    let s = angle.sin();
    let other = p > 0.5;
    TurnFrame { other, width: angle.cos().abs(), far_right: (direction > 0) != other, far: 1.0 - PERSPECTIVE * s, dark: DARKEN * s }
}

/// How far a drag of `dx` pixels has turned the jacket: the cover's width is a
/// half turn (from one face to the other).
pub fn drag_progress(dx: f64, width: f64) -> f64 {
    if width > 0.0 {
        (dx.abs() / width).min(1.0)
    } else {
        0.0
    }
}

/// Whether a drag let go at `dx` pixels turns the jacket over (else it springs
/// back): past `SWIPE_PX` or a quarter of the cover's width, whichever is less.
pub fn drag_finishes(dx: f64, width: f64) -> bool {
    dx.abs() > (SWIPE_PX as f64).min(width / 4.0)
}

/// The time the rest of a turn takes when the pointer lets go at `progress`: the
/// part of `whole_ms` that is left to the end (`finish`) or back to the start.
pub fn rest_ms(progress: f64, finish: bool, whole_ms: i64) -> i64 {
    let p = progress.clamp(0.0, 1.0);
    round_int(whole_ms as f64 * if finish { 1.0 - p } else { p })
}

/// Where an eased move from `start` to `target` is after `elapsed_ms` of `total_ms`.
pub fn progress_at(start: f64, target: f64, elapsed_ms: f64, total_ms: f64) -> f64 {
    if total_ms <= 0.0 || elapsed_ms >= total_ms {
        return target;
    }
    start + (target - start) * ease(elapsed_ms.max(0.0) / total_ms)
}

pub fn other_side(side: &str) -> &'static str {
    if side == "songs" {
        "cover"
    } else {
        "songs"
    }
}

/// The measures of the back of a jacket `size` pixels a side: the album's title
/// (bold, `title_px`) at (`pad`, `title_y`), its artist (`small_px`) at
/// `artist_y` (the middles of the lines), then `rows` rows of `row_h` from
/// `list_top`; in a row the number ends at `num_right`, the title starts at
/// `title_x` and the time ends at `time_right`; the thin scroll bar is `bar_w`
/// wide and ends at `bar_right`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackLayout {
    pub size: i64,
    pub pad: i64,
    pub title_px: i64,
    pub text_px: i64,
    pub small_px: i64,
    pub title_y: i64,
    pub artist_y: i64,
    pub list_top: i64,
    pub row_h: i64,
    pub rows: i64,
    pub num_right: i64,
    pub title_x: i64,
    pub time_right: i64,
    pub bar_w: i64,
    pub bar_right: i64,
}

pub fn back_layout(size: i64) -> BackLayout {
    let size = size.max(1);
    let step = if size >= 400 {
        3
    } else if size >= 300 {
        2
    } else if size >= 220 {
        1
    } else {
        0
    };
    let pad = 8.max((size * 5 + 50) / 100);
    let title_px = [12, 14, 16, 18][step];
    let text_px = [11, 12, 13, 14][step];
    let small_px = [10, 11, 12, 13][step];
    let row_h = [22, 26, 30, 34][step];
    let title_y = pad + title_px / 2 + 2;
    let artist_y = title_y + title_px / 2 + small_px / 2 + 5;
    let list_top = artist_y + small_px / 2 + 8;
    let bottom = size - 6.max(pad / 2);
    let rows = 1.max((bottom - list_top).div_euclid(row_h));
    let num_right = pad + text_px * 3 / 2;
    BackLayout {
        size,
        pad,
        title_px,
        text_px,
        small_px,
        title_y,
        artist_y,
        list_top,
        row_h,
        rows,
        num_right,
        title_x: num_right + 8,
        time_right: size - pad,
        bar_w: 3,
        bar_right: size - 3,
    }
}

/// The first row shown, kept so that the list never scrolls past its end.
pub fn clamp_first(first: i64, total: i64, rows: i64) -> i64 {
    first.max(0).min(0.max(total - rows))
}

/// The first row shown so that row `current` (the song playing) is in view.
pub fn first_for(current: i64, first: i64, total: i64, rows: i64) -> i64 {
    let mut first = first;
    if current < 0 {
        return clamp_first(first, total, rows);
    }
    if current < first {
        first = current;
    } else if current >= first + rows {
        first = current - rows + 1;
    }
    clamp_first(first, total, rows)
}

/// The thin bar of a list of `total` rows showing `rows` from `first`, in a
/// track `length` pixels long: (offset, length) in pixels, or (0, 0) when
/// everything fits.
pub fn scroll_bar(first: i64, total: i64, rows: i64, length: i64) -> (i64, i64) {
    if total <= rows || length <= 0 {
        return (0, 0);
    }
    let bar = 12.max(length * rows / total).min(length);
    let off = (length - bar) * clamp_first(first, total, rows) / (total - rows);
    (off, bar)
}

/// Colour `b` over colour `a` at `amount` (0..1), each (r, g, b), rounded.
pub fn mix(a: [u8; 3], b: [u8; 3], amount: f64) -> [u8; 3] {
    let k = round_int(amount.clamp(0.0, 1.0) * 256.0);
    let m = |x: u8, y: u8| ((x as i64 * (256 - k) + y as i64 * k + 128) >> 8) as u8;
    [m(a[0], b[0]), m(a[1], b[1]), m(a[2], b[2])]
}
