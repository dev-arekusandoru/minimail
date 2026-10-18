//! How message-list rows look: the user's row settings and the rules for combining them.
//! Independent of GPUI; the views in `src/app/` read a [`RowStyle`], never the raw settings.
//!
//! Collisions are handled by the types: the timeline rail and sender tiles are one [`Leading`]
//! choice, so both can never be on. Compact density ignores everything that needs a second
//! line, which the `effective`-style accessors below encode in one place.
use serde::{Deserialize, Serialize};

/// Width of the list panel (px) under which Compact rows drop the inline preview, shrink the
/// sender column and cap the icon cluster.
pub const NARROW_LIST_W: f32 = 480.;

/// Standard rows (two or three lines) or Compact rows (one line).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    #[default]
    Standard,
    Compact,
}

/// What leads a Standard row: the timeline rail with the time in a gutter, or a sender tile.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Leading {
    #[default]
    Rail,
    Tiles,
}

/// How a collapsed thread with several senders draws its tile (sender tiles only).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupTile {
    #[default]
    Ring,
    Deck,
    Mosaic,
    Pair,
}

/// Where the suggested-folder and urgency chips sit on a Standard row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Chips {
    #[default]
    Off,
    Above,
    Below,
}

/// The user's row settings, read together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowStyle {
    pub density: Density,
    pub leading: Leading,
    pub group_tile: GroupTile,
    pub chips: Chips,
    /// Subject on the first line (default). Off puts the sender first, as the old rows did.
    pub subject_first: bool,
}

impl Default for RowStyle {
    fn default() -> Self {
        Self {
            density: Density::default(),
            leading: Leading::default(),
            group_tile: GroupTile::default(),
            chips: Chips::default(),
            subject_first: true,
        }
    }
}

impl RowStyle {
    pub fn compact(&self) -> bool {
        self.density == Density::Compact
    }

    /// Standard rows with the timeline rail.
    pub fn rail(&self) -> bool {
        !self.compact() && self.leading == Leading::Rail
    }

    /// Standard rows with sender tiles.
    pub fn tiles(&self) -> bool {
        !self.compact() && self.leading == Leading::Tiles
    }

    /// Where chips go on this row; `Off` in Compact, which has no room for them.
    pub fn chips_placement(&self) -> Chips {
        if self.compact() { Chips::Off } else { self.chips }
    }

    /// Folder and urgency show as icons in the right-hand cluster exactly when no chips carry them.
    pub fn chips_as_icons(&self) -> bool {
        self.chips_placement() == Chips::Off
    }

    /// Subject on the first line. Compact always reads sender, then subject, on its one line.
    pub fn subject_first(&self) -> bool {
        !self.compact() && self.subject_first
    }

    /// The tile style for collapsed multi-sender threads; only meaningful with [`Self::tiles`].
    pub fn group_tile(&self) -> GroupTile {
        self.group_tile
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_ignores_standard_only_options() {
        let s = RowStyle {
            density: Density::Compact,
            leading: Leading::Tiles,
            chips: Chips::Above,
            subject_first: true,
            ..RowStyle::default()
        };
        assert!(s.compact() && !s.rail() && !s.tiles());
        assert_eq!(s.chips_placement(), Chips::Off);
        assert!(s.chips_as_icons());
        assert!(!s.subject_first());
    }

    #[test]
    fn rail_and_tiles_are_exclusive() {
        let mut s = RowStyle::default();
        assert!(s.rail() && !s.tiles());
        s.leading = Leading::Tiles;
        assert!(s.tiles() && !s.rail());
    }

    #[test]
    fn chips_replace_cluster_icons() {
        let mut s = RowStyle::default();
        assert!(s.chips_as_icons());
        s.chips = Chips::Below;
        assert!(!s.chips_as_icons());
    }
}
