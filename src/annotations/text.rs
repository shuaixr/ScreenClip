use std::{cell::RefCell, time::Duration};

use ab_glyph::{point, Font, FontArc, GlyphId, PxScale, ScaleFont};
use log::warn;
use screenshots::image::{Rgba, RgbaImage};
use winit::dpi::PhysicalPosition;

use crate::desktop_geometry::DesktopRect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TextPixelBounds {
    offset_x: i32,
    offset_y: i32,
    width: i32,
    height: i32,
}

#[derive(Debug, Clone)]
struct CachedTextBounds {
    text: String,
    line_height_bits: u32,
    bounds: TextPixelBounds,
}

fn px_scale_for_line_height(font: &FontArc, line_height_pixels: f32) -> PxScale {
    let base_height = font.as_scaled(PxScale::from(1.0)).height().max(0.0001);
    PxScale::from(line_height_pixels.max(1.0) / base_height)
}

fn measure_text_bounds(fonts: &[FontArc], line_height_pixels: f32, text: &str) -> TextPixelBounds {
    let mut caret_x = 0.0_f32;
    let mut line_top = 0.0_f32;
    let line_height = line_height_pixels.max(1.0);
    let mut previous = None;
    let mut min_x = 0.0_f32;
    let mut min_y = 0.0_f32;
    let mut max_x = 0.0_f32;
    let mut max_y = line_height;

    for ch in text.chars() {
        if ch == '\n' {
            caret_x = 0.0;
            line_top += line_height;
            max_y = max_y.max(line_top + line_height);
            previous = None;
            continue;
        }

        let Some((font_index, font)) = font_for_char(fonts, ch) else {
            continue;
        };
        let scale = px_scale_for_line_height(font, line_height_pixels);
        let scaled = font.as_scaled(scale);
        let glyph_id = scaled.glyph_id(ch);
        if let Some((prev_font_index, prev)) = previous {
            if prev_font_index == font_index {
                caret_x += scaled.kern(prev, glyph_id);
            }
        }

        let glyph =
            glyph_id.with_scale_and_position(scale, point(caret_x, line_top + scaled.ascent()));
        if let Some(outlined) = scaled.outline_glyph(glyph) {
            let bounds = outlined.px_bounds();
            min_x = min_x.min(bounds.min.x);
            min_y = min_y.min(bounds.min.y);
            max_x = max_x.max(bounds.max.x);
            max_y = max_y.max(bounds.max.y);
        }

        caret_x += scaled.h_advance(glyph_id);
        max_x = max_x.max(caret_x);
        previous = Some((font_index, glyph_id));
    }

    let offset_x = min_x.floor() as i32;
    let offset_y = min_y.floor() as i32;
    let width = (max_x - offset_x as f32).ceil().max(0.0) as i32;
    let height = (max_y - offset_y as f32).ceil().max(1.0) as i32;

    TextPixelBounds {
        offset_x,
        offset_y,
        width,
        height,
    }
}

fn measure_text_advance(fonts: &[FontArc], line_height_pixels: f32, text: &str) -> f32 {
    let mut advance = 0.0_f32;
    let mut previous = None;

    for ch in text.chars() {
        if ch == '\n' {
            break;
        }

        let Some((font_index, font)) = font_for_char(fonts, ch) else {
            continue;
        };
        let scale = px_scale_for_line_height(font, line_height_pixels);
        let scaled = font.as_scaled(scale);
        let glyph_id = scaled.glyph_id(ch);
        if let Some((prev_font_index, prev)) = previous {
            if prev_font_index == font_index {
                advance += scaled.kern(prev, glyph_id);
            }
        }

        advance += scaled.h_advance(glyph_id);
        previous = Some((font_index, glyph_id));
    }

    advance
}

fn font_for_char(fonts: &[FontArc], ch: char) -> Option<(usize, &FontArc)> {
    if ch.is_control() && ch != '\t' {
        return fonts.first().map(|font| (0, font));
    }

    fonts
        .iter()
        .enumerate()
        .find(|(_, font)| font.glyph_id(ch) != GlyphId(0))
        .or_else(|| fonts.iter().enumerate().next())
}

fn draw_text_line_image(
    image: &mut RgbaImage,
    color: Rgba<u8>,
    x: i32,
    y: i32,
    line_height_pixels: f32,
    fonts: &[FontArc],
    text: &str,
) {
    let mut sink = ImageSink { image };
    draw_text(&mut sink, color, x, y, line_height_pixels, fonts, text);
}

fn draw_text_line_frame(
    frame: &mut [u32],
    width: u32,
    height: u32,
    color: Rgba<u8>,
    x: i32,
    y: i32,
    line_height_pixels: f32,
    fonts: &[FontArc],
    text: &str,
) {
    let mut sink = FrameSink {
        frame,
        width,
        height,
    };
    draw_text(&mut sink, color, x, y, line_height_pixels, fonts, text);
}

trait TextSurface {
    fn blend_pixel(&mut self, x: i32, y: i32, color: Rgba<u8>, coverage: f32);
}

struct ImageSink<'a> {
    image: &'a mut RgbaImage,
}

impl TextSurface for ImageSink<'_> {
    fn blend_pixel(&mut self, x: i32, y: i32, color: Rgba<u8>, coverage: f32) {
        if x < 0 || y < 0 {
            return;
        }

        let ux = x as u32;
        let uy = y as u32;
        if ux >= self.image.width() || uy >= self.image.height() {
            return;
        }

        let src_a = (coverage.clamp(0.0, 1.0) * (color[3] as f32 / 255.0)).clamp(0.0, 1.0);
        if src_a <= 0.0 {
            return;
        }

        let dst = self.image.get_pixel_mut(ux, uy);
        let dr = dst[0] as f32;
        let dg = dst[1] as f32;
        let db = dst[2] as f32;
        let da = dst[3] as f32 / 255.0;

        let out_a = src_a + da * (1.0 - src_a);
        let out_r = color[0] as f32 * src_a + dr * (1.0 - src_a);
        let out_g = color[1] as f32 * src_a + dg * (1.0 - src_a);
        let out_b = color[2] as f32 * src_a + db * (1.0 - src_a);

        *dst = Rgba([
            out_r.round().clamp(0.0, 255.0) as u8,
            out_g.round().clamp(0.0, 255.0) as u8,
            out_b.round().clamp(0.0, 255.0) as u8,
            (out_a * 255.0).round().clamp(0.0, 255.0) as u8,
        ]);
    }
}

struct FrameSink<'a> {
    frame: &'a mut [u32],
    width: u32,
    height: u32,
}

impl TextSurface for FrameSink<'_> {
    fn blend_pixel(&mut self, x: i32, y: i32, color: Rgba<u8>, coverage: f32) {
        if x < 0 || y < 0 {
            return;
        }

        let ux = x as u32;
        let uy = y as u32;
        if ux >= self.width || uy >= self.height {
            return;
        }

        let src_a = (coverage.clamp(0.0, 1.0) * (color[3] as f32 / 255.0)).clamp(0.0, 1.0);
        if src_a <= 0.0 {
            return;
        }

        let idx = (uy * self.width + ux) as usize;
        let dst = self.frame[idx];
        let dr = ((dst >> 16) & 0xFF) as f32;
        let dg = ((dst >> 8) & 0xFF) as f32;
        let db = (dst & 0xFF) as f32;

        let out_r = color[0] as f32 * src_a + dr * (1.0 - src_a);
        let out_g = color[1] as f32 * src_a + dg * (1.0 - src_a);
        let out_b = color[2] as f32 * src_a + db * (1.0 - src_a);

        self.frame[idx] =
            ((out_r.round() as u32) << 16) | ((out_g.round() as u32) << 8) | out_b.round() as u32;
    }
}

fn draw_text<S: TextSurface>(
    surface: &mut S,
    color: Rgba<u8>,
    x: i32,
    y: i32,
    line_height_pixels: f32,
    fonts: &[FontArc],
    text: &str,
) {
    let mut caret_x = x as f32;
    let mut line_top = y as f32;
    let line_height = line_height_pixels.max(1.0);
    let mut previous = None;

    for ch in text.chars() {
        if ch == '\n' {
            caret_x = x as f32;
            line_top += line_height;
            previous = None;
            continue;
        }

        let Some((font_index, font)) = font_for_char(fonts, ch) else {
            continue;
        };
        let scale = px_scale_for_line_height(font, line_height_pixels);
        let scaled = font.as_scaled(scale);
        let glyph_id = scaled.glyph_id(ch);
        if let Some((prev_font_index, prev)) = previous {
            if prev_font_index == font_index {
                caret_x += scaled.kern(prev, glyph_id);
            }
        }

        let glyph =
            glyph_id.with_scale_and_position(scale, point(caret_x, line_top + scaled.ascent()));
        if let Some(outlined) = scaled.outline_glyph(glyph) {
            let bounds = outlined.px_bounds();
            let min_x = bounds.min.x.floor() as i32;
            let min_y = bounds.min.y.floor() as i32;
            outlined.draw(|gx, gy, coverage| {
                surface.blend_pixel(min_x + gx as i32, min_y + gy as i32, color, coverage);
            });
        }

        caret_x += scaled.h_advance(glyph_id);
        previous = Some((font_index, glyph_id));
    }
}

#[derive(Debug, Clone)]
pub struct TextAnnotation {
    pub id: u64,
    pub global_pos: (i32, i32),
    pub line_height_pixels: f32,
    pub text: String,
    caret_char_index: usize,
    selection_anchor_char_index: usize,
    cached_bounds: RefCell<Option<CachedTextBounds>>,
}

impl TextAnnotation {
    pub fn new(id: u64, global_pos: (i32, i32), line_height_pixels: f32) -> Self {
        Self {
            id,
            global_pos,
            line_height_pixels,
            text: String::new(),
            caret_char_index: 0,
            selection_anchor_char_index: 0,
            cached_bounds: RefCell::new(None),
        }
    }
}

pub struct TextAnnotationDrag {
    pub id: u64,
    pub pointer_offset: (i32, i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAnnotationCursorHit {
    Border,
    Text,
}

pub const TEXT_LINE_HEIGHT_PIXELS: f32 = 28.0;
pub const TEXT_CARET_BLINK_INTERVAL: Duration = Duration::from_millis(500);

const TEXT_INPUT_HOST_WIDTH_POINTS: f32 = 2.0;
const TEXT_PLACEHOLDER: &str = "Type text...";
const TEXT_CARET_BLINK_PERIOD_MS: u128 = 1_000;
const TEXT_BORDER_PADDING_PIXELS: i32 = 4;
const TEXT_BORDER_THICKNESS_PIXELS: i32 = 3;

pub fn draw_input_hosts(
    ctx: &egui::Context,
    window_origin: PhysicalPosition<i32>,
    window_size: (u32, u32),
    pixels_per_point: f32,
    annotations: &mut [TextAnnotation],
    pending_focus_id: &mut Option<u64>,
    text_fonts: &[FontArc],
) -> Option<u64> {
    let window_rect = DesktopRect::from_origin_size(
        (window_origin.x, window_origin.y),
        (window_size.0 as i32, window_size.1 as i32),
    );
    let mut focused_annotation_id = None;

    for annotation in annotations.iter_mut() {
        if !window_rect.contains_point(annotation.global_pos) {
            continue;
        }

        let local_x = (annotation.global_pos.0 - window_origin.x) as f32 / pixels_per_point;
        let local_y = (annotation.global_pos.1 - window_origin.y) as f32 / pixels_per_point;
        let id = annotation.id;
        let font_size_points =
            preview_font_size_points(ctx, annotation.line_height_pixels, pixels_per_point);
        let font_id = egui::FontId::proportional(font_size_points);
        let row_height = ctx.fonts(|fonts| fonts.row_height(&font_id));
        let host_size =
            text_input_host_size(ctx, annotation, text_fonts, &font_id, pixels_per_point);
        let desired_rows = annotation_line_count(annotation).max(1);

        egui::Area::new(egui::Id::new(("text_annotation", id)))
            .fixed_pos(egui::pos2(local_x, local_y))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let mut style = ui.style().as_ref().clone();
                style.visuals.extreme_bg_color = egui::Color32::TRANSPARENT;
                style.visuals.code_bg_color = egui::Color32::TRANSPARENT;
                style.visuals.widgets.noninteractive.bg_fill = egui::Color32::TRANSPARENT;
                style.visuals.widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
                style.visuals.widgets.hovered.bg_fill = egui::Color32::TRANSPARENT;
                style.visuals.widgets.active.bg_fill = egui::Color32::TRANSPARENT;
                style.visuals.widgets.open.bg_fill = egui::Color32::TRANSPARENT;
                style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
                style.visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
                style.visuals.widgets.hovered.bg_stroke = egui::Stroke::NONE;
                style.visuals.widgets.active.bg_stroke = egui::Stroke::NONE;
                style.visuals.widgets.open.bg_stroke = egui::Stroke::NONE;
                style.visuals.selection.bg_fill = egui::Color32::TRANSPARENT;
                style.visuals.selection.stroke = egui::Stroke::NONE;
                style.visuals.text_cursor.stroke = egui::Stroke::NONE;

                let output = ui
                    .scope(|ui| {
                        ui.set_style(style);
                        ui.set_min_width(host_size.x);
                        ui.set_max_width(host_size.x);
                        ui.allocate_ui_with_layout(
                            egui::vec2(host_size.x, host_size.y.max(row_height)),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                egui::TextEdit::multiline(&mut annotation.text)
                                    .id_source(("text_input", id))
                                    .desired_width(host_size.x)
                                    .desired_rows(desired_rows)
                                    .clip_text(false)
                                    .font(font_id)
                                    .text_color(egui::Color32::TRANSPARENT)
                                    .frame(false)
                                    .show(ui)
                            },
                        )
                        .inner
                    })
                    .inner;
                let response = output.response.clone();
                let wants_initial_focus = pending_focus_id
                    .as_ref()
                    .is_some_and(|focus_id| *focus_id == id);
                if pending_focus_id
                    .as_ref()
                    .is_some_and(|focus_id| *focus_id == id)
                {
                    response.request_focus();
                    *pending_focus_id = None;
                }
                if wants_initial_focus || response.has_focus() {
                    focused_annotation_id = Some(id);
                }
                if response.has_focus() {
                    if let Some(cursor_range) = output.cursor_range {
                        let cursor_range = cursor_range.as_ccursor_range();
                        let char_count = annotation.text.chars().count();
                        annotation.caret_char_index = cursor_range.primary.index.min(char_count);
                        annotation.selection_anchor_char_index =
                            cursor_range.secondary.index.min(char_count);
                    }
                }
            });
    }

    focused_annotation_id
}

pub fn draw_preview(
    frame: &mut [u32],
    frame_size: (u32, u32),
    window_origin: PhysicalPosition<i32>,
    annotations: &[TextAnnotation],
    text_fonts: &[FontArc],
    focused_annotation_id: Option<u64>,
    color: Rgba<u8>,
) {
    if text_fonts.is_empty() {
        return;
    }

    let window_rect = DesktopRect::from_origin_size(
        (window_origin.x, window_origin.y),
        (frame_size.0 as i32, frame_size.1 as i32),
    );

    for annotation in annotations {
        let is_focused = focused_annotation_id == Some(annotation.id);
        let (text, color) = if annotation.text.is_empty() {
            if is_focused {
                (TEXT_PLACEHOLDER, Rgba([180, 180, 180, 255]))
            } else {
                continue;
            }
        } else {
            (annotation.text.as_str(), color)
        };

        let text_bounds = measure_text_bounds(text_fonts, annotation.line_height_pixels, text);
        let annotation_rect = annotation_visual_rect(annotation, text_bounds);
        let border_rect = text_annotation_box_rect(annotation, text_bounds);
        if !annotation_rect.intersects(window_rect) && !border_rect.intersects(window_rect) {
            continue;
        }

        if is_focused {
            draw_text_annotation_border(frame, frame_size, window_origin, window_rect, border_rect);
        }

        if is_focused && !annotation.text.is_empty() {
            let (selection_start, selection_end) = sorted_selection_range(annotation);
            if selection_start < selection_end {
                draw_text_selection_range(
                    frame,
                    frame_size,
                    window_origin,
                    window_rect,
                    annotation,
                    text_fonts,
                    selection_start,
                    selection_end,
                );
            }
        }

        draw_text_line_frame(
            frame,
            frame_size.0,
            frame_size.1,
            color,
            annotation.global_pos.0 - window_origin.x,
            annotation.global_pos.1 - window_origin.y,
            annotation.line_height_pixels,
            text_fonts,
            text,
        );

        if is_focused && text_caret_visible_now() {
            let caret = text_position_to_char(
                text_fonts,
                annotation.line_height_pixels,
                &annotation.text,
                annotation.caret_char_index,
            );
            draw_text_caret(
                frame,
                frame_size,
                window_origin,
                window_rect,
                annotation.global_pos.0 + caret.x.round() as i32,
                annotation.global_pos.1 + caret.y.round() as i32,
                annotation.line_height_pixels.round() as i32,
            );
        }
    }
}

pub fn render_to_image(
    image: &mut RgbaImage,
    rect: (i32, i32, i32, i32),
    annotations: &[TextAnnotation],
    text_fonts: &[FontArc],
    color: Rgba<u8>,
) -> Result<(), String> {
    if text_fonts.is_empty() {
        if !annotations.is_empty() {
            warn!("text font unavailable; skipping text rendering in exported image");
        }
        return Ok(());
    }

    let (rx, ry, rw, rh) = rect;
    let export_rect = DesktopRect::from_origin_size((rx, ry), (rw, rh));

    for annotation in annotations {
        if annotation.text.trim().is_empty() {
            continue;
        }
        let annotation_rect = annotation_content_rect(annotation, text_fonts);
        if !annotation_rect.intersects(export_rect) {
            continue;
        }

        let local_x = annotation.global_pos.0 - rx;
        let local_y = annotation.global_pos.1 - ry;
        draw_text_line_image(
            image,
            color,
            local_x,
            local_y,
            annotation.line_height_pixels,
            text_fonts,
            &annotation.text,
        );
    }

    Ok(())
}

pub fn border_hit_test(
    annotations: &[TextAnnotation],
    text_fonts: &[FontArc],
    focused_id: u64,
    point: (i32, i32),
) -> bool {
    cursor_hit_test(annotations, text_fonts, Some(focused_id), point)
        == Some(TextAnnotationCursorHit::Border)
}

pub fn cursor_hit_test(
    annotations: &[TextAnnotation],
    text_fonts: &[FontArc],
    focused_id: Option<u64>,
    point: (i32, i32),
) -> Option<TextAnnotationCursorHit> {
    if text_fonts.is_empty() {
        return None;
    }

    let focused_annotation =
        focused_id.and_then(|id| annotations.iter().find(|annotation| annotation.id == id));
    if let Some(annotation) = focused_annotation {
        let outer = text_annotation_box_rect(annotation, annotation_bounds(annotation, text_fonts));
        if outer.contains_point(point) {
            let inner = DesktopRect::from_origin_size(
                (
                    outer.x + TEXT_BORDER_THICKNESS_PIXELS,
                    outer.y + TEXT_BORDER_THICKNESS_PIXELS,
                ),
                (
                    outer.width - TEXT_BORDER_THICKNESS_PIXELS * 2,
                    outer.height - TEXT_BORDER_THICKNESS_PIXELS * 2,
                ),
            );
            return if inner.contains_point(point) {
                Some(TextAnnotationCursorHit::Text)
            } else {
                Some(TextAnnotationCursorHit::Border)
            };
        }
    }

    annotations
        .iter()
        .filter(|annotation| focused_id != Some(annotation.id))
        .find_map(|annotation| {
            let text_rect =
                annotation_visual_rect(annotation, annotation_bounds(annotation, text_fonts));
            text_rect
                .contains_point(point)
                .then_some(TextAnnotationCursorHit::Text)
        })
}

fn annotation_bounds(annotation: &TextAnnotation, fonts: &[FontArc]) -> TextPixelBounds {
    let text = if annotation.text.is_empty() {
        TEXT_PLACEHOLDER
    } else {
        &annotation.text
    };

    cached_text_bounds(annotation, fonts, text)
}

fn preview_font_size_points(
    ctx: &egui::Context,
    line_height_pixels: f32,
    pixels_per_point: f32,
) -> f32 {
    let logical_height = line_height_pixels.max(1.0) / pixels_per_point.max(0.0001);
    let base_row_height = ctx.fonts(|fonts| {
        fonts
            .row_height(&egui::FontId::proportional(1.0))
            .max(0.0001)
    });
    logical_height / base_row_height
}

fn annotation_content_rect(annotation: &TextAnnotation, fonts: &[FontArc]) -> DesktopRect {
    let bounds = cached_text_bounds(annotation, fonts, &annotation.text);
    DesktopRect::from_origin_size(
        (
            annotation.global_pos.0 + bounds.offset_x,
            annotation.global_pos.1 + bounds.offset_y,
        ),
        (bounds.width, bounds.height),
    )
}

fn text_input_host_size(
    ctx: &egui::Context,
    annotation: &TextAnnotation,
    text_fonts: &[FontArc],
    font_id: &egui::FontId,
    pixels_per_point: f32,
) -> egui::Vec2 {
    let text = if annotation.text.is_empty() {
        TEXT_PLACEHOLDER
    } else {
        &annotation.text
    };

    let bounds = if !text_fonts.is_empty() {
        Some(cached_text_bounds(annotation, text_fonts, text))
    } else {
        None
    };

    let width_pixels = if let Some(bounds) = bounds {
        bounds.width as f32
    } else {
        text.lines()
            .map(|line| {
                ctx.fonts(|fonts| {
                    fonts
                        .layout_no_wrap(line.to_owned(), font_id.clone(), egui::Color32::WHITE)
                        .size()
                        .x
                        * pixels_per_point
                })
            })
            .fold(0.0_f32, f32::max)
    };

    let height_pixels = bounds
        .map(|bounds| bounds.height as f32)
        .unwrap_or_else(|| {
            annotation.line_height_pixels.max(1.0) * annotation_line_count(annotation) as f32
        });

    egui::vec2(
        (width_pixels / pixels_per_point + 24.0).max(TEXT_INPUT_HOST_WIDTH_POINTS),
        (height_pixels / pixels_per_point + 8.0).max(1.0),
    )
}

fn annotation_line_count(annotation: &TextAnnotation) -> usize {
    annotation.text.chars().filter(|ch| *ch == '\n').count() + 1
}

fn sorted_selection_range(annotation: &TextAnnotation) -> (usize, usize) {
    let char_count = annotation.text.chars().count();
    let caret = annotation.caret_char_index.min(char_count);
    let anchor = annotation.selection_anchor_char_index.min(char_count);
    (caret.min(anchor), caret.max(anchor))
}

#[derive(Debug, Clone, Copy)]
struct TextPosition {
    x: f32,
    y: f32,
}

#[derive(Debug, Clone, Copy)]
struct TextLineSegment<'a> {
    start_char: usize,
    end_char: usize,
    line_index: usize,
    text: &'a str,
}

fn text_position_to_char(
    fonts: &[FontArc],
    line_height_pixels: f32,
    text: &str,
    char_index: usize,
) -> TextPosition {
    let char_count = text.chars().count();
    let char_index = char_index.min(char_count);
    let line_height = line_height_pixels.max(1.0);

    for segment in text_line_segments(text) {
        if char_index <= segment.end_char {
            let local_index = char_index
                .saturating_sub(segment.start_char)
                .min(segment.end_char.saturating_sub(segment.start_char));
            return TextPosition {
                x: measure_text_advance(
                    fonts,
                    line_height_pixels,
                    text_prefix_by_chars(segment.text, local_index),
                ),
                y: segment.line_index as f32 * line_height,
            };
        }
    }

    TextPosition { x: 0.0, y: 0.0 }
}

fn text_line_segments(text: &str) -> Vec<TextLineSegment<'_>> {
    let mut segments = Vec::new();
    let mut start_char = 0_usize;
    let mut start_byte = 0_usize;
    let mut char_index = 0_usize;
    let mut line_index = 0_usize;

    for (byte_index, ch) in text.char_indices() {
        if ch == '\n' {
            segments.push(TextLineSegment {
                start_char,
                end_char: char_index,
                line_index,
                text: &text[start_byte..byte_index],
            });
            char_index += 1;
            start_char = char_index;
            start_byte = byte_index + ch.len_utf8();
            line_index += 1;
        } else {
            char_index += 1;
        }
    }

    segments.push(TextLineSegment {
        start_char,
        end_char: char_index,
        line_index,
        text: &text[start_byte..],
    });

    segments
}

fn text_prefix_by_chars(text: &str, char_index: usize) -> &str {
    if char_index == 0 {
        return "";
    }

    match text.char_indices().nth(char_index) {
        Some((byte_index, _)) => &text[..byte_index],
        None => text,
    }
}

fn text_caret_visible_now() -> bool {
    let Ok(elapsed) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else {
        return true;
    };

    elapsed.as_millis() % TEXT_CARET_BLINK_PERIOD_MS < TEXT_CARET_BLINK_PERIOD_MS / 2
}

fn annotation_visual_rect(annotation: &TextAnnotation, bounds: TextPixelBounds) -> DesktopRect {
    DesktopRect::from_origin_size(
        (
            annotation.global_pos.0 + bounds.offset_x,
            annotation.global_pos.1 + bounds.offset_y,
        ),
        (bounds.width, bounds.height),
    )
}

fn text_annotation_box_rect(annotation: &TextAnnotation, bounds: TextPixelBounds) -> DesktopRect {
    DesktopRect::from_origin_size(
        (
            annotation.global_pos.0 - TEXT_BORDER_PADDING_PIXELS,
            annotation.global_pos.1 - TEXT_BORDER_PADDING_PIXELS,
        ),
        (
            bounds.width + TEXT_BORDER_PADDING_PIXELS * 2,
            bounds.height + TEXT_BORDER_PADDING_PIXELS * 2,
        ),
    )
}

fn cached_text_bounds(
    annotation: &TextAnnotation,
    fonts: &[FontArc],
    text: &str,
) -> TextPixelBounds {
    let line_height_bits = annotation.line_height_pixels.to_bits();

    if let Some(cache) = annotation.cached_bounds.borrow().as_ref() {
        if cache.line_height_bits == line_height_bits && cache.text == text {
            return cache.bounds;
        }
    }

    let bounds = measure_text_bounds(fonts, annotation.line_height_pixels, text);
    *annotation.cached_bounds.borrow_mut() = Some(CachedTextBounds {
        text: text.to_owned(),
        line_height_bits,
        bounds,
    });
    bounds
}

fn draw_text_selection_range(
    frame: &mut [u32],
    frame_size: (u32, u32),
    window_origin: PhysicalPosition<i32>,
    window_rect: DesktopRect,
    annotation: &TextAnnotation,
    fonts: &[FontArc],
    selection_start: usize,
    selection_end: usize,
) {
    let line_height = annotation.line_height_pixels.max(1.0);
    let selection_height = line_height.round() as i32;

    for segment in text_line_segments(&annotation.text) {
        let start = selection_start.max(segment.start_char);
        let end = selection_end.min(segment.end_char);
        if start >= end {
            continue;
        }

        let start_local = start - segment.start_char;
        let end_local = end - segment.start_char;
        let selection_x0 = annotation.global_pos.0
            + measure_text_advance(
                fonts,
                annotation.line_height_pixels,
                text_prefix_by_chars(segment.text, start_local),
            )
            .round() as i32;
        let selection_x1 = annotation.global_pos.0
            + measure_text_advance(
                fonts,
                annotation.line_height_pixels,
                text_prefix_by_chars(segment.text, end_local),
            )
            .round() as i32;
        let selection_y =
            annotation.global_pos.1 + (segment.line_index as f32 * line_height).round() as i32;

        draw_text_selection(
            frame,
            frame_size,
            window_origin,
            window_rect,
            selection_x0,
            selection_x1,
            selection_y,
            selection_height,
        );
    }
}

fn draw_text_annotation_border(
    frame: &mut [u32],
    frame_size: (u32, u32),
    window_origin: PhysicalPosition<i32>,
    window_rect: DesktopRect,
    border_rect: DesktopRect,
) {
    let left = border_rect.x.max(window_rect.x);
    let right = border_rect.right().min(window_rect.right());
    let top = border_rect.y.max(window_rect.y);
    let bottom = border_rect.bottom().min(window_rect.bottom());
    if left >= right || top >= bottom {
        return;
    }

    for global_y in top..bottom {
        for global_x in left..right {
            let on_border = global_x - border_rect.x < TEXT_BORDER_THICKNESS_PIXELS
                || border_rect.right() - global_x <= TEXT_BORDER_THICKNESS_PIXELS
                || global_y - border_rect.y < TEXT_BORDER_THICKNESS_PIXELS
                || border_rect.bottom() - global_y <= TEXT_BORDER_THICKNESS_PIXELS;
            if !on_border {
                continue;
            }

            let local_x = global_x - window_origin.x;
            let local_y = global_y - window_origin.y;
            if local_x < 0
                || local_x >= frame_size.0 as i32
                || local_y < 0
                || local_y >= frame_size.1 as i32
            {
                continue;
            }

            let idx = (local_y as u32 * frame_size.0 + local_x as u32) as usize;
            frame[idx] = 0x28A8FF;
        }
    }
}

fn draw_text_caret(
    frame: &mut [u32],
    frame_size: (u32, u32),
    window_origin: PhysicalPosition<i32>,
    window_rect: DesktopRect,
    caret_x: i32,
    caret_y: i32,
    caret_height: i32,
) {
    if caret_height <= 0 || !window_rect.contains_point((caret_x, caret_y)) {
        return;
    }

    let local_x = caret_x - window_origin.x;
    let local_top = caret_y - window_origin.y;
    let local_bottom = (caret_y + caret_height).min(window_rect.bottom()) - window_origin.y;
    if local_x < 0 || local_x >= frame_size.0 as i32 {
        return;
    }

    for local_y in local_top.max(0)..local_bottom.max(0) {
        if local_y >= frame_size.1 as i32 {
            break;
        }
        let idx = (local_y as u32 * frame_size.0 + local_x as u32) as usize;
        frame[idx] = 0xFFFFFF;
    }
}

fn draw_text_selection(
    frame: &mut [u32],
    frame_size: (u32, u32),
    window_origin: PhysicalPosition<i32>,
    window_rect: DesktopRect,
    selection_x0: i32,
    selection_x1: i32,
    selection_y: i32,
    selection_height: i32,
) {
    if selection_height <= 0 || selection_x0 == selection_x1 {
        return;
    }

    let left = selection_x0.min(selection_x1).max(window_rect.x);
    let right = selection_x0.max(selection_x1).min(window_rect.right());
    let top = selection_y.max(window_rect.y);
    let bottom = (selection_y + selection_height).min(window_rect.bottom());
    if left >= right || top >= bottom {
        return;
    }

    for global_y in top..bottom {
        let local_y = global_y - window_origin.y;
        if local_y < 0 || local_y >= frame_size.1 as i32 {
            continue;
        }

        for global_x in left..right {
            let local_x = global_x - window_origin.x;
            if local_x < 0 || local_x >= frame_size.0 as i32 {
                continue;
            }

            let idx = (local_y as u32 * frame_size.0 + local_x as u32) as usize;
            frame[idx] = blend_selection_pixel(frame[idx]);
        }
    }
}

fn blend_selection_pixel(dst: u32) -> u32 {
    let alpha = 0.55_f32;
    let sr = 40.0_f32;
    let sg = 120.0_f32;
    let sb = 255.0_f32;
    let dr = ((dst >> 16) & 0xFF) as f32;
    let dg = ((dst >> 8) & 0xFF) as f32;
    let db = (dst & 0xFF) as f32;

    let r = sr * alpha + dr * (1.0 - alpha);
    let g = sg * alpha + dg * (1.0 - alpha);
    let b = sb * alpha + db * (1.0 - alpha);
    ((r.round() as u32) << 16) | ((g.round() as u32) << 8) | b.round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_fonts() -> Vec<FontArc> {
        let bytes = load_text_font_bytes_for_tests();
        bytes
            .into_iter()
            .filter_map(|b| FontArc::try_from_vec(b).ok())
            .collect()
    }

    fn load_text_font_bytes_for_tests() -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        for path in [
            r"C:\Windows\Fonts\segoeui.ttf",
            r"C:\Windows\Fonts\arial.ttf",
            r"C:\Windows\Fonts\consola.ttf",
        ] {
            if let Ok(bytes) = std::fs::read(path) {
                out.push(bytes);
            }
        }
        out
    }

    #[test]
    fn custom_text_color_is_respected() {
        let fonts = dummy_fonts();
        if fonts.is_empty() {
            // No fonts available on this host — nothing to test.
            return;
        }

        let mut image = RgbaImage::from_pixel(200, 200, Rgba([0, 0, 0, 0]));
        let mut annotation = TextAnnotation::new(0, (10, 10), TEXT_LINE_HEIGHT_PIXELS);
        annotation.text = "X".to_string();
        let red = Rgba([255, 0, 0, 255]);

        render_to_image(
            &mut image,
            (0, 0, 200, 200),
            &[annotation],
            &fonts,
            red,
        )
        .unwrap();

        let red_count = image
            .pixels()
            .filter(|p| p[0] > 200 && p[1] < 50 && p[2] < 50)
            .count();
        assert!(red_count > 0, "expected at least one red pixel from custom text color");
    }
}
