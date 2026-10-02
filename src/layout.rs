//! N-up sheet geometry. Coordinates are PDF points, origin at the bottom left.

/// Points in one inch.
pub const PT_PER_INCH: f64 = 72.0;
/// Points in one millimetre.
pub const PT_PER_MM: f64 = PT_PER_INCH / 25.4;

/// Crop marks start this far outside the cell.
pub const CROP_MARK_OFFSET_PT: f64 = PT_PER_MM;
/// Length of each crop mark.
pub const CROP_MARK_LENGTH_PT: f64 = 5.0 * PT_PER_MM;
/// Stroke width used when the marks are drawn.
pub const CROP_MARK_STROKE_PT: f64 = 0.25;

/// Clearance a margin or gutter needs so a crop mark stays out of the next cell.
pub fn crop_mark_clearance_pt() -> f64 {
    CROP_MARK_OFFSET_PT + CROP_MARK_LENGTH_PT
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayUnit {
    Millimetres,
    Inches,
}

impl DisplayUnit {
    pub fn suffix(self) -> &'static str {
        match self {
            Self::Millimetres => " mm",
            Self::Inches => " in",
        }
    }

    pub fn from_pt(self, points: f64) -> f64 {
        match self {
            Self::Millimetres => points / PT_PER_MM,
            Self::Inches => points / PT_PER_INCH,
        }
    }

    pub fn to_pt(self, value: f64) -> f64 {
        match self {
            Self::Millimetres => value * PT_PER_MM,
            Self::Inches => value * PT_PER_INCH,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Portrait,
    Landscape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SheetPreset {
    Letter,
    Legal,
    Tabloid,
    A4,
    A3,
    Sra3,
    Custom,
}

impl SheetPreset {
    pub const ALL: [Self; 7] = [
        Self::Letter,
        Self::Legal,
        Self::Tabloid,
        Self::A4,
        Self::A3,
        Self::Sra3,
        Self::Custom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Letter => "Letter",
            Self::Legal => "Legal",
            Self::Tabloid => "Tabloid",
            Self::A4 => "A4",
            Self::A3 => "A3",
            Self::Sra3 => "SRA3",
            Self::Custom => "Custom",
        }
    }

    /// Width and height in points before orientation is applied. Portrait presets
    /// are stored with the shorter side as the width.
    pub fn base_points(self) -> Option<(f64, f64)> {
        match self {
            Self::Letter => Some((612.0, 792.0)),
            Self::Legal => Some((612.0, 1008.0)),
            Self::Tabloid => Some((792.0, 1224.0)),
            Self::A4 => Some((210.0 * PT_PER_MM, 297.0 * PT_PER_MM)),
            Self::A3 => Some((297.0 * PT_PER_MM, 420.0 * PT_PER_MM)),
            Self::Sra3 => Some((320.0 * PT_PER_MM, 450.0 * PT_PER_MM)),
            Self::Custom => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FitMode {
    Contain,
    Fill,
}

impl FitMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Contain => "Contain",
            Self::Fill => "Fill",
        }
    }
}

/// Axis-aligned rectangle. `x` and `y` are the bottom-left corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct SourceSize {
    pub width_pt: f64,
    pub height_pt: f64,
    /// Finished trim size. `None` uses the cell from the sheet, margins, and gutters.
    pub target_width_pt: Option<f64>,
    pub target_height_pt: Option<f64>,
}

impl SourceSize {
    pub fn page(width_pt: f64, height_pt: f64) -> Self {
        Self {
            width_pt,
            height_pt,
            target_width_pt: None,
            target_height_pt: None,
        }
    }

    pub fn with_target(mut self, width_pt: f64, height_pt: f64) -> Self {
        self.target_width_pt = Some(width_pt);
        self.target_height_pt = Some(height_pt);
        self
    }
}

#[derive(Debug, Clone)]
pub struct SheetSpec {
    pub width_pt: f64,
    pub height_pt: f64,
    pub columns: u32,
    pub rows: u32,
    pub margin_top: f64,
    pub margin_right: f64,
    pub margin_bottom: f64,
    pub margin_left: f64,
    pub gutter_x: f64,
    pub gutter_y: f64,
    /// How far artwork extends past the trim on each side.
    pub bleed_top: f64,
    pub bleed_right: f64,
    pub bleed_bottom: f64,
    pub bleed_left: f64,
    pub crop_marks: bool,
    pub fit: FitMode,
    /// Pages per folded section. `0` keeps reading order.
    pub signature_pages: u32,
}

#[derive(Debug, Clone)]
pub struct Placement {
    pub source_index: usize,
    /// Finished trim. The grid cell when no target size is set.
    pub trim: Rect,
    /// Artwork rectangle before clipping.
    pub content: Rect,
    /// Trim expanded by bleed, clipped to the sheet and to the gutter center.
    pub clip: Rect,
}

#[derive(Debug, Clone)]
pub struct SheetLayout {
    /// Every cell on the sheet, row-major, including cells with no source.
    pub cells: Vec<Rect>,
    pub placements: Vec<Placement>,
    pub crop_marks: Vec<Segment>,
}

/// Visual page size after a clockwise PDF `/Rotate` value.
pub fn visual_page_size(width_pt: f64, height_pt: f64, rotate_deg: i64) -> (f64, f64) {
    let (_, w, h) = upright_matrix(0.0, 0.0, width_pt, height_pt, rotate_deg);
    (w, h)
}

/// Matrix that maps a page box into an upright rectangle at the origin.
///
/// PDF `/Rotate` is clockwise. The returned matrix is `[a b c d e f]`
/// (`x' = a x + c y + e`, `y' = b x + d y + f`). The last two values are the
/// upright width and height.
pub fn upright_matrix(
    llx: f64,
    lly: f64,
    urx: f64,
    ury: f64,
    rotate_deg: i64,
) -> ([f64; 6], f64, f64) {
    let (llx, urx) = if llx <= urx { (llx, urx) } else { (urx, llx) };
    let (lly, ury) = if lly <= ury { (lly, ury) } else { (ury, lly) };
    let w = urx - llx;
    let h = ury - lly;
    let rot = rotate_deg.rem_euclid(360);
    let (matrix, eff_w, eff_h) = match rot {
        90 => ([0.0, -1.0, 1.0, 0.0, -lly, w + llx], h, w),
        180 => ([-1.0, 0.0, 0.0, -1.0, w + llx, h + lly], w, h),
        270 => ([0.0, 1.0, -1.0, 0.0, h + lly, -llx], h, w),
        _ => ([1.0, 0.0, 0.0, 1.0, -llx, -lly], w, h),
    };
    (matrix, eff_w, eff_h)
}

/// True when a margin or gutter is too small for the bleed and crop marks.
///
/// With no sources, this uses the grid cell. With sources, each target trim is
/// measured instead, so a smaller target gets the extra room around it.
pub fn spacing_is_tight(spec: &SheetSpec, sources: &[SourceSize]) -> bool {
    if sources.is_empty() {
        return container_spacing_is_tight(spec);
    }
    let Ok(cells) = cell_rects(spec) else {
        return false;
    };
    if cells.is_empty() {
        return false;
    }
    let mark = if spec.crop_marks {
        crop_mark_clearance_pt()
    } else {
        0.0
    };
    let bleed_left = spec.bleed_left.max(0.0);
    let bleed_right = spec.bleed_right.max(0.0);
    let bleed_top = spec.bleed_top.max(0.0);
    let bleed_bottom = spec.bleed_bottom.max(0.0);
    let per_sheet = cells.len();
    let slots = placement_slots(sources.len(), spec);
    let placed: Vec<(usize, u32, u32, Rect, Rect)> = slots
        .iter()
        .enumerate()
        .filter_map(|(index, slot)| {
            let source = sources.get((*slot)?)?;
            let cell_index = index % per_sheet;
            let sheet = index / per_sheet;
            let column = (cell_index as u32) % spec.columns;
            let row = (cell_index as u32) / spec.columns;
            let grid = cells[cell_index];
            Some((sheet, column, row, grid, target_trim(grid, *source)))
        })
        .collect();

    for &(sheet, column, row, grid, trim) in &placed {
        if column == 0 && trim.x + 1e-6 < bleed_left + mark {
            return true;
        }
        if column + 1 == spec.columns
            && spec.width_pt - (trim.x + trim.w) + 1e-6 < bleed_right + mark
        {
            return true;
        }
        if row == 0 && spec.height_pt - (trim.y + trim.h) + 1e-6 < bleed_top + mark {
            return true;
        }
        if row + 1 == spec.rows && trim.y + 1e-6 < bleed_bottom + mark {
            return true;
        }
        if column + 1 < spec.columns {
            if let Some(neighbor) = placed
                .iter()
                .find(|item| item.0 == sheet && item.1 == column + 1 && item.2 == row)
            {
                let gap = neighbor.4.x - (trim.x + trim.w);
                if gap + 1e-6 < bleed_left + bleed_right + mark {
                    return true;
                }
            } else {
                let room = grid.x + grid.w + spec.gutter_x / 2.0 - (trim.x + trim.w);
                if room + 1e-6 < bleed_right {
                    return true;
                }
            }
        }
        if row + 1 < spec.rows {
            if let Some(neighbor) = placed
                .iter()
                .find(|item| item.0 == sheet && item.1 == column && item.2 == row + 1)
            {
                let gap = trim.y - (neighbor.4.y + neighbor.4.h);
                if gap + 1e-6 < bleed_top + bleed_bottom + mark {
                    return true;
                }
            } else {
                let room = trim.y - (grid.y - spec.gutter_y / 2.0);
                if room + 1e-6 < bleed_bottom {
                    return true;
                }
            }
        }
    }
    false
}

fn container_spacing_is_tight(spec: &SheetSpec) -> bool {
    let mark = if spec.crop_marks {
        crop_mark_clearance_pt()
    } else {
        0.0
    };
    let edges = [
        (spec.margin_top, spec.bleed_top),
        (spec.margin_right, spec.bleed_right),
        (spec.margin_bottom, spec.bleed_bottom),
        (spec.margin_left, spec.bleed_left),
    ];
    if edges
        .iter()
        .any(|(margin, bleed)| *margin + 1e-6 < *bleed + mark)
    {
        return true;
    }
    if spec.columns > 1 && spec.gutter_x + 1e-6 < spec.bleed_left + spec.bleed_right + mark {
        return true;
    }
    if spec.rows > 1 && spec.gutter_y + 1e-6 < spec.bleed_top + spec.bleed_bottom + mark {
        return true;
    }
    false
}

/// Cell order for a folded signature.
///
/// Pages are paired left/right the way a saddle signature folds. Fronts of the
/// nested sheets are placed first, then backs. On a 2×2 sheet a signature of 8
/// is `8, 1, 6, 3` and then `2, 7, 4, 5`. A wider sheet continues that sequence
/// left to right. `signature_pages` below 4, or fewer than 2 columns, keeps
/// reading order.
pub fn signature_slots(
    page_count: usize,
    columns: u32,
    rows: u32,
    signature_pages: u32,
) -> Vec<Option<usize>> {
    if page_count == 0 || signature_pages < 4 || columns < 2 || rows == 0 {
        return (0..page_count).map(Some).collect();
    }
    let signature = (signature_pages as usize).div_ceil(4) * 4;
    let mut slots = Vec::new();
    let mut start = 0usize;
    while start < page_count {
        let available = (page_count - start).min(signature);
        slots.extend(signature_cells(start, available, signature, columns, rows));
        start += signature;
    }
    slots
}

fn placement_slots(page_count: usize, spec: &SheetSpec) -> Vec<Option<usize>> {
    signature_slots(page_count, spec.columns, spec.rows, spec.signature_pages)
}

fn signature_cells(
    base: usize,
    available: usize,
    signature: usize,
    columns: u32,
    rows: u32,
) -> Vec<Option<usize>> {
    let nested = signature / 4;
    let mut fronts = Vec::with_capacity(nested);
    let mut backs = Vec::with_capacity(nested);
    for index in 0..nested {
        fronts.push((signature - 2 * index, 2 * index + 1));
        backs.push((2 * index + 2, signature - 2 * index - 1));
    }
    let mut cells = place_side(&fronts, columns, rows, base, available);
    cells.extend(place_side(&backs, columns, rows, base, available));
    cells
}

fn place_side(
    pairs: &[(usize, usize)],
    columns: u32,
    rows: u32,
    base: usize,
    available: usize,
) -> Vec<Option<usize>> {
    let per_sheet = (columns as usize) * (rows as usize);
    if per_sheet == 0 {
        return Vec::new();
    }
    let pages: Vec<usize> = pairs.iter().flat_map(|(left, right)| [*left, *right]).collect();
    let mut cells = Vec::new();
    for chunk in pages.chunks(per_sheet) {
        for page in chunk {
            cells.push(signature_page(*page, base, available));
        }
        while cells.len() % per_sheet != 0 {
            cells.push(None);
        }
    }
    cells
}

fn signature_page(one_based: usize, base: usize, available: usize) -> Option<usize> {
    let offset = one_based.saturating_sub(1);
    if offset < available {
        Some(base + offset)
    } else {
        None
    }
}

/// True when a requested trim fits inside one grid cell.
pub fn target_fits_cell(cell_w: f64, cell_h: f64, target_w: f64, target_h: f64) -> bool {
    target_w > 1.0 && target_h > 1.0 && target_w <= cell_w + 1e-4 && target_h <= cell_h + 1e-4
}

/// Width and height of one grid cell, the largest trim the sheet can hold.
pub fn trim_size(spec: &SheetSpec) -> Result<(f64, f64), String> {
    let cells = cell_rects(spec)?;
    let cell = cells
        .first()
        .ok_or("Columns and rows must be at least 1.")?;
    Ok((cell.w, cell.h))
}

pub fn impose(sources: &[SourceSize], spec: &SheetSpec) -> Result<Vec<SheetLayout>, String> {
    if spec.columns == 0 || spec.rows == 0 {
        return Err("Columns and rows must be at least 1.".into());
    }
    if spec.width_pt <= 1.0 || spec.height_pt <= 1.0 {
        return Err("The sheet is too small.".into());
    }

    let cells = cell_rects(spec)?;
    let per_sheet = cells.len();
    if sources.is_empty() {
        return Ok(vec![SheetLayout {
            cells,
            placements: Vec::new(),
            crop_marks: Vec::new(),
        }]);
    }

    let slots = placement_slots(sources.len(), spec);
    let sheet_count = slots.len().div_ceil(per_sheet).max(1);
    let mut sheets = Vec::with_capacity(sheet_count);
    for sheet_index in 0..sheet_count {
        let start = sheet_index * per_sheet;
        let mut placements = Vec::new();
        let mut crop_marks = Vec::new();
        for cell_index in 0..per_sheet {
            let Some(source_index) = slots.get(start + cell_index).copied().flatten() else {
                continue;
            };
            let Some(source) = sources.get(source_index).copied() else {
                continue;
            };
            let cell = cells[cell_index];
            let column = (cell_index as u32) % spec.columns;
            let row = (cell_index as u32) / spec.columns;
            let trim = target_trim(cell, source);
            let clip = bleed_rect(trim, cell, column, row, spec);
            placements.push(Placement {
                source_index,
                trim,
                content: fit_content(clip, source, spec.fit),
                clip,
            });
            if spec.crop_marks {
                crop_marks.extend(crop_marks_for_cell(trim, spec));
            }
        }
        sheets.push(SheetLayout {
            cells: cells.clone(),
            placements,
            crop_marks,
        });
    }
    Ok(sheets)
}

fn cell_rects(spec: &SheetSpec) -> Result<Vec<Rect>, String> {
    let columns = spec.columns as f64;
    let rows = spec.rows as f64;
    let content_w = spec.width_pt - spec.margin_left - spec.margin_right;
    let content_h = spec.height_pt - spec.margin_top - spec.margin_bottom;
    let gaps_x = spec.gutter_x * (columns - 1.0);
    let gaps_y = spec.gutter_y * (rows - 1.0);
    let cell_w = (content_w - gaps_x) / columns;
    let cell_h = (content_h - gaps_y) / rows;
    if cell_w <= 1.0 || cell_h <= 1.0 {
        return Err("Margins and gutters leave no room for pages on the sheet.".into());
    }

    let mut cells = Vec::with_capacity((spec.columns * spec.rows) as usize);
    for row in 0..spec.rows {
        for column in 0..spec.columns {
            let x = spec.margin_left + column as f64 * (cell_w + spec.gutter_x);
            let y = spec.height_pt
                - spec.margin_top
                - (row as f64 + 1.0) * cell_h
                - row as f64 * spec.gutter_y;
            cells.push(Rect {
                x,
                y,
                w: cell_w,
                h: cell_h,
            });
        }
    }
    Ok(cells)
}

fn target_trim(cell: Rect, source: SourceSize) -> Rect {
    match (source.target_width_pt, source.target_height_pt) {
        (Some(width), Some(height)) if target_fits_cell(cell.w, cell.h, width, height) => Rect {
            x: cell.x + (cell.w - width) / 2.0,
            y: cell.y + (cell.h - height) / 2.0,
            w: width,
            h: height,
        },
        _ => cell,
    }
}

fn bleed_rect(trim: Rect, grid: Rect, column: u32, row: u32, spec: &SheetSpec) -> Rect {
    let mut x0 = trim.x - spec.bleed_left.max(0.0);
    let mut y0 = trim.y - spec.bleed_bottom.max(0.0);
    let mut x1 = trim.x + trim.w + spec.bleed_right.max(0.0);
    let mut y1 = trim.y + trim.h + spec.bleed_top.max(0.0);

    if column > 0 {
        x0 = x0.max(grid.x - spec.gutter_x / 2.0);
    }
    if column + 1 < spec.columns {
        x1 = x1.min(grid.x + grid.w + spec.gutter_x / 2.0);
    }
    if row + 1 < spec.rows {
        y0 = y0.max(grid.y - spec.gutter_y / 2.0);
    }
    if row > 0 {
        y1 = y1.min(grid.y + grid.h + spec.gutter_y / 2.0);
    }

    x0 = x0.clamp(0.0, spec.width_pt);
    x1 = x1.clamp(x0, spec.width_pt);
    y0 = y0.clamp(0.0, spec.height_pt);
    y1 = y1.clamp(y0, spec.height_pt);
    Rect {
        x: x0,
        y: y0,
        w: x1 - x0,
        h: y1 - y0,
    }
}

fn fit_content(cell: Rect, source: SourceSize, fit: FitMode) -> Rect {
    if source.width_pt <= 0.0 || source.height_pt <= 0.0 {
        return cell;
    }
    let scale_x = cell.w / source.width_pt;
    let scale_y = cell.h / source.height_pt;
    let scale = match fit {
        FitMode::Contain => scale_x.min(scale_y),
        FitMode::Fill => scale_x.max(scale_y),
    };
    let w = source.width_pt * scale;
    let h = source.height_pt * scale;
    Rect {
        x: cell.x + (cell.w - w) / 2.0,
        y: cell.y + (cell.h - h) / 2.0,
        w,
        h,
    }
}

fn crop_marks_for_cell(cell: Rect, spec: &SheetSpec) -> Vec<Segment> {
    let offset = CROP_MARK_OFFSET_PT;
    let length = CROP_MARK_LENGTH_PT;
    let x0 = cell.x;
    let y0 = cell.y;
    let x1 = cell.x + cell.w;
    let y1 = cell.y + cell.h;
    let left = spec.bleed_left.max(0.0);
    let bottom = spec.bleed_bottom.max(0.0);
    let right = spec.bleed_right.max(0.0);
    let top = spec.bleed_top.max(0.0);
    let raw = [
        // Bottom left, outside the bleed.
        (x0 - left - offset - length, y0, x0 - left - offset, y0),
        (x0, y0 - bottom - offset - length, x0, y0 - bottom - offset),
        // Bottom right.
        (x1 + right + offset, y0, x1 + right + offset + length, y0),
        (
            x1,
            y0 - bottom - offset - length,
            x1,
            y0 - bottom - offset,
        ),
        // Top left.
        (x0 - left - offset - length, y1, x0 - left - offset, y1),
        (x0, y1 + top + offset, x0, y1 + top + offset + length),
        // Top right.
        (x1 + right + offset, y1, x1 + right + offset + length, y1),
        (x1, y1 + top + offset, x1, y1 + top + offset + length),
    ];
    raw.into_iter()
        .filter_map(|(x1, y1, x2, y2)| clip_segment(x1, y1, x2, y2, spec.width_pt, spec.height_pt))
        .collect()
}

fn clip_segment(x1: f64, y1: f64, x2: f64, y2: f64, sheet_w: f64, sheet_h: f64) -> Option<Segment> {
    const MIN_LENGTH: f64 = 0.5;
    if (x1 - x2).abs() < 1e-6 {
        if !(0.0..=sheet_w).contains(&x1) {
            return None;
        }
        let (lo, hi) = if y1 <= y2 { (y1, y2) } else { (y2, y1) };
        let lo = lo.max(0.0);
        let hi = hi.min(sheet_h);
        if hi - lo < MIN_LENGTH {
            return None;
        }
        return Some(Segment {
            x1,
            y1: lo,
            x2: x1,
            y2: hi,
        });
    }
    if (y1 - y2).abs() < 1e-6 {
        if !(0.0..=sheet_h).contains(&y1) {
            return None;
        }
        let (lo, hi) = if x1 <= x2 { (x1, x2) } else { (x2, x1) };
        let lo = lo.max(0.0);
        let hi = hi.min(sheet_w);
        if hi - lo < MIN_LENGTH {
            return None;
        }
        return Some(Segment {
            x1: lo,
            y1,
            x2: hi,
            y2: y1,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn letter_grid() -> SheetSpec {
        SheetSpec {
            width_pt: 612.0,
            height_pt: 792.0,
            columns: 2,
            rows: 2,
            margin_top: 36.0,
            margin_right: 36.0,
            margin_bottom: 36.0,
            margin_left: 36.0,
            gutter_x: 18.0,
            gutter_y: 18.0,
            bleed_top: 0.0,
            bleed_right: 0.0,
            bleed_bottom: 0.0,
            bleed_left: 0.0,
            crop_marks: false,
            fit: FitMode::Contain,
            signature_pages: 0,
        }
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
    }

    #[test]
    fn two_by_two_letter_cells() {
        let spec = letter_grid();
        let sheets = impose(&[], &spec).unwrap();
        assert_eq!(sheets.len(), 1);
        let cells = &sheets[0].cells;
        assert_eq!(cells.len(), 4);

        // content 540 x 720, gutters 18, cells 261 x 351.
        let (trim_w, trim_h) = trim_size(&spec).unwrap();
        assert_close(trim_w, 261.0);
        assert_close(trim_h, 351.0);
        let expected = [(36.0, 405.0), (315.0, 405.0), (36.0, 36.0), (315.0, 36.0)];
        for (cell, (x, y)) in cells.iter().zip(expected) {
            assert_close(cell.x, x);
            assert_close(cell.y, y);
            assert_close(cell.w, 261.0);
            assert_close(cell.h, 351.0);
        }
        assert_close(cells[1].x - (cells[0].x + cells[0].w), 18.0);
        assert_close(cells[0].y - (cells[2].y + cells[2].h), 18.0);
    }

    #[test]
    fn row_major_order_across_sheets() {
        let mut spec = letter_grid();
        spec.columns = 2;
        spec.rows = 1;
        let sources = vec![SourceSize::page(100.0, 100.0); 5];
        let sheets = impose(&sources, &spec).unwrap();
        assert_eq!(sheets.len(), 3);
        let indexes: Vec<Vec<_>> = sheets
            .iter()
            .map(|sheet| {
                sheet
                    .placements
                    .iter()
                    .map(|placement| placement.source_index)
                    .collect()
            })
            .collect();
        assert_eq!(indexes, vec![vec![0, 1], vec![2, 3], vec![4]]);
        assert_eq!(sheets[2].cells.len(), 2);
        assert!(sheets[2].placements.len() == 1);
    }

    #[test]
    fn rotation_swaps_page_size() {
        assert_eq!(visual_page_size(200.0, 400.0, 0), (200.0, 400.0));
        assert_eq!(visual_page_size(200.0, 400.0, 90), (400.0, 200.0));
        assert_eq!(visual_page_size(200.0, 400.0, 180), (200.0, 400.0));
        assert_eq!(visual_page_size(200.0, 400.0, 270), (400.0, 200.0));
        assert_eq!(visual_page_size(200.0, 400.0, -90), (400.0, 200.0));
    }

    #[test]
    fn upright_matrix_covers_positive_box() {
        let (matrix, width, height) = upright_matrix(10.0, 20.0, 110.0, 220.0, 90);
        assert_close(width, 200.0);
        assert_close(height, 100.0);
        let corners = [(10.0, 20.0), (110.0, 20.0), (10.0, 220.0), (110.0, 220.0)];
        let mapped: Vec<(f64, f64)> = corners
            .into_iter()
            .map(|(x, y)| {
                (
                    matrix[0] * x + matrix[2] * y + matrix[4],
                    matrix[1] * x + matrix[3] * y + matrix[5],
                )
            })
            .collect();
        let min_x = mapped
            .iter()
            .map(|point| point.0)
            .fold(f64::INFINITY, f64::min);
        let max_x = mapped
            .iter()
            .map(|point| point.0)
            .fold(f64::NEG_INFINITY, f64::max);
        let min_y = mapped
            .iter()
            .map(|point| point.1)
            .fold(f64::INFINITY, f64::min);
        let max_y = mapped
            .iter()
            .map(|point| point.1)
            .fold(f64::NEG_INFINITY, f64::max);
        assert_close(min_x, 0.0);
        assert_close(min_y, 0.0);
        assert_close(max_x, width);
        assert_close(max_y, height);
    }

    #[test]
    fn crop_marks_stay_outside_cells_and_inside_the_sheet() {
        let spec = SheetSpec {
            width_pt: 1000.0,
            height_pt: 1000.0,
            columns: 1,
            rows: 1,
            margin_top: 100.0,
            margin_right: 100.0,
            margin_bottom: 100.0,
            margin_left: 100.0,
            gutter_x: 0.0,
            gutter_y: 0.0,
            bleed_top: 0.0,
            bleed_right: 0.0,
            bleed_bottom: 0.0,
            bleed_left: 0.0,
            crop_marks: true,
            fit: FitMode::Contain,
            signature_pages: 0,
        };
        let sheets = impose(
            &[SourceSize::page(100.0, 100.0)],
            &spec,
        )
        .unwrap();
        let cell = sheets[0].cells[0];
        let marks = &sheets[0].crop_marks;
        assert_eq!(marks.len(), 8);
        for mark in marks {
            for (x, y) in [(mark.x1, mark.y1), (mark.x2, mark.y2)] {
                assert!((0.0..=spec.width_pt).contains(&x));
                assert!((0.0..=spec.height_pt).contains(&y));
                let inside = x > cell.x + 1e-6
                    && x < cell.x + cell.w - 1e-6
                    && y > cell.y + 1e-6
                    && y < cell.y + cell.h - 1e-6;
                assert!(!inside, "mark enters the cell: {mark:?}");
            }
        }

        let gap = marks
            .iter()
            .filter(|mark| (mark.y1 - cell.y).abs() < 1e-6 && mark.x2 < cell.x)
            .map(|mark| cell.x - mark.x2)
            .next()
            .expect("bottom-left horizontal mark");
        assert_close(gap, CROP_MARK_OFFSET_PT);
    }

    #[test]
    fn crop_marks_are_clipped_to_the_sheet() {
        let spec = SheetSpec {
            width_pt: 200.0,
            height_pt: 200.0,
            columns: 1,
            rows: 1,
            margin_top: 149.0,
            margin_right: 149.0,
            margin_bottom: 1.0,
            margin_left: 1.0,
            gutter_x: 0.0,
            gutter_y: 0.0,
            bleed_top: 0.0,
            bleed_right: 0.0,
            bleed_bottom: 0.0,
            bleed_left: 0.0,
            crop_marks: true,
            fit: FitMode::Contain,
            signature_pages: 0,
        };
        let sheets = impose(
            &[SourceSize::page(10.0, 10.0)],
            &spec,
        )
        .unwrap();
        assert!(!sheets[0].crop_marks.is_empty());
        for mark in &sheets[0].crop_marks {
            for value in [mark.x1, mark.x2] {
                assert!((0.0..=spec.width_pt).contains(&value));
            }
            for value in [mark.y1, mark.y2] {
                assert!((0.0..=spec.height_pt).contains(&value));
            }
        }
    }

    #[test]
    fn bleed_extends_artwork_and_crop_marks() {
        let bleed = 10.0;
        let spec = SheetSpec {
            width_pt: 400.0,
            height_pt: 400.0,
            columns: 1,
            rows: 1,
            margin_top: 80.0,
            margin_right: 80.0,
            margin_bottom: 80.0,
            margin_left: 80.0,
            gutter_x: 0.0,
            gutter_y: 0.0,
            bleed_top: bleed,
            bleed_right: bleed,
            bleed_bottom: bleed,
            bleed_left: bleed,
            crop_marks: true,
            fit: FitMode::Contain,
            signature_pages: 0,
        };
        let sheets = impose(
            &[SourceSize::page(100.0, 100.0)],
            &spec,
        )
        .unwrap();
        let cell = sheets[0].cells[0];
        let placement = &sheets[0].placements[0];
        assert_close(placement.clip.x, cell.x - bleed);
        assert_close(placement.clip.y, cell.y - bleed);
        assert_close(placement.clip.w, cell.w + bleed * 2.0);
        assert_close(placement.clip.h, cell.h + bleed * 2.0);
        assert_close(placement.content.x, placement.clip.x);
        assert_close(placement.content.w, placement.clip.w);

        let horizontal = sheets[0]
            .crop_marks
            .iter()
            .find(|mark| (mark.y1 - cell.y).abs() < 1e-6 && mark.x2 < cell.x)
            .unwrap();
        assert_close(horizontal.x2, cell.x - bleed - CROP_MARK_OFFSET_PT);
        assert_close(
            horizontal.x1,
            cell.x - bleed - CROP_MARK_OFFSET_PT - CROP_MARK_LENGTH_PT,
        );
    }

    #[test]
    fn bleed_stops_at_the_gutter_and_the_sheet_edge() {
        let spec = SheetSpec {
            width_pt: 400.0,
            height_pt: 200.0,
            columns: 2,
            rows: 1,
            margin_top: 20.0,
            margin_right: 5.0,
            margin_bottom: 20.0,
            margin_left: 5.0,
            gutter_x: 10.0,
            gutter_y: 0.0,
            bleed_top: 20.0,
            bleed_right: 20.0,
            bleed_bottom: 20.0,
            bleed_left: 20.0,
            crop_marks: false,
            fit: FitMode::Contain,
            signature_pages: 0,
        };
        let sheets = impose(
            &[
                SourceSize::page(100.0, 100.0),
                SourceSize::page(100.0, 100.0),
            ],
            &spec,
        )
        .unwrap();
        let left = sheets[0].cells[0];
        let clip = sheets[0].placements[0].clip;
        assert_close(clip.x, 0.0);
        assert_close(clip.x + clip.w, left.x + left.w + spec.gutter_x / 2.0);
        assert_close(clip.y, left.y - spec.bleed_bottom);
    }

    #[test]
    fn target_size_is_centered_when_it_fits_and_ignored_when_it_does_not() {
        let spec = SheetSpec {
            width_pt: 400.0,
            height_pt: 300.0,
            columns: 1,
            rows: 1,
            margin_top: 0.0,
            margin_right: 0.0,
            margin_bottom: 0.0,
            margin_left: 0.0,
            gutter_x: 0.0,
            gutter_y: 0.0,
            bleed_top: 0.0,
            bleed_right: 0.0,
            bleed_bottom: 0.0,
            bleed_left: 0.0,
            crop_marks: true,
            fit: FitMode::Contain,
            signature_pages: 0,
        };
        let fitted = impose(
            &[SourceSize::page(100.0, 100.0).with_target(100.0, 50.0)],
            &spec,
        )
        .unwrap();
        let cell = fitted[0].cells[0];
        let trim = fitted[0].placements[0].trim;
        assert_close(trim.w, 100.0);
        assert_close(trim.h, 50.0);
        assert_close(trim.x, cell.x + (cell.w - 100.0) / 2.0);
        assert_close(trim.y, cell.y + (cell.h - 50.0) / 2.0);
        let mark = fitted[0]
            .crop_marks
            .iter()
            .find(|mark| (mark.y1 - trim.y).abs() < 1e-6 && mark.x2 < trim.x)
            .unwrap();
        assert_close(mark.x2, trim.x - CROP_MARK_OFFSET_PT);

        let oversized = impose(
            &[SourceSize::page(100.0, 100.0).with_target(500.0, 40.0)],
            &spec,
        )
        .unwrap();
        let cell = oversized[0].cells[0];
        let trim = oversized[0].placements[0].trim;
        assert_close(trim.x, cell.x);
        assert_close(trim.y, cell.y);
        assert_close(trim.w, cell.w);
        assert_close(trim.h, cell.h);
        assert!(!target_fits_cell(cell.w, cell.h, 500.0, 40.0));
    }

    #[test]
    fn eight_page_signature_orders_fronts_then_backs() {
        let slots = signature_slots(8, 2, 2, 8);
        let pages: Vec<Option<usize>> = slots.iter().map(|slot| slot.map(|index| index + 1)).collect();
        assert_eq!(
            pages,
            vec![
                Some(8),
                Some(1),
                Some(6),
                Some(3),
                Some(2),
                Some(7),
                Some(4),
                Some(5),
            ]
        );

        let wide = signature_slots(8, 4, 1, 8);
        let wide_pages: Vec<Option<usize>> = wide.iter().map(|slot| slot.map(|index| index + 1)).collect();
        assert_eq!(
            wide_pages,
            vec![Some(8), Some(1), Some(6), Some(3), Some(2), Some(7), Some(4), Some(5)]
        );

        let grid = signature_slots(8, 4, 4, 8);
        let first_row: Vec<Option<usize>> = grid[..4].iter().map(|slot| slot.map(|index| index + 1)).collect();
        assert_eq!(first_row, vec![Some(8), Some(1), Some(6), Some(3)]);
        assert!(grid[4..16].iter().all(|slot| slot.is_none()));
        let second_row: Vec<Option<usize>> = grid[16..20]
            .iter()
            .map(|slot| slot.map(|index| index + 1))
            .collect();
        assert_eq!(second_row, vec![Some(2), Some(7), Some(4), Some(5)]);
    }

    #[test]
    fn signature_repeats_for_the_next_section() {
        let slots = signature_slots(16, 2, 2, 8);
        let second: Vec<usize> = slots[8..16].iter().map(|slot| slot.unwrap() + 1).collect();
        assert_eq!(second, vec![16, 9, 14, 11, 10, 15, 12, 13]);
    }

    #[test]
    fn spacing_warning_uses_the_target_trim() {
        let spec = SheetSpec {
            width_pt: 400.0,
            height_pt: 400.0,
            columns: 2,
            rows: 1,
            margin_top: 4.0,
            margin_right: 4.0,
            margin_bottom: 4.0,
            margin_left: 4.0,
            gutter_x: 4.0,
            gutter_y: 0.0,
            bleed_top: 0.0,
            bleed_right: 0.0,
            bleed_bottom: 0.0,
            bleed_left: 0.0,
            crop_marks: true,
            fit: FitMode::Contain,
            signature_pages: 0,
        };
        assert!(spacing_is_tight(&spec, &[]));
        assert!(spacing_is_tight(
            &spec,
            &[SourceSize::page(100.0, 100.0), SourceSize::page(100.0, 100.0)]
        ));
        let small = SourceSize::page(100.0, 100.0).with_target(40.0, 40.0);
        assert!(!spacing_is_tight(&spec, &[small, small]));
        let almost_cell = SourceSize::page(100.0, 100.0).with_target(180.0, 380.0);
        assert!(spacing_is_tight(&spec, &[almost_cell, almost_cell]));
    }
}
