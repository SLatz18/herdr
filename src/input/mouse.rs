#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Position {
    Cell { column: u16, row: u16 },
    Pixels { x: u32, y: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HostGeometry {
    pub(crate) cols: u16,
    pub(crate) rows: u16,
    pub(crate) width_px: u32,
    pub(crate) height_px: u32,
    cell_width_px: u32,
    cell_height_px: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HostPixels {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) geometry: HostGeometry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HostAxis {
    count: u16,
    cell: u32,
}

impl HostGeometry {
    pub(crate) fn new(cols: u16, rows: u16, width_px: u32, height_px: u32) -> Option<Self> {
        Self::with_cell_size(cols, rows, width_px, height_px, 0, 0)
    }

    pub(crate) fn with_cell_size(
        cols: u16,
        rows: u16,
        width_px: u32,
        height_px: u32,
        cell_width_px: u32,
        cell_height_px: u32,
    ) -> Option<Self> {
        (cols > 0 && rows > 0 && width_px > 0 && height_px > 0).then_some(Self {
            cols,
            rows,
            width_px,
            height_px,
            cell_width_px,
            cell_height_px,
        })
    }

    #[cfg(unix)]
    pub(crate) fn current_with_cell_size(cell_size: Option<(u32, u32)>) -> Option<Self> {
        let size = crossterm::terminal::window_size().ok()?;
        let (cell_width_px, cell_height_px) = cell_size.unwrap_or((0, 0));
        Self::with_cell_size(
            size.columns,
            size.rows,
            u32::from(size.width),
            u32::from(size.height),
            cell_width_px,
            cell_height_px,
        )
    }

    pub(crate) fn cell(self, x: u32, y: u32) -> Option<(u16, u16)> {
        Some((
            self.x_axis().grid_cell(x.checked_sub(1)?)?,
            self.y_axis().grid_cell(y.checked_sub(1)?)?,
        ))
    }

    fn x_axis(self) -> HostAxis {
        HostAxis::new(self.cols, self.width_px, self.cell_width_px)
    }

    fn y_axis(self) -> HostAxis {
        HostAxis::new(self.rows, self.height_px, self.cell_height_px)
    }

    #[cfg(test)]
    fn column_boundary(self, column: u16) -> Option<u32> {
        self.x_axis().boundary(column)
    }

    #[cfg(test)]
    fn row_boundary(self, row: u16) -> Option<u32> {
        self.y_axis().boundary(row)
    }

    #[cfg(test)]
    fn column_padding(self) -> u32 {
        self.width_px
            .saturating_sub(u32::from(self.cols).saturating_mul(self.x_axis().cell))
    }
}

impl HostAxis {
    fn new(count: u16, extent: u32, cell: u32) -> Self {
        let derived = (extent / u32::from(count.max(1))).max(1);
        let cell = if cell > 0 && u32::from(count).saturating_mul(cell) <= extent {
            cell
        } else {
            derived
        };
        Self { count, cell }
    }

    /// Content-space origin of cell `index`.
    ///
    /// Ghostty SGR-Pixels is terminal-space (window padding already removed).
    /// Ioctl leftover `padding = ws_xpixel - cols * cell` is a fixed inset in
    /// the window, not extra pitch: do not use `index * ws_xpixel / cols`.
    fn boundary(self, index: u16) -> Option<u32> {
        (index <= self.count).then_some(u32::from(index) * self.cell)
    }

    fn grid_cell(self, pixel: u32) -> Option<u16> {
        let content_extent = u32::from(self.count).saturating_mul(self.cell);
        if self.count == 0 || self.cell == 0 || pixel >= content_extent {
            return None;
        }
        let cell = pixel / self.cell;
        u16::try_from(cell).ok().filter(|cell| *cell < self.count)
    }
}

impl HostPixels {
    pub(crate) fn pane_position(
        self,
        inner: ratatui::layout::Rect,
        child_width_px: u32,
        child_height_px: u32,
    ) -> Option<Position> {
        let (host_column, host_row) = self.geometry.cell(self.x, self.y)?;
        let end_column = inner.x.checked_add(inner.width)?;
        let end_row = inner.y.checked_add(inner.height)?;
        if host_column < inner.x
            || host_column >= end_column
            || host_row < inner.y
            || host_row >= end_row
        {
            return None;
        }
        Some(Position::Pixels {
            x: map_axis_within_cell(
                self.x,
                host_column,
                inner.x,
                inner.width,
                self.geometry.x_axis(),
                child_width_px,
            )?,
            y: map_axis_within_cell(
                self.y,
                host_row,
                inner.y,
                inner.height,
                self.geometry.y_axis(),
                child_height_px,
            )?,
        })
    }
}

fn map_axis_within_cell(
    pixel: u32,
    host_cell: u16,
    pane_start: u16,
    pane_cells: u16,
    host: HostAxis,
    child_extent: u32,
) -> Option<u32> {
    let local_cell = host_cell.checked_sub(pane_start)?;
    if local_cell >= pane_cells {
        return None;
    }
    let source_start = host.boundary(host_cell)?;
    let source_end = host.boundary(host_cell.checked_add(1)?)?;
    let target_start = boundary(local_cell, pane_cells, child_extent)?;
    let target_end = boundary(local_cell.checked_add(1)?, pane_cells, child_extent)?;
    let source_width = source_end.checked_sub(source_start)?;
    let target_width = target_end.checked_sub(target_start)?;
    let offset = pixel.checked_sub(1)?.checked_sub(source_start)?;
    if source_width == 0 || target_width == 0 || offset >= source_width {
        return None;
    }
    target_start
        .checked_add(scale(offset, source_width, target_width))?
        .checked_add(1)
}

#[cfg(any(unix, test))]
pub(crate) fn parse_report(data: &[u8]) -> Option<(u32, u32)> {
    let body = data.strip_prefix(b"\x1b[<")?;
    let body = body
        .strip_suffix(b"M")
        .or_else(|| body.strip_suffix(b"m"))?;
    let mut fields = body.split(|byte| *byte == b';');
    parse_number(fields.next()?)?;
    let x = parse_number(fields.next()?)?;
    let y = parse_number(fields.next()?)?;
    fields.next().is_none().then_some((x, y))
}

#[cfg(any(unix, test))]
pub(crate) fn report_at_cell(data: &[u8], column: u16, row: u16) -> Option<Vec<u8>> {
    let body = data.strip_prefix(b"\x1b[<")?;
    let suffix = if body.ends_with(b"M") { 'M' } else { 'm' };
    let body = body.strip_suffix(&[suffix as u8])?;
    let buttons = body.split(|byte| *byte == b';').next()?;
    Some(
        format!(
            "\x1b[<{};{};{}{}",
            std::str::from_utf8(buttons).ok()?,
            u32::from(column) + 1,
            u32::from(row) + 1,
            suffix
        )
        .into_bytes(),
    )
}

#[cfg(any(unix, test))]
fn parse_number(value: &[u8]) -> Option<u32> {
    (!value.is_empty() && value.iter().all(u8::is_ascii_digit))
        .then(|| std::str::from_utf8(value).ok()?.parse().ok())
        .flatten()
}

fn boundary(index: u16, count: u16, extent: u32) -> Option<u32> {
    (count > 0 && index <= count && extent > 0)
        .then(|| (u64::from(index) * u64::from(extent) / u64::from(count)) as u32)
}

/// 1-based SGR pixel at the origin of `cell` in a `count`-cell axis of `extent` px.
pub(crate) fn cell_origin_pixel(cell: u16, count: u16, extent: u32) -> Option<u32> {
    boundary(cell, count, extent).map(|start| start.saturating_add(1))
}

fn scale(pixel: u32, source: u32, target: u32) -> u32 {
    ((u64::from(pixel) * u64::from(target)) / u64::from(source))
        .min(u64::from(target.saturating_sub(1))) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_accepts_only_complete_sgr_mouse_reports() {
        for (input, expected) in [
            (b"\x1b[<35;321;241M".as_slice(), Some((321, 241))),
            (b"\x1b[<0;1;2m".as_slice(), Some((1, 2))),
            (b"key".as_slice(), None),
            (b"\x1b[<0;1;2Mkey".as_slice(), None),
            (b"\x1b[<0;1M".as_slice(), None),
        ] {
            assert_eq!(parse_report(input), expected);
        }
    }

    #[test]
    fn integer_cell_pitch_maps_pane_pixels_without_stretching_padding() {
        let geometry = HostGeometry::with_cell_size(211, 57, 2_537, 1_429, 12, 25).unwrap();
        assert_eq!(geometry.column_padding(), 5);
        let inner = ratatui::layout::Rect::new(157, 7, 53, 49);
        let start_x = geometry.column_boundary(inner.x).unwrap();
        let end_x = geometry.column_boundary(inner.x + inner.width).unwrap();
        let start_y = geometry.row_boundary(inner.y).unwrap();
        let end_y = geometry.row_boundary(inner.y + inner.height).unwrap();
        assert_eq!(
            HostPixels {
                x: start_x + 1,
                y: start_y + 1,
                geometry,
            }
            .pane_position(inner, 636, 1_225),
            Some(Position::Pixels { x: 1, y: 1 })
        );
        assert_eq!(
            HostPixels {
                x: end_x,
                y: end_y,
                geometry,
            }
            .pane_position(inner, 636, 1_225),
            Some(Position::Pixels { x: 636, y: 1_225 })
        );
    }

    #[test]
    fn padded_ioctl_extent_keeps_late_column_click_in_the_right_half() {
        // Ghostty-style leftover: 1276px / 127 cols with a true 10px cell.
        let geometry = HostGeometry::with_cell_size(127, 24, 1_276, 480, 10, 20).unwrap();
        assert_eq!(geometry.column_padding(), 6);
        assert_eq!(
            HostGeometry::new(127, 24, 1_276, 480)
                .unwrap()
                .x_axis()
                .cell,
            10
        );
        let column = 120u16;
        let in_cell = 9u32;
        let x = u32::from(column) * 10 + in_cell + 1;
        assert_eq!(geometry.cell(x, 1), Some((column, 0)));
        let Position::Pixels { x: child_x, .. } = HostPixels { x, y: 1, geometry }
            .pane_position(ratatui::layout::Rect::new(0, 0, 127, 24), 1_270, 480)
            .expect("mapped pane pixels")
        else {
            panic!("expected pane-local pixels");
        };
        let half = 5;
        assert!(
            child_x > u32::from(column) * 10 + half,
            "right-side click slipped into the left half of the cell: {child_x}"
        );
        assert_eq!(child_x, u32::from(column) * 10 + in_cell + 1);
    }

    #[test]
    fn fractional_scaling_preserves_the_canonical_child_cell() {
        let geometry = HostGeometry::new(80, 1, 805, 20).unwrap();
        assert_eq!(geometry.cell(11, 1), Some((1, 0)));
        assert_eq!(
            HostPixels {
                x: 11,
                y: 1,
                geometry,
            }
            .pane_position(ratatui::layout::Rect::new(0, 0, 80, 1), 800, 20),
            Some(Position::Pixels { x: 11, y: 1 })
        );
    }

    #[test]
    fn cell_origin_pixel_is_the_one_based_boundary_not_the_cell_index() {
        assert_eq!(cell_origin_pixel(4, 80, 800), Some(41));
        assert_eq!(cell_origin_pixel(0, 80, 800), Some(1));
        assert_ne!(cell_origin_pixel(4, 80, 800), Some(5));
    }

    #[test]
    fn geometry_rejects_outside_pixels_and_maps_cells() {
        let geometry = HostGeometry::new(80, 24, 800, 480).unwrap();
        assert_eq!(geometry.cell(1, 1), Some((0, 0)));
        assert_eq!(geometry.cell(800, 480), Some((79, 23)));
        assert_eq!(geometry.cell(801, 1), None);
        assert_eq!(geometry.cell(0, 1), None);
    }
}
