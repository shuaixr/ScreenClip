use std::collections::HashMap;

use egui::epaint::{ClippedPrimitive, ColorImage, ImageData, Mesh, Primitive, TextureId};
use screenshots::image::{Rgba, RgbaImage};

use crate::{desktop_geometry::DesktopRect, selection_geometry};

pub struct CpuRenderer {
    width: u32,
    height: u32,
    frame: Vec<u32>,
    background_frame: Vec<u32>,
    background_key: Option<BackgroundKey>,
    textures: HashMap<TextureId, TextureData>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BackgroundKey {
    screenshot_size: (u32, u32),
    window_origin: (i32, i32),
    active_rect: Option<(i32, i32, i32, i32)>,
}

struct TextureData {
    width: usize,
    height: usize,
    pixels: Vec<egui::Color32>,
}

impl CpuRenderer {
    pub fn new(width: u32, height: u32) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        Self {
            width,
            height,
            frame: vec![0; (width * height) as usize],
            background_frame: vec![0; (width * height) as usize],
            background_key: None,
            textures: HashMap::new(),
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        if self.width == width && self.height == height {
            return;
        }

        self.width = width;
        self.height = height;
        let len = (self.width * self.height) as usize;
        self.frame.resize(len, 0);
        self.background_frame.resize(len, 0);
        self.background_key = None;
    }

    pub fn apply_textures_delta(&mut self, delta: &egui::TexturesDelta) {
        for (id, image_delta) in &delta.set {
            self.set_texture(*id, image_delta);
        }

        for id in &delta.free {
            self.textures.remove(id);
        }
    }

    pub fn render(
        &mut self,
        screenshot: &RgbaImage,
        window_origin: (i32, i32),
        active_rect: Option<(i32, i32, i32, i32)>,
        pixels_per_point: f32,
        paint_jobs: &[ClippedPrimitive],
        draw_scene: impl FnOnce(&mut Self),
    ) {
        self.draw_overlay_background(screenshot, window_origin, active_rect);
        draw_scene(self);

        for clipped in paint_jobs {
            if let Primitive::Mesh(mesh) = &clipped.primitive {
                self.draw_mesh(mesh, clipped.clip_rect, pixels_per_point);
            }
        }
    }

    pub fn frame(&self) -> &[u32] {
        &self.frame
    }

    pub fn frame_mut(&mut self) -> &mut [u32] {
        &mut self.frame
    }

    fn set_texture(&mut self, id: TextureId, delta: &egui::epaint::ImageDelta) {
        let source = match &delta.image {
            ImageData::Color(color) => image_from_color(color),
            ImageData::Font(font) => {
                let pixels = font.srgba_pixels(None).collect::<Vec<_>>();
                TextureData {
                    width: font.width(),
                    height: font.height(),
                    pixels,
                }
            }
        };

        if let Some([x, y]) = delta.pos {
            if let Some(existing) = self.textures.get_mut(&id) {
                blit_texture(existing, &source, x, y);
                return;
            }
        }

        self.textures.insert(id, source);
    }

    fn draw_overlay_background(
        &mut self,
        screenshot: &RgbaImage,
        window_origin: (i32, i32),
        active_rect: Option<(i32, i32, i32, i32)>,
    ) {
        let key = BackgroundKey {
            screenshot_size: (screenshot.width(), screenshot.height()),
            window_origin,
            active_rect,
        };

        if self.background_key != Some(key) {
            draw_overlay_background_into(
                &mut self.background_frame,
                self.width,
                self.height,
                screenshot,
                window_origin,
                active_rect,
            );
            self.background_key = Some(key);
        }

        self.frame.copy_from_slice(&self.background_frame);
    }

    fn draw_mesh(&mut self, mesh: &Mesh, clip_rect: egui::Rect, pixels_per_point: f32) {
        if mesh.indices.len() < 3 {
            return;
        }

        let scale = pixels_per_point.max(0.0001);
        let clip_min_x = (clip_rect.min.x * scale).floor().max(0.0) as i32;
        let clip_min_y = (clip_rect.min.y * scale).floor().max(0.0) as i32;
        let clip_max_x = (clip_rect.max.x * scale).ceil().min(self.width as f32) as i32;
        let clip_max_y = (clip_rect.max.y * scale).ceil().min(self.height as f32) as i32;

        if clip_min_x >= clip_max_x || clip_min_y >= clip_max_y {
            return;
        }

        let texture = self.textures.get(&mesh.texture_id);

        for tri in mesh.indices.chunks_exact(3) {
            let v0 = mesh.vertices[tri[0] as usize];
            let v1 = mesh.vertices[tri[1] as usize];
            let v2 = mesh.vertices[tri[2] as usize];

            let x0 = v0.pos.x * scale;
            let y0 = v0.pos.y * scale;
            let x1 = v1.pos.x * scale;
            let y1 = v1.pos.y * scale;
            let x2 = v2.pos.x * scale;
            let y2 = v2.pos.y * scale;

            let min_x = x0.min(x1).min(x2).floor().max(clip_min_x as f32).max(0.0) as i32;
            let max_x = x0
                .max(x1)
                .max(x2)
                .ceil()
                .min((clip_max_x - 1) as f32)
                .min((self.width.saturating_sub(1)) as f32) as i32;
            let min_y = y0.min(y1).min(y2).floor().max(clip_min_y as f32).max(0.0) as i32;
            let max_y = y0
                .max(y1)
                .max(y2)
                .ceil()
                .min((clip_max_y - 1) as f32)
                .min((self.height.saturating_sub(1)) as f32) as i32;

            if min_x > max_x || min_y > max_y {
                continue;
            }

            let area = edge(x0, y0, x1, y1, x2, y2);
            if area == 0.0 {
                continue;
            }

            for py in min_y..=max_y {
                for px in min_x..=max_x {
                    let fx = px as f32 + 0.5;
                    let fy = py as f32 + 0.5;

                    let w0 = edge(x1, y1, x2, y2, fx, fy) / area;
                    let w1 = edge(x2, y2, x0, y0, fx, fy) / area;
                    let w2 = 1.0 - w0 - w1;

                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }

                    let uv_x = v0.uv.x * w0 + v1.uv.x * w1 + v2.uv.x * w2;
                    let uv_y = v0.uv.y * w0 + v1.uv.y * w1 + v2.uv.y * w2;

                    let vc = mix_vertex_colors(v0.color, v1.color, v2.color, w0, w1, w2);
                    let tex = sample_texture(texture, uv_x, uv_y);
                    let src = modulate(vc, tex);

                    let idx = (py as u32 * self.width + px as u32) as usize;
                    self.frame[idx] = blend_over(self.frame[idx], src);
                }
            }
        }
    }
}

fn draw_overlay_background_into(
    frame: &mut [u32],
    width: u32,
    height: u32,
    screenshot: &RgbaImage,
    window_origin: (i32, i32),
    active_rect: Option<(i32, i32, i32, i32)>,
) {
    let rect = active_rect.map(|(x, y, w, h)| (x, y, x + w - 1, y + h - 1));
    let handle_rects = active_rect.map(selection_geometry::all_edge_handles);

    if screenshot.width() == width && screenshot.height() == height {
        draw_overlay_background_exact_size_into(
            frame,
            width,
            screenshot,
            window_origin,
            rect,
            handle_rects,
        );
        return;
    }

    for y in 0..height {
        for x in 0..width {
            let sx = x.min(screenshot.width().saturating_sub(1));
            let sy = y.min(screenshot.height().saturating_sub(1));
            let src = screenshot.get_pixel(sx, sy);

            let mut out = dim_pixel(*src);

            let gx = window_origin.0 + x as i32;
            let gy = window_origin.1 + y as i32;
            if let Some(pixel) =
                selection_overlay_pixel(src[0], src[1], src[2], gx, gy, rect, handle_rects.as_ref())
            {
                out = pixel;
            }

            frame[(y * width + x) as usize] = out;
        }
    }
}

fn draw_overlay_background_exact_size_into(
    frame: &mut [u32],
    width: u32,
    screenshot: &RgbaImage,
    window_origin: (i32, i32),
    rect: Option<(i32, i32, i32, i32)>,
    handle_rects: Option<[(selection_geometry::ResizeEdge, DesktopRect); 4]>,
) {
    let src = screenshot.as_raw();
    let Some((rx0, ry0, rx1, ry1)) = rect else {
        for (out, rgba) in frame.iter_mut().zip(src.chunks_exact(4)) {
            *out = dim_rgb(rgba[0], rgba[1], rgba[2]);
        }
        return;
    };

    for (idx, out) in frame.iter_mut().enumerate() {
        let src_idx = idx * 4;
        let mut pixel = dim_rgb(src[src_idx], src[src_idx + 1], src[src_idx + 2]);

        let x = (idx as u32 % width) as i32;
        let y = (idx as u32 / width) as i32;
        let gx = window_origin.0 + x;
        let gy = window_origin.1 + y;
        if let Some(overlay_pixel) = selection_overlay_pixel(
            src[src_idx],
            src[src_idx + 1],
            src[src_idx + 2],
            gx,
            gy,
            Some((rx0, ry0, rx1, ry1)),
            handle_rects.as_ref(),
        ) {
            pixel = overlay_pixel;
        }

        *out = pixel;
    }
}

fn selection_overlay_pixel(
    red: u8,
    green: u8,
    blue: u8,
    gx: i32,
    gy: i32,
    rect: Option<(i32, i32, i32, i32)>,
    handle_rects: Option<&[(selection_geometry::ResizeEdge, DesktopRect); 4]>,
) -> Option<u32> {
    if let Some(handles) = handle_rects {
        if handles
            .iter()
            .any(|(_, handle)| handle.contains_point((gx, gy)))
        {
            return Some(pack_rgb(240, 247, 255));
        }
    }

    let (rx0, ry0, rx1, ry1) = rect?;
    if gx < rx0 || gx > rx1 || gy < ry0 || gy > ry1 {
        return None;
    }

    let is_border = gx - rx0 <= 1 || rx1 - gx <= 1 || gy - ry0 <= 1 || ry1 - gy <= 1;
    Some(if is_border {
        pack_rgb(26, 179, 255)
    } else {
        pack_rgb(red, green, blue)
    })
}

fn image_from_color(color: &ColorImage) -> TextureData {
    TextureData {
        width: color.width(),
        height: color.height(),
        pixels: color.pixels.clone(),
    }
}

fn blit_texture(dst: &mut TextureData, src: &TextureData, x: usize, y: usize) {
    for row in 0..src.height {
        let dst_y = y + row;
        if dst_y >= dst.height {
            break;
        }

        for col in 0..src.width {
            let dst_x = x + col;
            if dst_x >= dst.width {
                break;
            }

            let dst_idx = dst_y * dst.width + dst_x;
            let src_idx = row * src.width + col;
            dst.pixels[dst_idx] = src.pixels[src_idx];
        }
    }
}

fn edge(ax: f32, ay: f32, bx: f32, by: f32, px: f32, py: f32) -> f32 {
    (px - ax) * (by - ay) - (py - ay) * (bx - ax)
}

fn sample_texture(texture: Option<&TextureData>, uv_x: f32, uv_y: f32) -> egui::Color32 {
    let Some(texture) = texture else {
        return egui::Color32::WHITE;
    };

    let tx = ((uv_x * texture.width as f32).floor() as i32).clamp(0, texture.width as i32 - 1);
    let ty = ((uv_y * texture.height as f32).floor() as i32).clamp(0, texture.height as i32 - 1);
    texture.pixels[ty as usize * texture.width + tx as usize]
}

fn mix_vertex_colors(
    c0: egui::Color32,
    c1: egui::Color32,
    c2: egui::Color32,
    w0: f32,
    w1: f32,
    w2: f32,
) -> egui::Color32 {
    let r = c0.r() as f32 * w0 + c1.r() as f32 * w1 + c2.r() as f32 * w2;
    let g = c0.g() as f32 * w0 + c1.g() as f32 * w1 + c2.g() as f32 * w2;
    let b = c0.b() as f32 * w0 + c1.b() as f32 * w1 + c2.b() as f32 * w2;
    let a = c0.a() as f32 * w0 + c1.a() as f32 * w1 + c2.a() as f32 * w2;
    egui::Color32::from_rgba_unmultiplied(
        r.round() as u8,
        g.round() as u8,
        b.round() as u8,
        a.round() as u8,
    )
}

fn modulate(vertex: egui::Color32, tex: egui::Color32) -> egui::Color32 {
    let r = (vertex.r() as u16 * tex.r() as u16 / 255) as u8;
    let g = (vertex.g() as u16 * tex.g() as u16 / 255) as u8;
    let b = (vertex.b() as u16 * tex.b() as u16 / 255) as u8;
    let a = (vertex.a() as u16 * tex.a() as u16 / 255) as u8;
    egui::Color32::from_rgba_unmultiplied(r, g, b, a)
}

fn blend_over(dst: u32, src: egui::Color32) -> u32 {
    let sa = src.a() as f32 / 255.0;
    if sa <= 0.0 {
        return dst;
    }

    let sr = src.r() as f32;
    let sg = src.g() as f32;
    let sb = src.b() as f32;

    let dr = ((dst >> 16) & 0xFF) as f32;
    let dg = ((dst >> 8) & 0xFF) as f32;
    let db = (dst & 0xFF) as f32;

    let out_r = sr * sa + dr * (1.0 - sa);
    let out_g = sg * sa + dg * (1.0 - sa);
    let out_b = sb * sa + db * (1.0 - sa);

    pack_rgb(
        out_r.round() as u8,
        out_g.round() as u8,
        out_b.round() as u8,
    )
}

fn dim_pixel(px: Rgba<u8>) -> u32 {
    dim_rgb(px[0], px[1], px[2])
}

fn dim_rgb(r: u8, g: u8, b: u8) -> u32 {
    let gray = 46.0;
    let r = r as f32 * 0.35 + gray * 0.65;
    let g = g as f32 * 0.35 + gray * 0.65;
    let b = b as f32 * 0.35 + gray * 0.65;
    pack_rgb(r.round() as u8, g.round() as u8, b.round() as u8)
}

fn pack_rgb(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 16) | ((g as u32) << 8) | b as u32
}
