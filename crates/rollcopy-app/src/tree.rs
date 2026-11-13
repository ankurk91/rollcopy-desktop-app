//! Library tree model: All media → year → month, plus Hidden and Recently
//! Deleted (PLAN §6, left pane). Pure data; the view renders it.
//!
//! Items in the phone's Hidden album are never part of All media or the
//! year/month nodes. iOS locks that album behind Face ID or the passcode, so
//! they only appear, under their own node, when the user turns on "Show
//! Hidden album" (audit SEC-01). With it off they aren't counted or listed
//! anywhere, Recently Deleted included.

use std::collections::BTreeMap;

use rollcopy_library::Asset;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Selection {
    #[default]
    All,
    Year(i32),
    Month(i32, u32),
    Undated,
    /// An album, by index into [`LibraryTree::albums`].
    Album(usize),
    Hidden,
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
    /// Hidden-album items; 0 unless they are shown.
    pub hidden: usize,
    /// User albums from the Photos database (name, items), sorted by name.
    /// Hidden and Recently Deleted items aren't counted or listed in them.
    pub albums: Vec<(String, usize)>,
    pub recently_deleted: usize,
}

impl LibraryTree {
    pub fn build(assets: &[Asset], show_hidden: bool) -> Self {
        let mut by_year: BTreeMap<i32, BTreeMap<u32, usize>> = BTreeMap::new();
        let mut albums: BTreeMap<(String, String), usize> = BTreeMap::new();
        let mut t = LibraryTree::default();
        for a in assets {
            if a.hidden && !show_hidden {
                continue;
            }
            if a.trashed {
                t.recently_deleted += 1;
                continue;
            }
            if a.hidden {
                t.hidden += 1;
                continue;
            }
            t.all += 1;
            for name in &a.albums {
                *albums
                    .entry((name.to_lowercase(), name.clone()))
                    .or_default() += 1;
            }
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
        t.albums = albums.into_iter().map(|((_, name), n)| (name, n)).collect();
        t
    }

    /// The album an `Album` selection points at, by name.
    pub fn album_index(&self, name: &str) -> Option<usize> {
        self.albums.iter().position(|(n, _)| n == name)
    }

    /// Whether an asset belongs to a selection.
    pub fn contains(&self, sel: Selection, a: &Asset, show_hidden: bool) -> bool {
        if a.hidden && !show_hidden {
            return false;
        }
        match sel {
            Selection::RecentlyDeleted => a.trashed,
            _ if a.trashed => false,
            Selection::Hidden => a.hidden,
            _ if a.hidden => false,
            Selection::All => true,
            Selection::Year(y) => a.year_month().is_some_and(|(ay, _)| ay == y),
            Selection::Month(y, m) => a.year_month() == Some((y, m)),
            Selection::Undated => a.year_month().is_none(),
            Selection::Album(i) => self
                .albums
                .get(i)
                .is_some_and(|(name, _)| a.albums.iter().any(|n| n == name)),
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
    use rollcopy_library::MediaKind;

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
        let t = LibraryTree::build(&assets, false);
        assert_eq!(t.all, 5);
        assert_eq!(t.recently_deleted, 1);
        assert_eq!(t.undated, 1);
        assert_eq!(t.years[0].year, 2026);
        assert_eq!(t.years[0].count, 3);
        assert_eq!(t.years[0].months, vec![(11, 2), (10, 1)]);
        assert_eq!(t.years[1].months, vec![(9, 1)]);

        assert!(t.contains(Selection::Month(2026, 11), &assets[2], false));
        assert!(!t.contains(Selection::All, &assets[4], false));
        assert!(t.contains(Selection::RecentlyDeleted, &assets[4], false));
        assert!(t.contains(Selection::Undated, &undated, false));
    }

    #[test]
    fn hidden_items_stay_out_unless_shown() {
        let mut hidden = asset(2026, 11, false);
        hidden.hidden = true;
        let mut hidden_trashed = asset(2026, 11, true);
        hidden_trashed.hidden = true;
        let assets = vec![
            asset(2026, 11, false),
            hidden.clone(),
            hidden_trashed.clone(),
        ];

        let t = LibraryTree::build(&assets, false);
        assert_eq!((t.all, t.hidden, t.recently_deleted), (1, 0, 0));
        assert_eq!(t.years[0].months, vec![(11, 1)]);
        for sel in [
            Selection::All,
            Selection::Year(2026),
            Selection::Month(2026, 11),
            Selection::Hidden,
            Selection::RecentlyDeleted,
        ] {
            assert!(!t.contains(sel, &hidden, false), "{sel:?}");
            assert!(!t.contains(sel, &hidden_trashed, false), "{sel:?}");
        }

        // Shown: only under Hidden (and Recently Deleted once trashed),
        // never in All media, so "Copy all" never picks them up.
        let t = LibraryTree::build(&assets, true);
        assert_eq!((t.all, t.hidden, t.recently_deleted), (1, 1, 1));
        assert!(t.contains(Selection::Hidden, &hidden, true));
        assert!(!t.contains(Selection::All, &hidden, true));
        assert!(!t.contains(Selection::Month(2026, 11), &hidden, true));
        assert!(t.contains(Selection::RecentlyDeleted, &hidden_trashed, true));
        assert!(!t.contains(Selection::Hidden, &hidden_trashed, true));
    }

    #[test]
    fn albums_list_their_items_but_not_hidden_or_deleted_ones() {
        let mut beach = asset(2026, 11, false);
        beach.albums = vec!["Beach".into(), "favourites".into()];
        let mut trip = asset(2026, 10, false);
        trip.albums = vec!["Beach".into()];
        let mut secret = asset(2026, 10, false);
        secret.hidden = true;
        secret.albums = vec!["Beach".into()];
        let mut gone = asset(2026, 10, true);
        gone.albums = vec!["Beach".into()];
        let assets = vec![beach.clone(), trip, secret.clone(), gone];

        let t = LibraryTree::build(&assets, true);
        // Sorted by name, ignoring case.
        assert_eq!(
            t.albums,
            vec![("Beach".to_owned(), 2), ("favourites".to_owned(), 1)]
        );
        let i = t.album_index("Beach").unwrap();
        assert!(t.contains(Selection::Album(i), &beach, true));
        assert!(!t.contains(Selection::Album(i), &secret, true));
        assert!(!t.contains(Selection::Album(99), &beach, true));
    }

    #[test]
    fn month_names() {
        assert_eq!(month_name(9), "September");
        assert_eq!(month_name(0), "?");
    }
}
