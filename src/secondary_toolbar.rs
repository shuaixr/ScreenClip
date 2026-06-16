use screenshots::image::Rgba;

use crate::overlay_ui::{ColorTarget, OverlayAction};

/// Fixed preset palette for the first iteration. No configuration, no picker.
pub const COLOR_SWATCHES: &[Rgba<u8>] = &[
    Rgba([255, 255, 255, 255]), // White
    Rgba([0,   0,   0,   255]), // Black
    Rgba([231, 76,  60,  255]), // Red
    Rgba([241, 196, 15,  255]), // Yellow
    Rgba([46,  204, 113, 255]), // Green
    Rgba([26,  179, 255, 255]), // Blue (default for rectangle)
];

const BTN: f32 = 24.0;
const GAP: f32 = 4.0;
const PAD: f32 = 6.0;
const GAP_FROM_PRIMARY: f32 = 4.0;

pub fn toolbar_size() -> (f32, f32) {
    let n = COLOR_SWATCHES.len() as f32;
    let toolbar_w = PAD * 2.0 + n * BTN + (n - 1.0) * GAP;
    let toolbar_h = PAD * 2.0 + BTN;
    (toolbar_w, toolbar_h)
}

/// Compute the position of the secondary toolbar in window-local logical pixels.
///
/// The secondary toolbar must never overlap the selection box. Priorities:
/// 1. **Same row, left of primary** (preferred — both form one visual row).
/// 2. **Above primary** (in the gap toward the screen top).
/// 3. **Below primary**, but only if it stays above the selection top.
/// 4. **Clamp to screen bottom** (last resort, may overlap selection).
///
/// `sel_top` is the selection's top edge in window-local logical pixels; it is
/// the only piece of selection geometry we need to keep clear.
pub fn compute_pos(
    primary_pos: egui::Pos2,
    primary_size: egui::Vec2,
    win_w: f32,
    win_h: f32,
    sel_top: f32,
) -> egui::Pos2 {
    let (toolbar_w, toolbar_h) = toolbar_size();
    let right_edge = primary_pos.x + primary_size.x;

    // 1. Same row, immediately left of primary (gap on the right of secondary,
    //    so its right edge sits one gap short of primary's left edge).
    let left_x = primary_pos.x - GAP_FROM_PRIMARY - toolbar_w;
    if left_x >= 0.0 {
        return egui::pos2(left_x, primary_pos.y);
    }

    // 2 & 3 share a right-aligned x: secondary's right edge = primary's right edge.
    let aligned_left_x = right_edge - toolbar_w;

    // 2. Above primary, right-aligned.
    let above_y = primary_pos.y - toolbar_h - GAP_FROM_PRIMARY;
    if above_y >= 0.0 {
        return egui::pos2(aligned_left_x, above_y);
    }

    // 3. Below primary, right-aligned, strictly above the selection.
    let below_y = primary_pos.y + primary_size.y + GAP_FROM_PRIMARY;
    if below_y + toolbar_h <= sel_top {
        return egui::pos2(aligned_left_x, below_y);
    }

    // 4. Last resort: clamp to screen bottom (selection overlap accepted).
    let fallback_y = (win_h - toolbar_h - GAP_FROM_PRIMARY).max(0.0);
    egui::pos2(
        aligned_left_x.min((win_w - toolbar_w).max(0.0)),
        fallback_y,
    )
}

/// Renders the color swatch toolbar and returns the action, if any.
pub fn draw(
    ctx: &egui::Context,
    target: ColorTarget,
    current_color: Rgba<u8>,
    pos: egui::Pos2,
) -> OverlayAction {
    let mut action = OverlayAction::None;

    egui::Area::new(egui::Id::new("secondary_toolbar"))
        .fixed_pos(pos)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgba_unmultiplied(30, 30, 30, 220))
                .corner_radius(egui::CornerRadius::same(6))
                .inner_margin(egui::Margin::same(PAD as i8))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = GAP;
                        for swatch in COLOR_SWATCHES {
                            if color_swatch_button(ui, *swatch, BTN, *swatch == current_color) {
                                action = OverlayAction::SetAnnotationColor(target, *swatch);
                            }
                        }
                    });
                });
        });

    action
}

fn color_swatch_button(ui: &mut egui::Ui, swatch: Rgba<u8>, size: f32, active: bool) -> bool {
    let color = egui::Color32::from_rgba_unmultiplied(
        swatch[0], swatch[1], swatch[2], swatch[3],
    );
    let border = if active {
        egui::Stroke::new(2.0, egui::Color32::WHITE)
    } else {
        egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 60))
    };
    let response = ui.add(
        egui::Button::new("")
            .min_size(egui::vec2(size, size))
            .fill(color)
            .stroke(border)
            .corner_radius(egui::CornerRadius::same(4)),
    );
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRIMARY_W: f32 = 184.0;
    const PRIMARY_H: f32 = 40.0;

    /// Standard case: primary is above the selection, plenty of room to the
    /// left. The secondary should land on the same row, immediately left of primary.
    #[test]
    fn primary_left_of_secondary_when_room() {
        let primary_pos = egui::pos2(800.0, 100.0);
        let primary_size = egui::vec2(PRIMARY_W, PRIMARY_H);
        let pos = compute_pos(primary_pos, primary_size, 1920.0, 1080.0, 140.0);
        let (w, h) = toolbar_size();
        // Same y as primary; secondary's right edge sits one gap short of
        // primary's left edge (so the two toolbars form one continuous row).
        assert_eq!(pos.y, primary_pos.y);
        assert_eq!(pos.x + w + GAP_FROM_PRIMARY, primary_pos.x);
        assert!(pos.x >= 0.0);
        // Must not overlap the selection (which starts at y = 140).
        assert!(pos.y + h <= 140.0);
    }

    /// Left branch clips because primary is too close to the left edge —
    /// secondary must fall back to above the primary, right-aligned.
    #[test]
    fn falls_back_above_primary_when_left_clips() {
        // primary_pos.x < GAP_FROM_PRIMARY + toolbar_w forces branch 1 to fail.
        let primary_pos = egui::pos2(50.0, 100.0);
        let primary_size = egui::vec2(PRIMARY_W, PRIMARY_H);
        let pos = compute_pos(primary_pos, primary_size, 1920.0, 1080.0, 140.0);
        let (w, h) = toolbar_size();
        // Right edge aligned with primary's right edge.
        assert_eq!(pos.x + w, primary_pos.x + primary_size.x);
        // Above primary, in the gap toward the screen top.
        assert!(pos.y + h + GAP_FROM_PRIMARY <= primary_pos.y);
        assert!(pos.y >= 0.0);
    }

    /// Both left-of and above are impossible (primary is near the top of the
    /// screen) but below still has clearance above the selection — that branch
    /// should be used.
    #[test]
    fn falls_back_below_primary_when_left_and_above_clipped() {
        // primary_pos.x = 50.0 forces branch 1 to fail; primary_pos.y = 20.0
        // forces branch 2 to fail; sel_top = 200.0 leaves clearance for branch 3.
        let primary_pos = egui::pos2(50.0, 20.0);
        let primary_size = egui::vec2(PRIMARY_W, PRIMARY_H);
        let pos = compute_pos(primary_pos, primary_size, 1920.0, 1080.0, 200.0);
        let (w, h) = toolbar_size();
        // Right edge aligned with primary's right edge.
        assert_eq!(pos.x + w, primary_pos.x + primary_size.x);
        // Below primary, must end strictly above the selection top.
        assert!(pos.y >= primary_pos.y + primary_size.y);
        assert!(pos.y + h <= 200.0);
    }

    /// Every fallback exhausted (squeezed corner) — the result must still be
    /// within the window, even if it overlaps the selection.
    #[test]
    fn last_resort_clamps_inside_window() {
        let primary_pos = egui::pos2(50.0, 20.0);
        let primary_size = egui::vec2(PRIMARY_W, PRIMARY_H);
        let win_w = 1920.0;
        let win_h = 1080.0;
        // Selection also at the top, so "below primary" would overlap.
        let pos = compute_pos(primary_pos, primary_size, win_w, win_h, 30.0);
        let (w, h) = toolbar_size();
        assert!(pos.x >= 0.0 && pos.x + w <= win_w);
        assert!(pos.y >= 0.0 && pos.y + h <= win_h);
    }
}
