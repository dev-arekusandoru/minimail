//! Compact rows: which cluster icons survive in a narrow list. Independent of GPUI drawing.
use crate::app::icons::Glyph;

/// Glyphs kept when space is tight, most severe first. Glyphs not listed rank after these.
const SEVERITY: [Glyph; 8] = [
    Glyph::PossibleSpam,
    Glyph::UrgentHigh,
    Glyph::NewSender,
    Glyph::UrgentMid,
    Glyph::Attachment,
    Glyph::FolderSuggestion,
    Glyph::Muted,
    Glyph::Snoozed,
];

/// Narrow lists keep this many non-kind icons.
const NARROW_KEEP: usize = 2;

fn is_kind(g: Glyph) -> bool {
    matches!(
        g,
        Glyph::KindPerson | Glyph::KindReceipt | Glyph::KindNewsletter | Glyph::KindNotification | Glyph::KindOther
    )
}

fn severity(g: Glyph) -> usize {
    SEVERITY.iter().position(|&s| s == g).unwrap_or(SEVERITY.len())
}

/// The cluster icons a Compact row shows, in their original order: all of them normally; in a
/// narrow list the kind icon plus the two most severe others.
pub fn cluster_icons(glyphs: &[Glyph], narrow: bool) -> Vec<Glyph> {
    if !narrow {
        return glyphs.to_vec();
    }
    let mut ranked: Vec<Glyph> = glyphs.iter().copied().filter(|&g| !is_kind(g)).collect();
    ranked.sort_by_key(|&g| severity(g));
    ranked.truncate(NARROW_KEEP);
    glyphs.iter().copied().filter(|&g| is_kind(g) || ranked.contains(&g)).collect()
}
