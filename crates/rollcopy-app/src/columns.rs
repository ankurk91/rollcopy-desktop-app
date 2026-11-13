//! Tables with resizable columns, shared by the library list and the
//! transfers panel. The header and the rows are built by the same function
//! from the same widths, so they always line up. A column is resized by
//! dragging its right edge in the header; widths never push other columns
//! off-screen: each drag is limited so the columns after it keep their
//! minimum, and when the window gets narrower the drawn widths shrink from
//! the right.

use gpui_kit::base::h_flex;
use gpui_kit::component::resizable::{HandleEdge, resize_handle};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

pub(crate) struct Col {
    pub title: &'static str,
    pub width: f32,
    pub min: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TableKind {
    Library,
    Transfers,
}

pub(crate) struct Table {
    pub kind: TableKind,
    pub cols: &'static [Col],
    /// The column that takes leftover room (it isn't dragged itself).
    pub flex: usize,
    /// Fixed column before the resizable ones (icons, marks).
    pub lead: f32,
    /// Fixed column after them (row buttons), or 0.
    pub trail: f32,
}

/// Space inside a row before the lead column.
pub(crate) const ROW_PAD: f32 = 8.;
/// Each row and the header have a 1 px border (the focus ring).
const BORDER: f32 = 1.;

pub(crate) const LIBRARY: Table = Table {
    kind: TableKind::Library,
    cols: &[
        Col {
            title: "Name",
            width: 220.,
            min: 80.,
        },
        Col {
            title: "Taken",
            width: 150.,
            min: 80.,
        },
        Col {
            title: "Size",
            width: 110.,
            min: 60.,
        },
        Col {
            title: "File size",
            width: 90.,
            min: 60.,
        },
        Col {
            title: "Tags",
            width: 140.,
            min: 60.,
        },
    ],
    flex: 4,
    lead: 64.,
    trail: 0.,
};

pub(crate) const TRANSFERS: Table = Table {
    kind: TableKind::Transfers,
    cols: &[
        Col {
            title: "File",
            width: 150.,
            min: 80.,
        },
        Col {
            title: "Folder",
            width: 130.,
            min: 60.,
        },
        Col {
            title: "Status",
            width: 200.,
            min: 80.,
        },
        Col {
            title: "Finished",
            width: 120.,
            min: 80.,
        },
    ],
    flex: 2,
    lead: 22.,
    trail: 96.,
};

impl Table {
    /// Saved widths if they match this table, else the defaults.
    pub fn widths_from(&self, saved: &[f32]) -> Vec<f32> {
        if saved.len() == self.cols.len() {
            saved
                .iter()
                .zip(self.cols)
                .map(|(w, c)| w.max(c.min))
                .collect()
        } else {
            self.cols.iter().map(|c| c.width).collect()
        }
    }

    /// Room for the resizable columns in a row `row_w` wide.
    fn room(&self, row_w: f32) -> f32 {
        row_w - 2. * BORDER - ROW_PAD - self.lead - self.trail
    }

    /// Widths to draw in a row `row_w` wide: if the columns don't fit, the
    /// right-most ones give way first, down to their minimum.
    pub fn fit(&self, widths: &[f32], row_w: f32) -> Vec<f32> {
        let mut out = widths.to_vec();
        let mut excess = out.iter().sum::<f32>() - self.room(row_w);
        for (w, c) in out.iter_mut().zip(self.cols).rev() {
            if excess <= 0. {
                break;
            }
            let take = excess.min(*w - c.min).max(0.);
            *w -= take;
            excess -= take;
        }
        out
    }

    /// New width for column `col` when its edge is dragged to `x`, measured
    /// from the row's left edge. The columns after it keep at least their
    /// minimum inside the row.
    pub fn dragged(&self, shown: &[f32], col: usize, x: f32, row_w: f32) -> f32 {
        let before: f32 = shown[..col].iter().sum();
        let after_min: f32 = self.cols[col + 1..].iter().map(|c| c.min).sum();
        let start = BORDER + ROW_PAD + self.lead + before;
        let max = (self.room(row_w) - before - after_min).max(self.cols[col].min);
        (x - start).clamp(self.cols[col].min, max)
    }

    /// One row (header or item): lead, the columns, then the trail.
    pub fn row(
        &self,
        shown: &[f32],
        lead: AnyElement,
        cells: Vec<AnyElement>,
        trail: Option<AnyElement>,
    ) -> Div {
        let flex = self.flex;
        h_flex()
            .w_full()
            .pl(px(ROW_PAD))
            .items_center()
            .border(px(BORDER))
            .border_color(transparent_black())
            .child(
                h_flex()
                    .w(px(self.lead))
                    .flex_shrink_0()
                    .gap_1()
                    .child(lead),
            )
            .children(cells.into_iter().enumerate().map(|(i, cell)| {
                div()
                    .relative()
                    .h_full()
                    .flex()
                    .items_center()
                    .pr_2()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .map(|d| {
                        if i == flex {
                            d.flex_1().min_w(px(shown[i].min(self.cols[i].min)))
                        } else {
                            d.w(px(shown[i])).flex_shrink_0()
                        }
                    })
                    .child(cell)
            }))
            .when(self.trail > 0., |d| {
                d.child(
                    h_flex()
                        .w(px(self.trail))
                        .flex_shrink_0()
                        .justify_end()
                        .gap_1()
                        .pr_1()
                        .children(trail),
                )
            })
    }

    /// The header cells: titles, with a drag handle on each column's right
    /// edge (not on the column that takes the leftover room).
    pub fn header_cells(&self) -> Vec<AnyElement> {
        let kind = self.kind;
        self.cols
            .iter()
            .enumerate()
            .map(|(i, c)| {
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .child(c.title)
                    .when(i != self.flex, |d| {
                        d.child(
                            resize_handle::<ResizeColumn, ResizeColumn>(
                                (SharedString::from(format!("{kind:?}-edge")), i),
                                Axis::Horizontal,
                            )
                            .inside(HandleEdge::Trailing)
                            .on_drag(
                                ResizeColumn {
                                    table: kind,
                                    col: i,
                                },
                                |drag, _, _, cx| cx.new(|_| drag.clone()),
                            ),
                        )
                    })
                    .into_any_element()
            })
            .collect()
    }
}

/// Drag payload for a column edge.
#[derive(Clone)]
pub(crate) struct ResizeColumn {
    pub table: TableKind,
    pub col: usize,
}

impl Render for ResizeColumn {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

#[cfg(test)]
mod tests {
    use super::{LIBRARY, TRANSFERS};

    #[test]
    fn saved_widths_are_used_when_they_match() {
        let defaults: Vec<f32> = LIBRARY.cols.iter().map(|c| c.width).collect();
        assert_eq!(LIBRARY.widths_from(&[]), defaults);
        assert_eq!(LIBRARY.widths_from(&[1., 2.]), defaults);
        let w = LIBRARY.widths_from(&[300., 10., 120., 90., 140.]);
        assert_eq!(w, vec![300., 80., 120., 90., 140.]);
    }

    #[test]
    fn columns_never_run_off_the_row() {
        let w = vec![2000., 150., 110., 90., 140.];
        let row_w = 900.;
        let shown = LIBRARY.fit(&w, row_w);
        let room = row_w - 2. - 8. - 64.;
        assert!(shown.iter().sum::<f32>() <= room + 0.01, "{shown:?}");
        // The right-most columns gave way first, down to their minimum.
        assert_eq!(&shown[2..], &[60., 60., 60.]);
        // Narrow enough already: unchanged.
        assert_eq!(
            LIBRARY.fit(&[220., 150., 110., 90., 140.], 1200.),
            vec![220., 150., 110., 90., 140.]
        );
    }

    #[test]
    fn a_drag_leaves_room_for_the_columns_after() {
        let shown = LIBRARY.widths_from(&[]);
        let row_w = 900.;
        // Name starts after the border, padding and lead (73 px).
        assert_eq!(LIBRARY.dragged(&shown, 0, 73. + 250., row_w), 250.);
        // Dragged far past the right edge: stops where the rest still fit.
        let max = LIBRARY.dragged(&shown, 0, 5000., row_w);
        assert_eq!(max, (900. - 2. - 8. - 64.) - (80. + 60. + 60. + 60.));
        // Never below the minimum.
        assert_eq!(LIBRARY.dragged(&shown, 0, 0., row_w), 80.);
        // The trailing buttons column counts too.
        let t = TRANSFERS.widths_from(&[]);
        let max = TRANSFERS.dragged(&t, 0, 5000., 800.);
        assert_eq!(max, (800. - 2. - 8. - 22. - 96.) - (60. + 80. + 80.));
    }
}
