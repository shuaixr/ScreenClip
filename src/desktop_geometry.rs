#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesktopRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl DesktopRect {
    pub fn from_origin_size(origin: (i32, i32), size: (i32, i32)) -> Self {
        Self {
            x: origin.0,
            y: origin.1,
            width: size.0.max(0),
            height: size.1.max(0),
        }
    }

    pub fn right(self) -> i32 {
        self.x + self.width
    }

    pub fn bottom(self) -> i32 {
        self.y + self.height
    }

    pub fn contains_point(self, point: (i32, i32)) -> bool {
        point.0 >= self.x && point.0 < self.right() && point.1 >= self.y && point.1 < self.bottom()
    }

    pub fn intersects(self, other: Self) -> bool {
        self.x < other.right()
            && self.right() > other.x
            && self.y < other.bottom()
            && self.bottom() > other.y
    }
}

#[cfg(test)]
mod tests {
    use super::DesktopRect;

    #[test]
    fn seam_spanning_rect_intersects_both_viewports() {
        let left = DesktopRect::from_origin_size((0, 0), (1920, 1080));
        let right = DesktopRect::from_origin_size((1920, 0), (1920, 1080));
        let text = DesktopRect::from_origin_size((1880, 400), (120, 32));

        assert!(text.intersects(left));
        assert!(text.intersects(right));
    }

    #[test]
    fn exclusive_edges_prevent_duplicate_intersection() {
        let left = DesktopRect::from_origin_size((0, 0), (1920, 1080));
        let right = DesktopRect::from_origin_size((1920, 0), (1920, 1080));
        let text = DesktopRect::from_origin_size((1920, 400), (80, 32));

        assert!(!text.intersects(left));
        assert!(text.intersects(right));
    }

    #[test]
    fn viewport_contains_anchor_with_negative_origin() {
        let viewport = DesktopRect::from_origin_size((-1600, 0), (1600, 900));

        assert!(viewport.contains_point((-1, 120)));
        assert!(!viewport.contains_point((0, 120)));
    }
}
