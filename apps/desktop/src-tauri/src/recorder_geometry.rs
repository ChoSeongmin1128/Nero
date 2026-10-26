#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    fn right(self) -> i64 {
        i64::from(self.x) + i64::from(self.width)
    }

    fn bottom(self) -> i64 {
        i64::from(self.y) + i64::from(self.height)
    }

    fn contains(self, window: Rect) -> bool {
        self.width > 0
            && self.height > 0
            && i64::from(window.x) >= i64::from(self.x)
            && i64::from(window.y) >= i64::from(self.y)
            && window.right() <= self.right()
            && window.bottom() <= self.bottom()
    }

    fn overlap(self, window: Rect) -> u64 {
        let width =
            (self.right().min(window.right()) - i64::from(self.x).max(i64::from(window.x))).max(0);
        let height = (self.bottom().min(window.bottom())
            - i64::from(self.y).max(i64::from(window.y)))
        .max(0);
        (width as u64) * (height as u64)
    }
}

fn coordinate(position: i32, size: u32, origin: i32, available: u32) -> i32 {
    if size > available {
        return origin;
    }
    i64::from(position).clamp(
        i64::from(origin),
        i64::from(origin) + i64::from(available) - i64::from(size),
    ) as i32
}

pub fn recover_position(window: Rect, work_areas: &[Rect], preferred: usize) -> Option<(i32, i32)> {
    if window.width == 0
        || window.height == 0
        || work_areas.iter().any(|area| area.contains(window))
    {
        return None;
    }
    let valid = |area: &Rect| area.width > 0 && area.height > 0;
    let fallback = work_areas
        .get(preferred)
        .filter(|area| valid(area))
        .or_else(|| work_areas.iter().find(|area| valid(area)))?;
    let mut selected = fallback;
    let mut largest = fallback.overlap(window);
    for area in work_areas.iter().filter(|area| valid(area)) {
        let overlap = area.overlap(window);
        if overlap > largest {
            selected = area;
            largest = overlap;
        }
    }
    let position = (
        coordinate(window.x, window.width, selected.x, selected.width),
        coordinate(window.y, window.height, selected.y, selected.height),
    );
    (position != (window.x, window.y)).then_some(position)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, width: u32, height: u32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn a_removed_monitor_returns_the_window_to_the_preferred_work_area() {
        let window = rect(2400, 150, 236, 44);
        assert_eq!(
            recover_position(window, &[rect(0, 24, 1440, 876)], 0),
            Some((1204, 150))
        );
    }

    #[test]
    fn a_fully_visible_drag_position_is_preserved_on_any_monitor() {
        let areas = [rect(-1920, -100, 1920, 1080), rect(0, 24, 1440, 876)];
        assert_eq!(recover_position(rect(-1800, -80, 236, 44), &areas, 1), None);
        assert_eq!(recover_position(rect(1204, 856, 236, 44), &areas, 0), None);
    }

    #[test]
    fn the_greatest_physical_overlap_wins_over_the_preferred_monitor() {
        let areas = [rect(-1920, 0, 1920, 1080), rect(0, 24, 2560, 1416)];
        assert_eq!(
            recover_position(rect(-100, 10, 472, 88), &areas, 0),
            Some((0, 24))
        );
    }

    #[test]
    fn negative_origins_and_work_area_edges_are_clamped_without_scale_conversion() {
        let area = rect(-2560, -1440, 2560, 1400);
        assert_eq!(
            recover_position(rect(-2700, -1450, 472, 88), &[area], 0),
            Some((-2560, -1440))
        );
        assert_eq!(
            recover_position(rect(-200, -50, 472, 88), &[area], 0),
            Some((-472, -128))
        );
    }

    #[test]
    fn a_window_larger_than_the_work_area_keeps_its_top_left_drag_area_visible() {
        assert_eq!(
            recover_position(rect(200, 200, 472, 88), &[rect(-50, 24, 300, 40)], 0),
            Some((-50, 24))
        );
    }

    #[test]
    fn empty_invalid_or_not_yet_sized_surfaces_do_not_move() {
        let window = rect(100, 100, 236, 44);
        assert_eq!(recover_position(window, &[], 0), None);
        assert_eq!(recover_position(window, &[rect(0, 0, 0, 1080)], 0), None);
        assert_eq!(
            recover_position(rect(0, 0, 0, 44), &[rect(100, 100, 1000, 1000)], 0),
            None
        );
        assert_eq!(
            recover_position(window, &[rect(0, 0, 0, 0), rect(300, 20, 1000, 800)], 0),
            Some((300, 100))
        );
    }

    #[test]
    fn arithmetic_handles_extreme_physical_coordinates_and_dimensions() {
        let areas = [rect(i32::MIN, i32::MIN, u32::MAX, u32::MAX)];
        assert_eq!(
            recover_position(rect(i32::MAX, i32::MAX, u32::MAX, u32::MAX), &areas, 0),
            Some((i32::MIN, i32::MIN))
        );
        assert_eq!(
            recover_position(
                rect(i32::MIN, i32::MIN, 236, 44),
                &[rect(i32::MAX, i32::MAX, u32::MAX, u32::MAX)],
                usize::MAX
            ),
            Some((i32::MAX, i32::MAX))
        );
    }
}
