//! Grid rows grouped by month: a heading row ("November 2026 · 10") before
//! each month's tile rows. Pure data, so the layout and the ↑/↓ moves across
//! headings are unit-tested; `grid.rs` renders it.

/// One row of the grid. `start..end` are positions in the visible list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// A month heading; `month` is `None` for undated items.
    Header {
        month: Option<(i32, u32)>,
        start: usize,
        end: usize,
    },
    Tiles {
        start: usize,
        end: usize,
    },
}

/// Lays out `months` (the year/month of each visible item, in grid order)
/// in rows of `cols` tiles. With `grouped`, each run of the same month gets
/// a heading and starts on a new row; otherwise it's one continuous grid.
pub fn build(months: &[Option<(i32, u32)>], cols: usize, grouped: bool) -> Vec<Row> {
    let cols = cols.max(1);
    let mut rows = Vec::new();
    let push_tiles = |rows: &mut Vec<Row>, start: usize, end: usize| {
        let mut s = start;
        while s < end {
            let e = (s + cols).min(end);
            rows.push(Row::Tiles { start: s, end: e });
            s = e;
        }
    };
    if !grouped {
        push_tiles(&mut rows, 0, months.len());
        return rows;
    }
    let mut start = 0;
    while start < months.len() {
        let month = months[start];
        let end = start + months[start..].iter().take_while(|m| **m == month).count();
        rows.push(Row::Header { month, start, end });
        push_tiles(&mut rows, start, end);
        start = end;
    }
    rows
}

/// Index of the tile row holding visible position `pos`.
pub fn row_of(rows: &[Row], pos: usize) -> Option<usize> {
    rows.iter()
        .position(|r| matches!(r, Row::Tiles { start, end } if (*start..*end).contains(&pos)))
}

/// The position one tile row up (`dir < 0`) or down from `pos`, in the same
/// column where that row is long enough, skipping headings. `None` at the
/// top or bottom.
pub fn vertical(rows: &[Row], pos: usize, dir: i32) -> Option<usize> {
    let here = row_of(rows, pos)?;
    let Row::Tiles { start, .. } = rows[here] else {
        return None;
    };
    let col = pos - start;
    let mut i = here;
    loop {
        i = if dir < 0 { i.checked_sub(1)? } else { i + 1 };
        match rows.get(i)? {
            Row::Tiles { start, end } => return Some((start + col).min(end - 1)),
            Row::Header { .. } => continue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Row, build, row_of, vertical};

    const NOV: Option<(i32, u32)> = Some((2026, 11));
    const OCT: Option<(i32, u32)> = Some((2026, 10));

    #[test]
    fn months_get_headings_and_their_own_rows() {
        // 5 in November, 2 in October, 1 undated; 3 per row.
        let months = [NOV, NOV, NOV, NOV, NOV, OCT, OCT, None];
        let rows = build(&months, 3, true);
        assert_eq!(
            rows,
            vec![
                Row::Header {
                    month: NOV,
                    start: 0,
                    end: 5
                },
                Row::Tiles { start: 0, end: 3 },
                Row::Tiles { start: 3, end: 5 },
                Row::Header {
                    month: OCT,
                    start: 5,
                    end: 7
                },
                Row::Tiles { start: 5, end: 7 },
                Row::Header {
                    month: None,
                    start: 7,
                    end: 8
                },
                Row::Tiles { start: 7, end: 8 },
            ]
        );
        assert_eq!(build(&months, 3, false).len(), 3);
        assert!(build(&[], 3, true).is_empty());
    }

    #[test]
    fn up_and_down_skip_headings_and_keep_the_column() {
        let months = [NOV, NOV, NOV, NOV, NOV, OCT, OCT, None];
        let rows = build(&months, 3, true);
        assert_eq!(row_of(&rows, 4), Some(2));
        // Down from the first row's 3rd tile: the next row is shorter.
        assert_eq!(vertical(&rows, 2, 1), Some(4));
        // Down across the October heading, same column.
        assert_eq!(vertical(&rows, 4, 1), Some(6));
        assert_eq!(vertical(&rows, 3, 1), Some(5));
        // Up across a heading.
        assert_eq!(vertical(&rows, 5, -1), Some(3));
        assert_eq!(vertical(&rows, 7, -1), Some(5));
        // Edges.
        assert_eq!(vertical(&rows, 1, -1), None);
        assert_eq!(vertical(&rows, 7, 1), None);
    }
}
