use screenshots::image::RgbaImage;
use winit::dpi::PhysicalPosition;

use crate::desktop_geometry::DesktopRect;

pub const RECTANGLE_BORDER_THICKNESS: i32 = 3;
const RECTANGLE_BORDER_COLOR: u32 = 0x001AB3FF;

#[derive(Debug, Clone)]
pub struct RectangleAnnotation {
    pub global_rect: (i32, i32, i32, i32),
}

pub fn draw_preview(
    frame: &mut [u32],
    frame_size: (u32, u32),
    window_origin: PhysicalPosition<i32>,
    annotations: &[RectangleAnnotation],
    active_rect: Option<(i32, i32, i32, i32)>,
) {
    let window_rect = DesktopRect::from_origin_size(
        (window_origin.x, window_origin.y),
        (frame_size.0 as i32, frame_size.1 as i32),
    );
    let origin = (window_origin.x, window_origin.y);
    let mut surface = FrameSurface { frame, size: frame_size };

    for annotation in annotations {
        if rect_intersects_viewport(annotation.global_rect, window_rect) {
            stroke_into_surface(&mut surface, annotation.global_rect, origin);
        }
    }
    if let Some(active) = active_rect {
        if rect_intersects_viewport(active, window_rect) {
            stroke_into_surface(&mut surface, active, origin);
        }
    }
}

pub fn render_to_image(
    image: &mut RgbaImage,
    selection: (i32, i32, i32, i32),
    annotations: &[RectangleAnnotation],
) {
    let (sx, sy, sw, sh) = selection;
    let selection_rect = DesktopRect::from_origin_size((sx, sy), (sw, sh));
    let origin = (sx, sy);
    let mut surface = ImageSurface { image };

    for annotation in annotations {
        if rect_intersects_viewport(annotation.global_rect, selection_rect) {
            stroke_into_surface(&mut surface, annotation.global_rect, origin);
        }
    }
}

fn rect_intersects_viewport(rect: (i32, i32, i32, i32), viewport: DesktopRect) -> bool {
    let (rx, ry, rw, rh) = rect;
    if rw <= 0 || rh <= 0 {
        return false;
    }
    DesktopRect::from_origin_size((rx, ry), (rw, rh)).intersects(viewport)
}

fn stroke_into_surface<S: Surface>(
    surface: &mut S,
    global_rect: (i32, i32, i32, i32),
    origin: (i32, i32),
) {
    let (rx, ry, rw, rh) = global_rect;
    let c0 = rx - origin.0;
    let r0 = ry - origin.1;
    let c1 = c0 + rw;
    let r1 = r0 + rh;
    stroke_edges(surface, c0, r0, c1, r1);
}

fn stroke_edges<S: Surface>(surface: &mut S, c0: i32, r0: i32, c1: i32, r1: i32) {
    if c0 >= c1 || r0 >= r1 {
        return;
    }
    let t = RECTANGLE_BORDER_THICKNESS;
    let top_end = (r0 + t).min(r1);
    let bottom_start = (r1 - t).max(r0);
    let left_end = (c0 + t).min(c1);
    let right_start = (c1 - t).max(c0);

    if r0 < top_end {
        for ly in r0..top_end {
            for lx in c0..c1 {
                surface.write_pixel(lx, ly);
            }
        }
    }
    if bottom_start < r1 {
        for ly in bottom_start..r1 {
            for lx in c0..c1 {
                surface.write_pixel(lx, ly);
            }
        }
    }
    if c0 < left_end {
        for ly in r0..r1 {
            for lx in c0..left_end {
                surface.write_pixel(lx, ly);
            }
        }
    }
    if right_start < c1 {
        for ly in r0..r1 {
            for lx in right_start..c1 {
                surface.write_pixel(lx, ly);
            }
        }
    }
}

trait Surface {
    fn write_pixel(&mut self, lx: i32, ly: i32);
}

struct FrameSurface<'a> {
    frame: &'a mut [u32],
    size: (u32, u32),
}

impl<'a> Surface for FrameSurface<'a> {
    fn write_pixel(&mut self, lx: i32, ly: i32) {
        if lx < 0 || ly < 0 {
            return;
        }
        let ulx = lx as u32;
        let uly = ly as u32;
        if ulx >= self.size.0 || uly >= self.size.1 {
            return;
        }
        self.frame[(uly * self.size.0 + ulx) as usize] = RECTANGLE_BORDER_COLOR;
    }
}

struct ImageSurface<'a> {
    image: &'a mut RgbaImage,
}

impl<'a> Surface for ImageSurface<'a> {
    fn write_pixel(&mut self, lx: i32, ly: i32) {
        if lx < 0 || ly < 0 {
            return;
        }
        let ulx = lx as u32;
        let uly = ly as u32;
        if ulx >= self.image.width() || uly >= self.image.height() {
            return;
        }
        let pixel = self.image.get_pixel_mut(ulx, uly);
        pixel[0] = (RECTANGLE_BORDER_COLOR >> 16) as u8;
        pixel[1] = (RECTANGLE_BORDER_COLOR >> 8) as u8;
        pixel[2] = RECTANGLE_BORDER_COLOR as u8;
        pixel[3] = 0xFF;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use screenshots::image::Rgba;

    const BORDER: u32 = 0x001AB3FF;

    fn count_border_pixels(frame: &[u32]) -> usize {
        frame.iter().filter(|&&p| p == BORDER).count()
    }

    // 100x50 rectangle perimeter (3px thick):
    //   4 edge rectangles - 4 corner overlaps (3x3 each)
    fn expected_perimeter(w: i32, h: i32, t: i32) -> i32 {
        w * t + w * t + t * h + t * h - 4 * t * t
    }

    #[test]
    fn clips_to_window() {
        let mut frame_left = vec![0u32; 1920 * 1080];
        let mut frame_right = vec![0u32; 1920 * 1080];

        let annotation = RectangleAnnotation {
            global_rect: (1900, 100, 100, 50),
        };

        draw_preview(
            &mut frame_left,
            (1920, 1080),
            PhysicalPosition::new(0, 0),
            &[annotation.clone()],
            None,
        );
        draw_preview(
            &mut frame_right,
            (1920, 1080),
            PhysicalPosition::new(1920, 0),
            &[annotation],
            None,
        );

        let left = count_border_pixels(&frame_left);
        let right = count_border_pixels(&frame_right);

        assert!(left > 0, "left window should see the rectangle");
        assert!(right > 0, "right window should see the rectangle");
        assert_eq!(left + right, expected_perimeter(100, 50, 3) as usize);
    }

    #[test]
    fn active_rect_renders_in_both_windows() {
        let mut frame_left = vec![0u32; 1920 * 1080];
        let mut frame_right = vec![0u32; 1920 * 1080];

        let active = Some((1900, 100, 100, 50));

        draw_preview(
            &mut frame_left,
            (1920, 1080),
            PhysicalPosition::new(0, 0),
            &[],
            active,
        );
        draw_preview(
            &mut frame_right,
            (1920, 1080),
            PhysicalPosition::new(1920, 0),
            &[],
            active,
        );

        let left = count_border_pixels(&frame_left);
        let right = count_border_pixels(&frame_right);

        assert!(left > 0);
        assert!(right > 0);
        assert_eq!(left + right, expected_perimeter(100, 50, 3) as usize);
    }

    #[test]
    fn render_burns_only_inside_selection() {
        let mut image = RgbaImage::from_pixel(200, 200, Rgba([0, 0, 0, 0]));

        let inside = RectangleAnnotation { global_rect: (50, 50, 50, 50) };
        let outside = RectangleAnnotation { global_rect: (300, 300, 50, 50) };

        render_to_image(
            &mut image,
            (0, 0, 200, 200),
            &[inside, outside],
        );

        let blue_count = image
            .pixels()
            .filter(|p| p[0] == 0x1A && p[1] == 0xB3 && p[2] == 0xFF)
            .count();

        assert_eq!(blue_count, expected_perimeter(50, 50, 3) as usize);
    }
}
