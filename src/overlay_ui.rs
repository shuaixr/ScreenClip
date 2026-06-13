use egui;
use winit::dpi::PhysicalPosition;

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

/// Below the bottom edge, left-aligned. Falls back to above the top edge,
/// then clamps inside the window.
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

/// Above the top edge, right-aligned. Falls back to below the bottom edge,
/// then clamps inside the window.
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

/// Draws a square icon-only button. Returns true when clicked.
fn icon_button(ui: &mut egui::Ui, icon: &str, size: f32) -> bool {
    let rich = egui::RichText::new(icon)
        .color(egui::Color32::WHITE)
        .size(size * 0.55);
    let btn = egui::Button::new(rich)
        .min_size(egui::vec2(size, size))
        .corner_radius(egui::CornerRadius::same(4));
    ui.add(btn)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// Action requested by an overlay widget during a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayAction {
    None,
    Save,
    Copy,
    StartTextInsert,
    Exit,
}

/// Modular overlay UI — add more widgets by calling more draw_* helpers in `draw`.
pub struct OverlayUi;

impl OverlayUi {
    pub fn new() -> Self {
        Self
    }

    /// Draws all overlay widgets for one window frame.
    ///
    /// Pass `show_toolbar = true` only after the selection is confirmed (drag finished).
    pub fn draw(
        &self,
        ctx: &egui::Context,
        active_rect: Option<(i32, i32, i32, i32)>,
        _cursor_global: Option<(i32, i32)>,
        show_toolbar: bool,
        window_origin: PhysicalPosition<i32>,
        window_size: (u32, u32),
        pixels_per_point: f32,
    ) -> OverlayAction {
        let Some((x, y, w, h)) = active_rect else {
            return OverlayAction::None;
        };
        if w <= 0 || h <= 0 {
            return OverlayAction::None;
        }

        // Widget ownership uses deterministic selection corners so a cross-monitor
        // selection never "jumps" UI between windows while the cursor moves.
        let win_gx0 = window_origin.x;
        let win_gy0 = window_origin.y;
        let win_gx1 = win_gx0 + window_size.0 as i32;
        let win_gy1 = win_gy0 + window_size.1 as i32;
        let label_anchor = size_label_anchor(x, y, h);
        let toolbar_anchor = toolbar_anchor(x, y, w);
        let owns_label = anchor_owns_window(label_anchor, win_gx0, win_gy0, win_gx1, win_gy1);
        let owns_toolbar = anchor_owns_window(toolbar_anchor, win_gx0, win_gy0, win_gx1, win_gy1);

        if !owns_label && (!show_toolbar || !owns_toolbar) {
            return OverlayAction::None;
        }

        let to_l = |phys: f32| phys / pixels_per_point;
        let local_left = to_l((x - window_origin.x) as f32);
        let local_right = to_l((x + w - window_origin.x) as f32);
        let local_top = to_l((y - window_origin.y) as f32);
        let local_bottom = to_l((y + h - window_origin.y) as f32);
        let win_w = to_l(window_size.0 as f32);
        let win_h = to_l(window_size.1 as f32);

        if owns_label {
            self.draw_size_label(ctx, w, h, local_left, local_top, local_bottom, win_w, win_h);
        }

        if show_toolbar && owns_toolbar {
            return self.draw_toolbar(ctx, local_right, local_top, local_bottom, win_w, win_h);
        }

        OverlayAction::None
    }

    fn draw_size_label(
        &self,
        ctx: &egui::Context,
        w: i32,
        h: i32,
        local_left: f32,
        local_top: f32,
        local_bottom: f32,
        win_w: f32,
        win_h: f32,
    ) {
        let text = format!("{} × {}", w, h);
        let pos = size_label_pos(
            local_left,
            local_top,
            local_bottom,
            win_w,
            win_h,
            96.0,
            26.0,
            8.0,
        );

        egui::Area::new(egui::Id::new("sel_size_label"))
            .fixed_pos(pos)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(egui::Color32::from_rgba_unmultiplied(0, 0, 0, 180))
                    .corner_radius(egui::CornerRadius::same(4))
                    .inner_margin(egui::Margin {
                        left: 8,
                        right: 8,
                        top: 4,
                        bottom: 4,
                    })
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&text)
                                    .color(egui::Color32::WHITE)
                                    .size(14.0),
                            )
                            .wrap_mode(egui::TextWrapMode::Extend),
                        );
                    });
            });
    }

    fn draw_toolbar(
        &self,
        ctx: &egui::Context,
        local_right: f32,
        local_top: f32,
        local_bottom: f32,
        win_w: f32,
        win_h: f32,
    ) -> OverlayAction {
        const BTN: f32 = 32.0;
        const GAP: f32 = 4.0;
        const PAD: f32 = 4.0;
        const N: f32 = 4.0;

        let toolbar_w = PAD * 2.0 + N * BTN + (N - 1.0) * GAP;
        let toolbar_h = PAD * 2.0 + BTN;

        let pos = toolbar_pos(
            local_right,
            local_top,
            local_bottom,
            win_w,
            win_h,
            toolbar_w,
            toolbar_h,
            8.0,
        );

        let mut action = OverlayAction::None;

        egui::Area::new(egui::Id::new("sel_toolbar"))
            .fixed_pos(pos)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(egui::Color32::from_rgba_unmultiplied(30, 30, 30, 220))
                    .corner_radius(egui::CornerRadius::same(6))
                    .inner_margin(egui::Margin {
                        left: PAD as i8,
                        right: PAD as i8,
                        top: PAD as i8,
                        bottom: PAD as i8,
                    })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = GAP;
                            if icon_button(ui, egui_phosphor::regular::FLOPPY_DISK, BTN) {
                                action = OverlayAction::Save;
                            }
                            if icon_button(ui, egui_phosphor::regular::COPY, BTN) {
                                action = OverlayAction::Copy;
                            }
                            if icon_button(ui, egui_phosphor::regular::TEXT_T, BTN) {
                                action = OverlayAction::StartTextInsert;
                            }
                            if icon_button(ui, egui_phosphor::regular::X, BTN) {
                                action = OverlayAction::Exit;
                            }
                        });
                    });
            });

        action
    }
}

#[cfg(test)]
mod tests {
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

    /// Regression for the original bug: cursor exactly at the column junction
    /// (x = 1920) was owned by BOTH monitors because the old check used `<= win_gx1`.
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
}
