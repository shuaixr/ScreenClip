use crate::desktop_geometry::DesktopRect;

pub const RESIZE_HANDLE_THICKNESS: i32 = 8;
pub const RESIZE_HANDLE_LENGTH: i32 = 48;
pub const RESIZE_HIT_SLOP: i32 = 6;
pub const MIN_SELECTION_SIZE: i32 = 1;

pub type SelectionRect = (i32, i32, i32, i32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeEdge {
    Top,
    Bottom,
    Left,
    Right,
}

pub fn selection_rect_from_points(
    start: Option<(i32, i32)>,
    current: Option<(i32, i32)>,
) -> Option<SelectionRect> {
    let (sx, sy) = start?;
    let (cx, cy) = current?;
    let x0 = sx.min(cx);
    let y0 = sy.min(cy);
    let x1 = sx.max(cx);
    let y1 = sy.max(cy);
    Some((x0, y0, x1 - x0 + 1, y1 - y0 + 1))
}

pub fn detect_resize_edge(rect: SelectionRect, point: (i32, i32)) -> Option<ResizeEdge> {
    [
        ResizeEdge::Top,
        ResizeEdge::Bottom,
        ResizeEdge::Left,
        ResizeEdge::Right,
    ]
    .into_iter()
    .find(|edge| expanded_handle_rect(rect, *edge, RESIZE_HIT_SLOP).contains_point(point))
}

pub fn resize_rect(rect: SelectionRect, edge: ResizeEdge, pointer: (i32, i32)) -> SelectionRect {
    let (x, y, width, height) = rect;
    let mut x0 = x;
    let mut y0 = y;
    let mut x1 = x + width.max(MIN_SELECTION_SIZE);
    let mut y1 = y + height.max(MIN_SELECTION_SIZE);

    match edge {
        ResizeEdge::Top => {
            y0 = pointer.1.min(y1 - MIN_SELECTION_SIZE);
        }
        ResizeEdge::Bottom => {
            y1 = (pointer.1 + 1).max(y0 + MIN_SELECTION_SIZE);
        }
        ResizeEdge::Left => {
            x0 = pointer.0.min(x1 - MIN_SELECTION_SIZE);
        }
        ResizeEdge::Right => {
            x1 = (pointer.0 + 1).max(x0 + MIN_SELECTION_SIZE);
        }
    }

    (x0, y0, x1 - x0, y1 - y0)
}

pub fn edge_handle_rect(rect: SelectionRect, edge: ResizeEdge) -> DesktopRect {
    let bounds = selection_bounds(rect);
    let horizontal_length = bounds.width.clamp(1, RESIZE_HANDLE_LENGTH);
    let vertical_length = bounds.height.clamp(1, RESIZE_HANDLE_LENGTH);
    let horizontal_x = bounds.x + (bounds.width - horizontal_length) / 2;
    let vertical_y = bounds.y + (bounds.height - vertical_length) / 2;
    let half_thickness = RESIZE_HANDLE_THICKNESS / 2;

    match edge {
        ResizeEdge::Top => DesktopRect::from_origin_size(
            (horizontal_x, bounds.y - half_thickness),
            (horizontal_length, RESIZE_HANDLE_THICKNESS),
        ),
        ResizeEdge::Bottom => DesktopRect::from_origin_size(
            (horizontal_x, bounds.bottom() - half_thickness),
            (horizontal_length, RESIZE_HANDLE_THICKNESS),
        ),
        ResizeEdge::Left => DesktopRect::from_origin_size(
            (bounds.x - half_thickness, vertical_y),
            (RESIZE_HANDLE_THICKNESS, vertical_length),
        ),
        ResizeEdge::Right => DesktopRect::from_origin_size(
            (bounds.right() - half_thickness, vertical_y),
            (RESIZE_HANDLE_THICKNESS, vertical_length),
        ),
    }
}

pub fn all_edge_handles(rect: SelectionRect) -> [(ResizeEdge, DesktopRect); 4] {
    [
        (ResizeEdge::Top, edge_handle_rect(rect, ResizeEdge::Top)),
        (
            ResizeEdge::Bottom,
            edge_handle_rect(rect, ResizeEdge::Bottom),
        ),
        (ResizeEdge::Left, edge_handle_rect(rect, ResizeEdge::Left)),
        (ResizeEdge::Right, edge_handle_rect(rect, ResizeEdge::Right)),
    ]
}

fn selection_bounds(rect: SelectionRect) -> DesktopRect {
    DesktopRect::from_origin_size((rect.0, rect.1), (rect.2.max(1), rect.3.max(1)))
}

fn expanded_handle_rect(rect: SelectionRect, edge: ResizeEdge, slop: i32) -> DesktopRect {
    let handle = edge_handle_rect(rect, edge);
    DesktopRect::from_origin_size(
        (handle.x - slop, handle.y - slop),
        (handle.width + slop * 2, handle.height + slop * 2),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        all_edge_handles, detect_resize_edge, edge_handle_rect, resize_rect,
        selection_rect_from_points, ResizeEdge,
    };

    #[test]
    fn selection_rect_normalizes_drag_points() {
        assert_eq!(
            selection_rect_from_points(Some((30, 40)), Some((10, 20))),
            Some((10, 20, 21, 21))
        );
    }

    #[test]
    fn detect_resize_edge_hits_each_midpoint_handle() {
        let rect = (100, 200, 80, 60);

        assert_eq!(detect_resize_edge(rect, (140, 200)), Some(ResizeEdge::Top));
        assert_eq!(
            detect_resize_edge(rect, (140, 259)),
            Some(ResizeEdge::Bottom)
        );
        assert_eq!(detect_resize_edge(rect, (100, 230)), Some(ResizeEdge::Left));
        assert_eq!(
            detect_resize_edge(rect, (179, 230)),
            Some(ResizeEdge::Right)
        );
        assert_eq!(detect_resize_edge(rect, (140, 230)), None);
    }

    #[test]
    fn top_handle_crossing_monitor_seam_still_hits() {
        let rect = (1880, 100, 80, 40);
        assert_eq!(detect_resize_edge(rect, (1920, 100)), Some(ResizeEdge::Top));
    }

    #[test]
    fn resize_rect_moves_only_target_edge() {
        let rect = (100, 200, 80, 60);

        assert_eq!(
            resize_rect(rect, ResizeEdge::Top, (140, 180)),
            (100, 180, 80, 80)
        );
        assert_eq!(
            resize_rect(rect, ResizeEdge::Bottom, (140, 279)),
            (100, 200, 80, 80)
        );
        assert_eq!(
            resize_rect(rect, ResizeEdge::Left, (80, 230)),
            (80, 200, 100, 60)
        );
        assert_eq!(
            resize_rect(rect, ResizeEdge::Right, (199, 230)),
            (100, 200, 100, 60)
        );
    }

    #[test]
    fn resize_rect_clamps_to_minimum_size() {
        let rect = (100, 200, 80, 60);

        assert_eq!(
            resize_rect(rect, ResizeEdge::Left, (250, 230)),
            (179, 200, 1, 60)
        );
        assert_eq!(
            resize_rect(rect, ResizeEdge::Top, (140, 400)),
            (100, 259, 80, 1)
        );
    }

    #[test]
    fn handle_rects_cover_all_four_edges() {
        let rect = (100, 200, 80, 60);
        let handles = all_edge_handles(rect);

        assert_eq!(handles.len(), 4);
        assert_eq!(edge_handle_rect(rect, ResizeEdge::Top).y, 196);
        assert_eq!(edge_handle_rect(rect, ResizeEdge::Right).x, 176);
    }
}
