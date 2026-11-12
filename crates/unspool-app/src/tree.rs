//! Library tree model: All media → year → month, plus Recently Deleted
//! (PLAN §6, left pane). Pure data; the view renders it.

use std::collections::BTreeMap;

use unspool_library::Asset;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Selection {
    #[default]
    All,
    Year(i32),
    Month(i32, u32),
    Undated,
    RecentlyDeleted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YearNode {
    pub year: i32,
    pub count: usize,
    /// (month 1–12, count), newest first.
    pub months: Vec<(u32, usize)>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibraryTree {
    pub all: usize,
    /// Newest year first.
    pub years: Vec<YearNode>,
    pub undated: usize,
    pub recently_deleted: usize,
}

impl LibraryTree {
    pub fn build(assets: &[Asset]) -> Self {
        let mut by_year: BTreeMap<i32, BTreeMap<u32, usize>> = BTreeMap::new();
        let mut t = LibraryTree::default();
        for a in assets {
            if a.trashed {
                t.recently_deleted += 1;
                continue;
            }
            t.all += 1;
            match a.year_month() {
                Some((y, m)) => *by_year.entry(y).or_default().entry(m).or_default() += 1,
                None => t.undated += 1,
            }
        }
        t.years = by_year
            .into_iter()
            .rev()
            .map(|(year, months)| YearNode {
                year,
                count: months.values().sum(),
                months: months.into_iter().rev().collect(),
            })
            .collect();
        t
    }

    /// Whether an asset belongs to a selection.
    pub fn contains(sel: Selection, a: &Asset) -> bool {
        match sel {
            Selection::RecentlyDeleted => a.trashed,
            _ if a.trashed => false,
            Selection::All => true,
            Selection::Year(y) => a.year_month().is_some_and(|(ay, _)| ay == y),
            Selection::Month(y, m) => a.year_month() == Some((y, m)),
            Selection::Undated => a.year_month().is_none(),
        }
    }
}

pub fn month_name(m: u32) -> &'static str {
    const NAMES: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    NAMES
        .get(m.wrapping_sub(1) as usize)
        .copied()
        .unwrap_or("?")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use unspool_library::MediaKind;

    fn asset(y: i32, m: u32, trashed: bool) -> Asset {
        let mut a = Asset::new("DCIM/100APPLE", "IMG_1.JPG", MediaKind::Photo);
        a.captured = Some(Utc.with_ymd_and_hms(y, m, 15, 12, 0, 0).unwrap());
        a.trashed = trashed;
        a
    }

    #[test]
    fn builds_newest_first_and_excludes_trash() {
        let mut undated = Asset::new("DCIM/100APPLE", "IMG_2.JPG", MediaKind::Photo);
        undated.captured = None;
        let assets = vec![
            asset(2025, 9, false),
            asset(2026, 10, false),
            asset(2026, 11, false),
            asset(2026, 11, false),
            asset(2026, 11, true),
            undated.clone(),
        ];
        let t = LibraryTree::build(&assets);
        assert_eq!(t.all, 5);
        assert_eq!(t.recently_deleted, 1);
        assert_eq!(t.undated, 1);
        assert_eq!(t.years[0].year, 2026);
        assert_eq!(t.years[0].count, 3);
        assert_eq!(t.years[0].months, vec![(11, 2), (10, 1)]);
        assert_eq!(t.years[1].months, vec![(9, 1)]);

        assert!(LibraryTree::contains(
            Selection::Month(2026, 11),
            &assets[2]
        ));
        assert!(!LibraryTree::contains(Selection::All, &assets[4]));
        assert!(LibraryTree::contains(
            Selection::RecentlyDeleted,
            &assets[4]
        ));
        assert!(LibraryTree::contains(Selection::Undated, &undated));
    }

    #[test]
    fn month_names() {
        assert_eq!(month_name(9), "September");
        assert_eq!(month_name(0), "?");
    }
}
