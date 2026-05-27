use super::*;

// Monitor layout used by most tests:
//   Left  monitor: [0, 1920) x [0, 1080)
//   Right monitor: [1920, 3840) x [0, 1080)
const L0: i32 = 0;
const L1: i32 = 1920;
const R0: i32 = 1920;
const R1: i32 = 3840;
const T0: i32 = 0;
const T1: i32 = 1080;

// -- anchor_owns_window ------------------------------------------------------

/// The original bug: cursor exactly at the junction (x = 1920) was owned
/// by BOTH monitors because the old check used `<= win_gx1`.
#[test]
fn junction_belongs_to_right_monitor_only() {
    let junction = (1920, 500);
    let in_left = anchor_owns_window(junction, L0, T0, L1, T1);
    let in_right = anchor_owns_window(junction, R0, T0, R1, T1);
    assert!(!in_left, "junction must NOT be inside the left monitor");
    assert!(in_right, "junction must be inside the right monitor");
}

/// Same bug, vertical: cursor exactly at the row junction (y = 1080).
#[test]
fn junction_row_belongs_to_lower_monitor_only() {
    let anchor = (500, 1080);
    let in_top = anchor_owns_window(anchor, 0, 0, 1920, 1080);
    let in_bottom = anchor_owns_window(anchor, 0, 1080, 1920, 2160);
    assert!(!in_top, "junction row must NOT be inside the top monitor");
    assert!(in_bottom, "junction row must be inside the bottom monitor");
}

#[test]
fn interior_point_left_monitor() {
    let anchor = (960, 540);
    assert!(anchor_owns_window(anchor, L0, T0, L1, T1));
    assert!(!anchor_owns_window(anchor, R0, T0, R1, T1));
}

#[test]
fn interior_point_right_monitor() {
    let anchor = (2880, 540);
    assert!(!anchor_owns_window(anchor, L0, T0, L1, T1));
    assert!(anchor_owns_window(anchor, R0, T0, R1, T1));
}

#[test]
fn top_left_corner_owned() {
    assert!(anchor_owns_window((0, 0), L0, T0, L1, T1));
}

#[test]
fn outside_both_monitors() {
    let anchor = (5000, 500);
    assert!(!anchor_owns_window(anchor, L0, T0, L1, T1));
    assert!(!anchor_owns_window(anchor, R0, T0, R1, T1));
}

// -- size_label_pos ----------------------------------------------------------

#[test]
fn size_label_below_when_room() {
    let pos = size_label_pos(100.0, 100.0, 400.0, 1920.0, 1080.0, 96.0, 26.0, 8.0);
    assert!(pos.y > 400.0);
}

#[test]
fn size_label_above_when_bottom_full() {
    let pos = size_label_pos(100.0, 100.0, 1070.0, 1920.0, 1080.0, 96.0, 26.0, 8.0);
    assert!(pos.y < 100.0);
}

#[test]
fn size_label_stays_in_window() {
    for bottom in [200.0_f32, 600.0, 1060.0, 1079.0] {
        let pos = size_label_pos(10.0, 10.0, bottom, 1920.0, 1080.0, 96.0, 26.0, 8.0);
        assert!(
            pos.y >= 0.0 && pos.y + 26.0 <= 1081.0,
            "label y={} out of window for bottom={}",
            pos.y,
            bottom
        );
    }
}

// -- toolbar_pos -------------------------------------------------------------

#[test]
fn toolbar_above_when_room() {
    let pos = toolbar_pos(800.0, 200.0, 600.0, 1920.0, 1080.0, 90.0, 32.0, 8.0);
    assert!(pos.y < 200.0);
}

#[test]
fn toolbar_below_when_top_full() {
    let pos = toolbar_pos(800.0, 5.0, 600.0, 1920.0, 1080.0, 90.0, 32.0, 8.0);
    assert!(pos.y > 5.0);
}

#[test]
fn wider_toolbar_still_stays_above_when_room() {
    let pos = toolbar_pos(800.0, 200.0, 600.0, 1920.0, 1080.0, 140.0, 40.0, 8.0);
    assert!(pos.y < 200.0);
}

#[test]
fn size_label_anchor_stays_on_selection_corner() {
    let anchor = size_label_anchor(100, 200, 300);
    assert_eq!(anchor, (100, 499));
}

#[test]
fn cross_monitor_label_anchor_owned_by_corner_monitor() {
    let selection_x = 1800;
    let selection_y = 200;
    let selection_w = 400;
    let selection_h = 300;

    let anchor = size_label_anchor(selection_x, selection_y, selection_h);
    let in_left = anchor_owns_window(anchor, L0, T0, L1, T1);
    let in_right = anchor_owns_window(anchor, R0, T0, R1, T1);

    assert!(
        in_left,
        "size label anchor should stay on the selection corner monitor"
    );
    assert!(
        !in_right,
        "size label anchor must not jump to the opposite monitor"
    );

    let toolbar_corner = toolbar_anchor(selection_x, selection_y, selection_w);
    let toolbar_in_left = anchor_owns_window(toolbar_corner, L0, T0, L1, T1);
    let toolbar_in_right = anchor_owns_window(toolbar_corner, R0, T0, R1, T1);

    assert!(
        !toolbar_in_left,
        "toolbar corner should be on the opposite selection corner"
    );
    assert!(
        toolbar_in_right,
        "toolbar corner should be on the right monitor for this case"
    );
}
