use mail_classifier::tile::{
    Cell, DeckLayer, Hue, deck_layers, hue, hue_index, initial, mosaic_cells, pair_back,
    ring_point, ring_segments, unread_tooltip,
};

#[test]
fn hue_matches_the_mockup_hash() {
    // Values computed with the mockup's `reduce((a,c)=>(a*31+c.charCodeAt(0))>>>0, 7) % 7`.
    assert_eq!(hue_index("helumi"), 4);
    assert_eq!(hue_index("Apple Developer"), 4);
    assert_eq!(hue_index("GitHub"), 2);
    assert_eq!(hue_index("Trevor Shan"), 4);
    assert_eq!(hue_index("a"), 6);
    assert_eq!(hue_index(""), 0);
    assert_eq!(hue("GitHub"), Hue::Purple);
    assert_eq!(hue("a"), Hue::Yellow);
}

#[test]
fn initial_is_the_uppercased_first_char() {
    assert_eq!(initial("helumi"), "H");
    assert_eq!(initial("élan"), "É");
    assert_eq!(initial(""), "?");
}

#[test]
fn ring_splits_into_gapped_segments_capped_at_six() {
    assert_eq!(ring_segments(2), vec![(0, 0., 166.), (1, 180., 346.)]);
    assert_eq!(ring_segments(9).len(), 6);
    assert_eq!(ring_segments(3)[1], (1, 120., 226.));
}

#[test]
fn ring_points_sit_on_the_rounded_square() {
    let near = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3;
    assert!(near(ring_point(0., 15., 9.), (0., -15.)));
    assert!(near(ring_point(90., 15., 9.), (15., 0.)));
    assert!(near(ring_point(180., 15., 9.), (0., 15.)));
    // The diagonal hits the corner arc, not the box corner.
    let (x, y) = ring_point(45., 15., 9.);
    let d = ((x - 6.).powi(2) + (y + 6.).powi(2)).sqrt();
    assert!((d - 9.).abs() < 1e-3, "corner point {x},{y} is {d} from the arc centre");
}

#[test]
fn deck_has_one_or_two_back_layers() {
    assert!(deck_layers(1).is_empty());
    assert_eq!(deck_layers(2), vec![DeckLayer { sender: 1, offset: 3. }]);
    assert_eq!(
        deck_layers(5),
        vec![DeckLayer { sender: 2, offset: 6. }, DeckLayer { sender: 1, offset: 3. }]
    );
}

#[test]
fn mosaic_shows_up_to_four_then_a_more_cell() {
    assert_eq!(mosaic_cells(2), vec![Cell::Letter(0), Cell::Letter(1)]);
    assert_eq!(mosaic_cells(4).len(), 4);
    assert_eq!(
        mosaic_cells(7),
        vec![Cell::Letter(0), Cell::Letter(1), Cell::Letter(2), Cell::More(4)]
    );
}

#[test]
fn pair_back_is_the_second_sender_or_a_more_cell() {
    assert_eq!(pair_back(2), Cell::Letter(1));
    assert_eq!(pair_back(3), Cell::More(2));
    assert_eq!(pair_back(6), Cell::More(5));
}

#[test]
fn unread_tooltip_pluralises() {
    assert_eq!(unread_tooltip(1), "1 unread message");
    assert_eq!(unread_tooltip(3), "3 unread messages");
}
