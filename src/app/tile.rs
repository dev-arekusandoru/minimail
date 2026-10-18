//! Sender tiles for the Tiles row style: the plain letter tile, the four group styles (ring,
//! deck, mosaic, pair), the selected check tile and the unread badge. Geometry follows
//! `mockups/tiles-default.html`; hue/letter/layout logic lives in [`crate::tile`].

use crate::row_style::GroupTile;
use crate::theme::ThemeColor;
use crate::tile::{self, Cell, Hue};
use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, Sizable as _, tooltip::Tooltip};
use gpui_kit::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileSize {
    /// 32px, message rows and thread headers.
    Row,
    /// 24px, thread children.
    Child,
}

pub struct TileSpec {
    pub id: ElementId,
    /// Display names, newest first, distinct.
    pub senders: Vec<String>,
    pub style: GroupTile,
    pub size: TileSize,
    pub selected: bool,
    /// 0 none, 1 dot, n >= 2 number.
    pub unread: usize,
    /// Row background: the ring around the unread badge and between stacked tiles.
    pub row_bg: Hsla,
}

fn hue_color(h: Hue, t: &ThemeColor) -> Hsla {
    match h {
        Hue::Cyan => t.cyan,
        Hue::Primary => t.primary,
        Hue::Purple => t.magenta,
        Hue::Warn => t.warning,
        Hue::Green => t.green,
        Hue::Danger => t.danger,
        Hue::Yellow => t.yellow,
    }
}

/// `color` at `alpha` over the theme background (the mockup's `color-mix(... var(--bg))`).
fn tint(color: Hsla, alpha: f32, t: &ThemeColor) -> Hsla {
    t.background.blend(color.opacity(alpha))
}

/// A sender's letter tile filling its parent's `size`.
fn letter(name: &str, size: f32, radius: f32, font: f32, t: &ThemeColor) -> Div {
    let c = hue_color(tile::hue(name), t);
    div()
        .size(px(size))
        .flex_none()
        .rounded(px(radius))
        .flex()
        .items_center()
        .justify_center()
        .bg(tint(c, 0.18, t))
        .text_color(c)
        .text_size(px(font))
        .font_weight(FontWeight::SEMIBOLD)
        .child(tile::initial(name))
}

/// The neutral `+N` tile.
fn more(n: usize, radius: f32, font: f32, t: &ThemeColor) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(radius))
        .bg(t.accent)
        .text_color(t.muted_foreground)
        .text_size(px(font))
        .font_weight(FontWeight::SEMIBOLD)
        .child(tile::more_label(n))
}

fn cell(c: Cell, names: &[String], font: f32, t: &ThemeColor) -> Div {
    let tile = match c {
        Cell::Letter(i) => letter(&names[i], 0., 5., font, t).size_auto(),
        Cell::More(n) => more(n, 5., font, t),
    };
    tile.flex_1()
}

/// `inner` ringed with a 2px `row_bg` border, placed so the inner tile sits at (`x`, `y`).
fn ringed(inner: Div, x: f32, y: f32, size: f32, radius: f32, row_bg: Hsla) -> Div {
    div()
        .absolute()
        .left(px(x - 2.))
        .top(px(y - 2.))
        .size(px(size + 4.))
        .rounded(px(radius + 2.))
        .bg(row_bg)
        .p(px(2.))
        .child(inner)
}

fn deck(names: &[String], row_bg: Hsla, t: &ThemeColor) -> Div {
    let mut d = div().absolute().inset_0();
    for l in tile::deck_layers(names.len()) {
        let c = hue_color(tile::hue(&names[l.sender]), t);
        let back = div().size(px(26.)).rounded(px(7.)).bg(tint(c, 0.32, t));
        d = d.child(ringed(back, l.offset, l.offset, 26., 7., row_bg));
    }
    d.child(ringed(letter(&names[0], 26., 7., 13., t), 0., 0., 26., 7., row_bg))
}

fn mosaic(names: &[String], t: &ThemeColor) -> Div {
    let cells = tile::mosaic_cells(names.len());
    let c = |i: usize, font: f32| cell(cells[i], names, font, t);
    let frame = div().absolute().inset_0().rounded(px(8.)).bg(t.accent).p(px(2.)).gap(px(2.)).flex();
    match cells.len() {
        2 => frame.child(c(0, 11.)).child(c(1, 11.)),
        3 => frame
            .child(c(0, 12.))
            .child(div().flex_1().flex().flex_col().gap(px(2.)).child(c(1, 9.)).child(c(2, 9.))),
        _ => frame.flex_col().child(
            div().flex_1().flex().gap(px(2.)).child(c(0, 9.)).child(c(1, 9.)),
        )
        .child(div().flex_1().flex().gap(px(2.)).child(c(2, 9.)).child(c(3, 9.))),
    }
}

fn pair(names: &[String], row_bg: Hsla, t: &ThemeColor) -> Div {
    let back = div().absolute().left(px(12.)).top(px(12.)).size(px(20.)).pl(px(4.)).pt(px(4.)).flex().items_center().justify_center();
    let back = match tile::pair_back(names.len()) {
        Cell::Letter(i) => {
            let c = hue_color(tile::hue(&names[i]), t);
            back.rounded(px(6.))
                .bg(tint(c, 0.18, t))
                .text_color(c)
                .text_size(px(9.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(tile::initial(&names[i]))
        }
        Cell::More(n) => back
            .rounded(px(6.))
            .bg(t.accent)
            .text_color(t.muted_foreground)
            .text_size(px(9.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(tile::more_label(n)),
    };
    div().absolute().inset_0().child(back).child(ringed(
        letter(&names[0], 20., 6., 10., t),
        0.,
        0.,
        20.,
        6.,
        row_bg,
    ))
}

/// Latest sender inside a rounded-square ring split into one gapped segment per sender.
fn ring(names: &[String], t: &ThemeColor) -> Div {
    let segs: Vec<(Hsla, f32, f32)> = tile::ring_segments(names.len())
        .into_iter()
        .map(|(i, a, b)| (hue_color(tile::hue(&names[i]), t), a, b))
        .collect();
    let paint = canvas(
        |_, _, _| (),
        move |bounds, (), window, _| {
            let half = f32::from(bounds.size.width) / 2. - 1.;
            for (color, from, to) in &segs {
                let steps = ((to - from) / 2.).ceil().max(1.) as usize;
                let mut path = PathBuilder::stroke(px(2.));
                for s in 0..=steps {
                    let deg = from + (to - from) * s as f32 / steps as f32;
                    let (dx, dy) = tile::ring_point(deg, half, 9.);
                    let p = bounds.center() + point(px(dx), px(dy));
                    if s == 0 {
                        path.move_to(p);
                    } else {
                        path.line_to(p);
                    }
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, *color);
                }
            }
        },
    )
    .absolute()
    .size_full();
    div()
        .absolute()
        .inset_0()
        .child(paint)
        .child(letter(&names[0], 24., 6., 13., t).absolute().left(px(4.)).top(px(4.)))
}

fn badge(spec: &TileSpec, group: bool, t: &ThemeColor) -> Stateful<Div> {
    let child = spec.size == TileSize::Child;
    let (mut top, mut right) = if child { (-5., -5.) } else { (-6., -6.) };
    if group && !child {
        match spec.style {
            GroupTile::Deck => right = 0.,
            GroupTile::Pair => top = 6.,
            _ => {}
        }
    }
    let tip = tile::unread_tooltip(spec.unread);
    let el = div()
        .id((spec.id.clone(), "unread"))
        .absolute()
        .top(px(top))
        .right(px(right))
        .border_2()
        .border_color(spec.row_bg)
        .bg(t.primary)
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx));
    if spec.unread == 1 {
        let d = if child { 10. } else { 12. };
        el.size(px(d)).rounded_full()
    } else {
        el.min_w(px(16.))
            .h(px(16.))
            .px(px(3.))
            .rounded(px(8.))
            .flex()
            .items_center()
            .justify_center()
            .text_color(t.primary_foreground)
            .text_size(px(9.))
            .line_height(px(12.))
            .font_weight(FontWeight::BOLD)
            .child(spec.unread.to_string())
    }
}

/// The tile of a message, thread header or child: exactly 32x32 (24x24 for [`TileSize::Child`]),
/// with the unread badge overflowing at its top-right corner.
pub fn sender_tile(spec: TileSpec, t: &ThemeColor) -> AnyElement {
    let (side, radius, font) = match spec.size {
        TileSize::Row => (32., 8., 14.),
        TileSize::Child => (24., 6., 12.),
    };
    let group = spec.senders.len() > 1 && spec.size == TileSize::Row;
    let body = if spec.selected {
        div()
            .absolute()
            .inset_0()
            .rounded(px(radius))
            .bg(t.primary)
            .flex()
            .items_center()
            .justify_center()
            .child(Icon::new(IconName::Check).with_size(px(15.)).text_color(t.primary_foreground))
    } else if !group {
        letter(&spec.senders[0], side, radius, font, t).absolute().inset_0()
    } else {
        match spec.style {
            GroupTile::Ring => ring(&spec.senders, t),
            GroupTile::Deck => deck(&spec.senders, spec.row_bg, t),
            GroupTile::Mosaic => mosaic(&spec.senders, t),
            GroupTile::Pair => pair(&spec.senders, spec.row_bg, t),
        }
    };
    let ub = (spec.unread > 0).then(|| badge(&spec, group, t));
    div()
        .id(spec.id.clone())
        .relative()
        .size(px(side))
        .flex_none()
        .child(body)
        .children(ub)
        .into_any_element()
}
