//! Sender tiles: the letter, hue and group geometry behind the tiles that replace the timeline
//! rail. Pure; `app::tile` paints it.

/// Palette slots, in the order the hue hash indexes them. `app::tile` maps each to a theme token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hue {
    Cyan,
    Primary,
    Purple,
    Warn,
    Green,
    Danger,
    Yellow,
}

const PALETTE: [Hue; 7] =
    [Hue::Cyan, Hue::Primary, Hue::Purple, Hue::Warn, Hue::Green, Hue::Danger, Hue::Yellow];

/// Palette index of a sender: `fold(a * 31 + unit, 7) mod 7` over UTF-16 code units, wrapping at
/// 32 bits (the mockup's `>>> 0`).
pub fn hue_index(name: &str) -> usize {
    let h = name.encode_utf16().fold(7u32, |a, c| a.wrapping_mul(31).wrapping_add(u32::from(c)));
    (h % PALETTE.len() as u32) as usize
}

pub fn hue(name: &str) -> Hue {
    PALETTE[hue_index(name)]
}

/// The tile letter: first character uppercased (`?` for an empty name).
pub fn initial(name: &str) -> String {
    name.chars().next().map_or_else(|| "?".to_string(), |c| c.to_uppercase().collect())
}

/// Most senders a ring splits into segments.
pub const RING_MAX: usize = 6;
/// Gap between ring segments, in degrees.
pub const RING_GAP_DEG: f32 = 14.;

/// Ring segments as `(sender index, start°, end°)` clockwise from 12 o'clock: one per sender
/// (newest first, at most [`RING_MAX`]), each leaving [`RING_GAP_DEG`] before the next.
pub fn ring_segments(senders: usize) -> Vec<(usize, f32, f32)> {
    let k = senders.min(RING_MAX);
    let s = 360. / k.max(1) as f32;
    (0..k).map(|i| (i, i as f32 * s, (i + 1) as f32 * s - RING_GAP_DEG)).collect()
}

/// Offset from the centre of a rounded square (half-size `half`, corner radius `radius`) to its
/// outline along the ray at `deg` clockwise from 12 o'clock; this is where a conic gradient's
/// angle lands. Returns `(dx, dy)` with y down.
pub fn ring_point(deg: f32, half: f32, radius: f32) -> (f32, f32) {
    let (dx, dy) = (deg.to_radians().sin(), -deg.to_radians().cos());
    let t = half / dx.abs().max(dy.abs());
    let (px, py) = (t * dx, t * dy);
    let c = half - radius;
    if px.abs() <= c || py.abs() <= c {
        return (px, py);
    }
    // In a corner: hit the corner circle centred at (±c, ±c).
    let (cx, cy) = (c * px.signum(), c * py.signum());
    let b = dx * cx + dy * cy;
    let t = b + (b * b - (cx * cx + cy * cy) + radius * radius).sqrt();
    (t * dx, t * dy)
}

/// A back layer of a deck: which sender it shows (index into the newest-first list) and its
/// offset right and down from the front tile, in px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeckLayer {
    pub sender: usize,
    pub offset: f32,
}

/// Deck back layers, bottom-most first: two for 3+ senders (offsets 6, 3), one for 2.
pub fn deck_layers(senders: usize) -> Vec<DeckLayer> {
    match senders {
        0 | 1 => vec![],
        2 => vec![DeckLayer { sender: 1, offset: 3. }],
        _ => vec![DeckLayer { sender: 2, offset: 6. }, DeckLayer { sender: 1, offset: 3. }],
    }
}

/// A cell of a mosaic or the back tile of a pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell {
    /// The letter tile of the sender at this index.
    Letter(usize),
    /// Neutral `+N` tile for the senders not shown.
    More(usize),
}

/// Mosaic cells in reading order: every sender up to 4; past 4 the first three plus `+(n-3)`.
pub fn mosaic_cells(senders: usize) -> Vec<Cell> {
    if senders > 4 {
        (0..3).map(Cell::Letter).chain([Cell::More(senders - 3)]).collect()
    } else {
        (0..senders).map(Cell::Letter).collect()
    }
}

/// The tile behind a pair's front (latest) letter: the second sender for two senders, else `+(n-1)`.
pub fn pair_back(senders: usize) -> Cell {
    if senders > 2 { Cell::More(senders - 1) } else { Cell::Letter(1) }
}

/// `+N` label of a neutral cell.
pub fn more_label(n: usize) -> String {
    format!("+{n}")
}

/// Tooltip of the unread badge.
pub fn unread_tooltip(n: usize) -> String {
    if n == 1 { "1 unread message".to_string() } else { format!("{n} unread messages") }
}
