use egui;

pub fn size_label_anchor(x: i32, y: i32, h: i32) -> (i32, i32) {
    (x, y + h - 1)
}

pub fn toolbar_anchor(x: i32, y: i32, w: i32) -> (i32, i32) {
    (x + w - 1, y)
}

/// Returns `true` if `anchor` belongs to the window described by the given
/// global-pixel rectangle `[gx0, gx1) x [gy0, gy1)`.
///
/// Right and bottom edges are exclusive so each pixel is owned by exactly
/// one monitor, which prevents duplicate UI at screen junctions.
pub fn anchor_owns_window(
    anchor: (i32, i32),
    win_gx0: i32,
    win_gy0: i32,
    win_gx1: i32,
    win_gy1: i32,
) -> bool {
    anchor.0 >= win_gx0 && anchor.0 < win_gx1 && anchor.1 >= win_gy0 && anchor.1 < win_gy1
}

/// Position for the size label: below the bottom edge, left-aligned.
/// Falls back to above the top edge, then clamps inside the window.
pub fn size_label_pos(
    sel_left: f32,
    sel_top: f32,
    sel_bottom: f32,
    win_w: f32,
    win_h: f32,
    label_w: f32,
    label_h: f32,
    gap: f32,
) -> egui::Pos2 {
    let px = sel_left.max(0.0).min((win_w - label_w).max(0.0));
    let below_y = sel_bottom + gap;
    let above_y = sel_top - label_h - gap;
    let py = if below_y + label_h <= win_h {
        below_y
    } else if above_y >= 0.0 {
        above_y
    } else {
        (win_h - label_h - gap).max(0.0)
    };
    egui::pos2(px, py)
}

/// Position for the toolbar: above the top edge, right-aligned.
/// Falls back to below the bottom edge, then clamps inside the window.
pub fn toolbar_pos(
    sel_right: f32,
    sel_top: f32,
    sel_bottom: f32,
    win_w: f32,
    win_h: f32,
    toolbar_w: f32,
    toolbar_h: f32,
    gap: f32,
) -> egui::Pos2 {
    let px = (sel_right - toolbar_w)
        .max(0.0)
        .min((win_w - toolbar_w).max(0.0));
    let above_y = sel_top - toolbar_h - gap;
    let below_y = sel_bottom + gap;
    let py = if above_y >= 0.0 {
        above_y
    } else if below_y + toolbar_h <= win_h {
        below_y
    } else {
        (win_h - toolbar_h - gap).max(0.0)
    };
    egui::pos2(px, py)
}
