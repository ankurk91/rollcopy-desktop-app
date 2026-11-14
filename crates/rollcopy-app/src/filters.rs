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
}

impl Filter {
    pub(crate) const ALL: [Filter; 4] =
        [Filter::All, Filter::Photos, Filter::Videos, Filter::Edited];

    pub(crate) fn count_index(self) -> usize {
        self as usize
    }

    pub(crate) fn matches(self, a: &rollcopy_library::Asset) -> bool {
        match self {
            Filter::All => true,
            Filter::Photos => a.kind == MediaKind::Photo,
            Filter::Videos => a.kind == MediaKind::Video,
            Filter::Edited => a.is_edited(),
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
            self.filter_counts = [0; 4];
            self.sync_counts = [0; 3];
            return;
        };
        let mut counts = [0; 4];
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
        // By date taken; undated items last either way.
        let assets = self.assets();
        sort_by_date(assets, &mut visible, self.newest_first);
        self.visible = visible;
        self.selected.clear();
        self.anchor = None;
        self.focused = self.visible.first().copied();
        self.grid_scroll.scroll_to_item(0, ScrollStrategy::Top);
    }

    /// Whether the item with this device path is known to be copied.
    pub(crate) fn is_copied(&self, key: &str) -> bool {
        matches!(self.in_dest.get(key), Some(Some(_)))
    }
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

/// Orders item indices by date taken (newest or oldest first), then by
/// file name; undated items go last either way.
pub(crate) fn sort_by_date(
    assets: &[rollcopy_library::Asset],
    items: &mut [usize],
    newest_first: bool,
) {
    use std::cmp::Ordering;
    items.sort_by(|&i, &j| {
        let (a, b) = (&assets[i], &assets[j]);
        let by_date = match (a.captured, b.captured) {
            (Some(x), Some(y)) if newest_first => y.cmp(&x),
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        };
        by_date.then_with(|| a.filename.cmp(&b.filename))
    });
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that would bring in gpui's own `test` macro.
    use super::{Filter, Query, SyncFilter, sort_by_date};
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

    #[test]
    fn sorted_by_date_with_undated_last() {
        let assets = vec![
            item("IMG_0003.JPG", MediaKind::Photo, None),
            item("IMG_0001.JPG", MediaKind::Photo, Some(100)),
            item("IMG_0002.JPG", MediaKind::Photo, Some(200)),
            item("IMG_0004.JPG", MediaKind::Photo, Some(200)),
        ];
        let mut items = vec![0, 1, 2, 3];
        sort_by_date(&assets, &mut items, true);
        assert_eq!(items, vec![2, 3, 1, 0]);
        sort_by_date(&assets, &mut items, false);
        assert_eq!(items, vec![1, 2, 3, 0]);
    }
}
