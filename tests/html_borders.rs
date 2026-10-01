use blitz_traits::shell::ColorScheme;
use mail_classifier::html;

#[test]
fn borderless_collapsed_layout_tables_stay_borderless() {
    let source = "<body style='margin:0;background:white'>\
        <table role='presentation' border='0' cellspacing='0' cellpadding='0' style='border-collapse:collapse'>\
        <tr><td><table border='0' style='border-collapse:collapse'><tr>\
        <td style='width:80px;height:40px;padding:0'></td>\
        <td style='width:80px;height:40px;padding:0'></td>\
        </tr></table></td></tr></table></body>";
    let rendered = html::render(source, 200, 1.0, true, ColorScheme::Light).unwrap();
    assert!(
        rendered
            .pixels
            .chunks_exact(4)
            .all(|pixel| pixel == [255, 255, 255, 255]),
        "a borderless presentation table acquired painted borders"
    );
}

#[test]
fn collapsed_cell_borders_respect_each_edges_style_and_color() {
    for (edge, horizontal) in [("left", false), ("top", true)] {
        let source = format!(
            "<body style='margin:0;background:white'>\
            <table border='0' cellspacing='0' cellpadding='0' style='border-collapse:collapse'>\
            <tr><td style='padding:0;width:80px;height:40px;border:none;border-{edge}:4px solid red'></td></tr>\
            </table></body>"
        );
        let rendered = html::render(&source, 200, 1.0, true, ColorScheme::Light).unwrap();
        let mut bounds = (u32::MAX, u32::MAX, 0, 0);
        for (i, pixel) in rendered.pixels.chunks_exact(4).enumerate() {
            assert!(
                pixel[0] > 240,
                "an unintended black border was painted on {edge}"
            );
            if pixel[1] < 30 && pixel[2] < 30 {
                let x = i as u32 % rendered.width;
                let y = i as u32 / rendered.width;
                bounds.0 = bounds.0.min(x);
                bounds.1 = bounds.1.min(y);
                bounds.2 = bounds.2.max(x);
                bounds.3 = bounds.3.max(y);
            }
        }
        assert_ne!(
            bounds.0,
            u32::MAX,
            "the requested red {edge} border disappeared"
        );
        let painted_width = bounds.2 - bounds.0 + 1;
        let painted_height = bounds.3 - bounds.1 + 1;
        if horizontal {
            assert!(
                painted_width >= 70 && painted_height <= 5,
                "the top-only border painted other edges: {bounds:?}"
            );
        } else {
            assert!(
                painted_width <= 5 && painted_height >= 30,
                "the left-only border painted other edges: {bounds:?}"
            );
        }
    }
}
