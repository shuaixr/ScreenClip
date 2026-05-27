use egui;
use winit::dpi::PhysicalPosition;

pub use crate::overlay_ui_utils::{
    anchor_owns_window, size_label_anchor, size_label_pos, toolbar_anchor, toolbar_pos,
};

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

    /// Entry point: draws all overlay widgets for one window frame.
    ///
    /// - `show_toolbar`: pass `true` only after the selection is confirmed (drag finished).
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

        // Widget ownership is based on deterministic selection corners so a cross-monitor
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

        // Convert global pixel coords → window-local logical pixels.
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

    // ── size label ────────────────────────────────────────────────────────────

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

    // ── toolbar ───────────────────────────────────────────────────────────────

    /// Shows an action toolbar on the opposite side from the size label.
    ///
    fn draw_toolbar(
        &self,
        ctx: &egui::Context,
        local_right: f32,
        local_top: f32,
        local_bottom: f32,
        win_w: f32,
        win_h: f32,
    ) -> OverlayAction {
        const BTN: f32 = 32.0; // square side length (logical px)
        const GAP: f32 = 4.0; // gap between buttons
        const PAD: f32 = 4.0; // frame padding
        const N: f32 = 4.0; // number of buttons

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

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "overlay_ui_tests.rs"]
mod tests;
