//! Filter chips (Photos, Videos, Edited; Copied / Not copied), their
//! counts, and the list of items the grid shows.

use gpui_kit::*;
use rollcopy_library::MediaKind;

use crate::view::*;

/// Centre-pane filter chips (PLAN §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Filter {
    #[default]
    All,
    Photos,
    Videos,
    Edited,
    Favorites,
    Live,
    Screenshots,
    Selfies,
}

impl Filter {
    pub(crate) const ALL: [Filter; 8] = [
        Filter::All,
        Filter::Photos,
        Filter::Videos,
        Filter::Edited,
        Filter::Favorites,
        Filter::Live,
        Filter::Screenshots,
        Filter::Selfies,
    ];

    pub(crate) fn count_index(self) -> usize {
        self as usize
    }

    pub(crate) fn matches(self, a: &rollcopy_library::Asset) -> bool {
        match self {
            Filter::All => true,
            Filter::Photos => a.kind == MediaKind::Photo,
            Filter::Videos => a.kind == MediaKind::Video,
            Filter::Edited => a.is_edited(),
            Filter::Favorites => a.favorite,
            Filter::Live => a.live_photo,
            Filter::Screenshots => a.screenshot,
            Filter::Selfies => a.is_selfie(),
        }
    }

    /// Name shown in the chip's tooltip and the empty state.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Filter::All => "All",
            Filter::Photos => "Photos",
            Filter::Videos => "Videos",
            Filter::Edited => "Edited",
            Filter::Favorites => "Favourites",
            Filter::Live => "Live Photos",
            Filter::Screenshots => "Screenshots",
            Filter::Selfies => "Selfies",
        }
    }
}

/// Copied-state chips: is the item already in the Save-to folder?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum SyncFilter {
    #[default]
    All,
    Copied,
    NotCopied,
}

impl SyncFilter {
    pub(crate) fn matches(self, copied: bool) -> bool {
        match self {
            SyncFilter::All => true,
            SyncFilter::Copied => copied,
            SyncFilter::NotCopied => !copied,
        }
    }
}

impl MainView {
    /// Rebuilds the grid's item list after the tree selection, filter or
    /// library changed. Clears the multi-selection (it belongs to a view).
    /// Rebuilds the chip counts for the current tree selection. Copied state
    /// comes from [`MainView::refresh_copied`]; items not checked yet count
    /// as not copied.
    pub(crate) fn recount(&mut self) {
        let (sel, filter) = (self.selection, self.filter);
        let show_hidden = self.config.library.show_hidden;
        let LibState::Loaded { library, tree } = &self.library else {
            self.filter_counts = [0; Filter::ALL.len()];
            self.sync_counts = [0; 3];
            return;
        };
        let mut counts = [0; Filter::ALL.len()];
        let mut sync = [0; 3];
        let query = Query::new(&self.query);
        for a in library
            .assets
            .iter()
            .filter(|a| tree.contains(sel, a, show_hidden) && query.matches(a))
        {
            for (n, f) in counts.iter_mut().zip(Filter::ALL) {
                if f.matches(a) {
                    *n += 1;
                }
            }
            if !filter.matches(a) {
                continue;
            }
            let copied = matches!(self.in_dest.get(&a.device_path()), Some(Some(_)));
            sync[0] += 1;
            sync[if copied { 1 } else { 2 }] += 1;
        }
        self.filter_counts = counts;
        self.sync_counts = sync;
        self.new_count = if self.copied_for.is_some() {
            library
                .assets
                .iter()
                .filter(|a| is_new(a, &self.in_dest))
                .count()
        } else {
            0
        };
        self.counts_dirty = false;
    }

    pub(crate) fn recompute_visible(&mut self) {
        self.recount();
        let (sel, filter, sync) = (self.selection, self.filter, self.sync_filter);
        let show_hidden = self.config.library.show_hidden;
        let LibState::Loaded { library, tree } = &self.library else {
            self.visible.clear();
            return;
        };
        let query = Query::new(&self.query);
        let mut visible: Vec<usize> = library
            .assets
            .iter()
            .enumerate()
            .filter(|(_, a)| {
                tree.contains(sel, a, show_hidden)
                    && query.matches(a)
                    && filter.matches(a)
                    && sync.matches(self.is_copied(&a.device_path()))
            })
            .map(|(i, _)| i)
            .collect();
        let assets = self.assets();
        sort_items(assets, &mut visible, self.sort);
        self.visible = visible;
        self.selected.clear();
        self.anchor = None;
        self.focused = self.visible.first().copied();
        self.visible_gen += 1;
        self.grid_scroll.scroll_to_item(0, ScrollStrategy::Top);
    }

    /// Whether the item with this device path is known to be copied.
    pub(crate) fn is_copied(&self, key: &str) -> bool {
        matches!(self.in_dest.get(key), Some(Some(_)))
    }
}

/// Not copied yet, and allowed in "Copy new": no Hidden or Recently Deleted
/// items, and only once the copied-state scan has an answer for it.
pub(crate) fn is_new(
    a: &rollcopy_library::Asset,
    in_dest: &std::collections::HashMap<String, Option<std::path::PathBuf>>,
) -> bool {
    !a.trashed && !a.hidden && matches!(in_dest.get(&a.device_path()), Some(None))
}

/// The search box: every word must appear (case-insensitive) in the file
/// name, place, an album, the app that saved it or the camera. Empty
/// matches everything.
pub(crate) struct Query(Vec<String>);

impl Query {
    pub(crate) fn new(text: &str) -> Self {
        Self(text.split_whitespace().map(str::to_lowercase).collect())
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn matches(&self, a: &rollcopy_library::Asset) -> bool {
        if self.0.is_empty() {
            return true;
        }
        let mut hay = a.filename.to_lowercase();
        for extra in [&a.place, &a.source_app, &a.camera].into_iter().flatten() {
            hay.push('\n');
            hay.push_str(&extra.to_lowercase());
        }
        for album in &a.albums {
            hay.push('\n');
            hay.push_str(&album.to_lowercase());
        }
        self.0.iter().all(|w| hay.contains(w.as_str()))
    }
}

/// What the grid is sorted by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum SortKey {
    /// Date taken (the only order with month headings).
    #[default]
    Date,
    Name,
    /// File size on the phone.
    Size,
    /// File type: the extension (HEIC, JPG, MOV, PNG, …).
    Type,
}

impl SortKey {
    pub(crate) const ALL: [SortKey; 4] =
        [SortKey::Date, SortKey::Name, SortKey::Size, SortKey::Type];

    pub(crate) fn label(self) -> &'static str {
        match self {
            SortKey::Date => "Date taken",
            SortKey::Name => "Name",
            SortKey::Size => "Size",
            SortKey::Type => "Type",
        }
    }
}

/// A sort key and direction. The default is newest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Sort {
    pub(crate) key: SortKey,
    pub(crate) descending: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Sort {
            key: SortKey::Date,
            descending: true,
        }
    }
}

impl Sort {
    /// The two directions in words, (descending, ascending).
    pub(crate) fn direction_labels(self) -> (&'static str, &'static str) {
        match self.key {
            SortKey::Date => ("Newest first", "Oldest first"),
            SortKey::Name | SortKey::Type => ("Z to A", "A to Z"),
            SortKey::Size => ("Largest first", "Smallest first"),
        }
    }

    /// The direction in words ("Newest first", "A to Z").
    pub(crate) fn direction(self) -> &'static str {
        let (desc, asc) = self.direction_labels();
        if self.descending { desc } else { asc }
    }

    /// Short label for the sort button: the direction for dates, the key
    /// otherwise (the button's arrow shows which way).
    pub(crate) fn label(self) -> &'static str {
        match self.key {
            SortKey::Date => self.direction(),
            k => k.label(),
        }
    }

    /// Month headings only make sense when items are in date order.
    pub(crate) fn groups_by_month(self) -> bool {
        self.key == SortKey::Date
    }
}

/// Upper-case extension of a file name ("HEIC"), empty if none.
fn file_type(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, e)| e.to_ascii_uppercase())
        .unwrap_or_default()
}

/// Orders item indices by `sort`. Items without the value sorted on
/// (undated, unknown size) go last either way; ties go newest first, then
/// by file name.
pub(crate) fn sort_items(assets: &[rollcopy_library::Asset], items: &mut [usize], sort: Sort) {
    use std::cmp::Ordering;
    // Present values first, then compare them in the chosen direction.
    fn opt<T: Ord>(a: Option<T>, b: Option<T>, descending: bool) -> Ordering {
        match (a, b) {
            (Some(x), Some(y)) if descending => y.cmp(&x),
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        }
    }
    items.sort_by(|&i, &j| {
        let (a, b) = (&assets[i], &assets[j]);
        let primary = match sort.key {
            SortKey::Date => opt(a.captured, b.captured, sort.descending),
            SortKey::Size => opt(a.original_size, b.original_size, sort.descending),
            SortKey::Name => {
                let o = a.filename.to_lowercase().cmp(&b.filename.to_lowercase());
                if sort.descending { o.reverse() } else { o }
            }
            SortKey::Type => {
                let o = file_type(&a.filename).cmp(&file_type(&b.filename));
                if sort.descending { o.reverse() } else { o }
            }
        };
        primary
            .then_with(|| opt(a.captured, b.captured, true))
            .then_with(|| a.filename.cmp(&b.filename))
    });
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that would bring in gpui's own `test` macro.
    use super::{Filter, Query, Sort, SortKey, SyncFilter, sort_items};
    use chrono::{TimeZone, Utc};
    use rollcopy_library::{Asset, MediaKind};

    fn item(name: &str, kind: MediaKind, secs: Option<i64>) -> Asset {
        let mut a = Asset::new("DCIM/100APPLE", name, kind);
        a.captured = secs.map(|s| Utc.timestamp_opt(s, 0).unwrap());
        a
    }

    #[test]
    fn chips_match_their_items() {
        let photo = item("IMG_0001.HEIC", MediaKind::Photo, Some(1));
        let mut video = item("IMG_0002.MOV", MediaKind::Video, Some(2));
        video.edited_render = Some("/x/FullSizeRender.mov".into());
        assert!(Filter::Photos.matches(&photo) && !Filter::Photos.matches(&video));
        assert!(Filter::Videos.matches(&video) && !Filter::Videos.matches(&photo));
        assert!(Filter::Edited.matches(&video) && !Filter::Edited.matches(&photo));
        assert!(Filter::All.matches(&photo) && Filter::All.matches(&video));
        assert!(SyncFilter::Copied.matches(true) && !SyncFilter::Copied.matches(false));
        assert!(SyncFilter::NotCopied.matches(false) && SyncFilter::All.matches(true));
    }

    #[test]
    fn search_matches_every_word_anywhere() {
        let mut a = item("IMG_0042.HEIC", MediaKind::Photo, Some(1));
        a.place = Some("1 Example Street, Springfield".into());
        a.albums = vec!["Holiday 2026".into()];
        a.source_app = Some("Snapchat".into());
        assert!(Query::new("").matches(&a));
        assert!(Query::new("img_0042").matches(&a));
        assert!(Query::new("0042 heic").matches(&a));
        assert!(Query::new("springfield").matches(&a));
        assert!(Query::new("holiday snap").matches(&a));
        assert!(!Query::new("holiday whatsapp").matches(&a));
        assert!(!Query::new("IMG_0043").matches(&a));
        assert!(Query::new("   ").is_empty());
    }

    fn sort(key: SortKey, descending: bool) -> Sort {
        Sort { key, descending }
    }

    #[test]
    fn sorted_by_date_with_undated_last() {
        let assets = vec![
            item("IMG_0003.JPG", MediaKind::Photo, None),
            item("IMG_0001.JPG", MediaKind::Photo, Some(100)),
            item("IMG_0002.JPG", MediaKind::Photo, Some(200)),
            item("IMG_0004.JPG", MediaKind::Photo, Some(200)),
        ];
        let mut items = vec![0, 1, 2, 3];
        sort_items(&assets, &mut items, Sort::default());
        assert_eq!(items, vec![2, 3, 1, 0]);
        sort_items(&assets, &mut items, sort(SortKey::Date, false));
        assert_eq!(items, vec![1, 2, 3, 0]);
    }

    #[test]
    fn sorted_by_size_with_unknown_last() {
        let mut assets = vec![
            item("IMG_0001.JPG", MediaKind::Photo, Some(100)),
            item("IMG_0002.JPG", MediaKind::Photo, Some(200)),
            item("IMG_0003.JPG", MediaKind::Photo, Some(300)),
            item("IMG_0004.JPG", MediaKind::Photo, Some(400)),
        ];
        assets[0].original_size = Some(5_000);
        assets[1].original_size = Some(9_000);
        assets[3].original_size = Some(5_000);
        let mut items = vec![0, 1, 2, 3];
        sort_items(&assets, &mut items, sort(SortKey::Size, true));
        // Equal sizes: newer first.
        assert_eq!(items, vec![1, 3, 0, 2]);
        sort_items(&assets, &mut items, sort(SortKey::Size, false));
        assert_eq!(items, vec![3, 0, 1, 2]);
    }

    #[test]
    fn sorted_by_type_then_newest() {
        let assets = vec![
            item("IMG_0001.MOV", MediaKind::Video, Some(100)),
            item("IMG_0002.heic", MediaKind::Photo, Some(200)),
            item("IMG_0003.JPG", MediaKind::Photo, Some(300)),
            item("IMG_0004.HEIC", MediaKind::Photo, Some(400)),
        ];
        let mut items = vec![0, 1, 2, 3];
        sort_items(&assets, &mut items, sort(SortKey::Type, false));
        assert_eq!(items, vec![3, 1, 2, 0]);
        sort_items(&assets, &mut items, sort(SortKey::Name, true));
        assert_eq!(items, vec![3, 2, 1, 0]);
    }

    #[test]
    fn only_date_order_has_month_headings() {
        assert!(Sort::default().groups_by_month());
        assert!(!sort(SortKey::Name, false).groups_by_month());
        assert_eq!(Sort::default().label(), "Newest first");
        assert_eq!(sort(SortKey::Size, false).label(), "Size");
        assert_eq!(sort(SortKey::Size, false).direction(), "Smallest first");
    }
}
