//! Upstream `OnDemandPagedList(fetch_page, PAGE_SIZE)` as `PlaylistEntries`
//! reads it, for pending playlist entries: only the pages holding the
//! requested entries are fetched (cached, in request order), and reading
//! past the end reads every page (`get_full_count`'s `len`).
//!
//! The pending state keeps `random_access: true` (so the CLI attaches the
//! selection's `requests`, see
//! [`crate::playlist::PlaylistSelection::entry_requests`]) and the per-page
//! entry counts (`pages`, `null` for a page not fetched yet). Entries of
//! pages not fetched yet are placeholders the selection never picks.
use crate::{Result, VideoInfo};
use serde_json::{Value, json};

/// A page fetcher: the page's entries (fewer than the page size for the last
/// page).
pub(crate) type FetchPage<'a> = dyn FnMut(usize) -> Result<Vec<VideoInfo>> + 'a;

struct Paged<'a, 'b> {
    page_size: usize,
    fetch: &'b mut FetchPage<'a>,
    placeholder: &'b dyn Fn() -> Result<VideoInfo>,
    counts: Vec<Option<usize>>,
    entries: &'b mut Vec<VideoInfo>,
}

impl Paged<'_, '_> {
    /// `getpage(page)` (cached): its entries put in place of placeholders.
    fn page(&mut self, page: usize) -> Result<usize> {
        if let Some(Some(count)) = self.counts.get(page) {
            return Ok(*count);
        }
        let fetched = (self.fetch)(page)?;
        let count = fetched.len();
        let offset = self.page_size * page;
        // Pages before this one that were not fetched yet are full.
        while self.entries.len() < offset {
            self.entries.push((self.placeholder)()?);
        }
        if self.counts.len() <= page {
            self.counts.resize(page + 1, None);
        }
        let end = (offset + self.page_size).min(self.entries.len());
        if count < self.page_size {
            // The last page: nothing follows it.
            self.entries.truncate(offset);
            self.entries.extend(fetched);
            self.counts.truncate(page + 1);
        } else {
            self.entries.splice(offset..end, fetched);
        }
        self.counts[page] = Some(count);
        Ok(count)
    }

    /// `entries[index]`: whether the entry exists, after fetching its page.
    fn has(&mut self, index: usize) -> Result<bool> {
        Ok(index % self.page_size < self.page(index / self.page_size)?)
    }

    /// `len(entries)`: every page in order until one is shorter than the
    /// page size.
    fn read_all(&mut self) -> Result<()> {
        for page in 0.. {
            if self.page(page)? < self.page_size {
                return Ok(());
            }
        }
        Ok(())
    }

    fn exhausted(&self) -> bool {
        self.counts.iter().all(Option::is_some)
            && self
                .counts
                .last()
                .is_some_and(|count| count.is_some_and(|count| count < self.page_size))
    }
}

/// Read the pending `state`'s requested entries (or, without `requests`, the
/// first `limit` entries, or every page): the remaining state, or `None`
/// once every page is read.
pub(crate) fn fetch_requested(
    mut state: Value,
    limit: Option<usize>,
    entries: &mut Vec<VideoInfo>,
    page_size: usize,
    fetch: &mut FetchPage<'_>,
    placeholder: &dyn Fn() -> Result<VideoInfo>,
) -> Result<Option<Value>> {
    let requests = match state
        .as_object_mut()
        .and_then(|state| state.shift_remove("requests"))
    {
        Some(Value::Array(requests)) => requests,
        _ => match limit {
            Some(limit) => vec![json!({"start": 0, "stop": limit, "step": 1})],
            None => vec!["all".into()],
        },
    };
    let counts = state["pages"]
        .as_array()
        .map(|pages| {
            pages
                .iter()
                .map(|count| count.as_u64().map(|count| count as usize))
                .collect()
        })
        .unwrap_or_default();
    let mut paged = Paged {
        page_size,
        fetch,
        placeholder,
        counts,
        entries,
    };
    let mut exhausted = false;
    for request in &requests {
        if request == "all" {
            exhausted = true;
            break;
        }
        let step = request["step"].as_i64().unwrap_or(1);
        let stop = request["stop"].as_i64();
        let mut index = request["start"].as_i64().unwrap_or_default();
        while stop.is_none_or(|stop| if step > 0 { index < stop } else { index > stop }) {
            if index >= 0 && !paged.has(index as usize)? {
                exhausted = true;
                if step > 0 {
                    break;
                }
            }
            index += step;
            if index < 0 && step < 0 {
                break;
            }
        }
    }
    if exhausted {
        paged.read_all()?;
    }
    if paged.exhausted() {
        return Ok(None);
    }
    state["pages"] = paged.counts.into();
    Ok(Some(state))
}
