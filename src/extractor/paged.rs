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
//!
//! A page whose fetch fails ends the list before it (`_getslice` sets
//! `_pagecount = pagenum - 1`), and the entry being read is `None` after
//! `_handle_extraction_exceptions` reported the error: a
//! [`crate::FETCH_ERROR`] entry in its place, carrying the list's length
//! without it (`fetch_error_count`).
use crate::{Error, Result, VideoInfo};
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
    /// The index of the entry a failed page fetch left `None`.
    failed: Option<usize>,
}

/// A page's fetch error, before it is put in place of the entry being read.
struct PageError {
    page: usize,
    error: Error,
}

impl Paged<'_, '_> {
    /// `getpage(page)` (cached): its entries put in place of placeholders.
    fn page(&mut self, page: usize) -> Result<std::result::Result<usize, PageError>> {
        if let Some(Some(count)) = self.counts.get(page) {
            return Ok(Ok(*count));
        }
        let fetched = match (self.fetch)(page) {
            Ok(fetched) => fetched,
            Err(error) => return Ok(Err(PageError { page, error })),
        };
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
            if self.failed.is_some_and(|failed| failed >= offset) {
                self.failed = None;
            }
        } else {
            self.entries.splice(offset..end, fetched);
        }
        self.counts[page] = Some(count);
        Ok(Ok(count))
    }

    /// `_pagecount = pagenum - 1` for the failed page, and the entry at
    /// `index` (on that page) `None`: its error entry.
    fn fail(&mut self, index: usize, PageError { page, error }: PageError) -> Result<()> {
        if self.failed.is_some() {
            // A second failure (re-reading for the full count) is not
            // reported in its entry's place.
            return Err(error);
        }
        let offset = self.page_size * page;
        if self.counts.len() <= page {
            self.counts.resize(page + 1, None);
        }
        self.counts.truncate(page + 1);
        self.counts[page] = Some(0);
        while self.entries.len() < index {
            self.entries.push((self.placeholder)()?);
        }
        self.entries.truncate(index);
        let mut entry = (self.placeholder)()?;
        entry
            .extra
            .insert(crate::FETCH_ERROR.into(), error.to_string().into());
        entry
            .extra
            .insert(crate::FETCH_ERROR_COUNT.into(), offset.into());
        self.entries.push(entry);
        self.failed = Some(index);
        Ok(())
    }

    /// `entries[index]`: whether the entry exists (a failed fetch's `None`
    /// does), after fetching its page.
    fn has(&mut self, index: usize) -> Result<bool> {
        let page = index / self.page_size;
        if let Some(failed) = self.failed
            && failed / self.page_size == page
            && index < failed
        {
            // The placeholders before the failed entry would be picked.
            return Err(Error::UnsupportedHtml5(
                "selecting entries before a failed page's entry is not supported natively".into(),
            ));
        }
        match self.page(page)? {
            Ok(count) => Ok(index % self.page_size < count),
            Err(error) => {
                self.fail(index, error)?;
                Ok(true)
            }
        }
    }

    /// `len(entries)`: every page in order until one is shorter than the
    /// page size.
    fn read_all(&mut self) -> Result<()> {
        for page in 0.. {
            match self.page(page)? {
                Ok(count) if count < self.page_size => return Ok(()),
                Ok(_) => {}
                Err(error) => {
                    // Read in order: the page's first entry is the one read.
                    let index = self.page_size * page;
                    return self.fail(index, error);
                }
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
        failed: state["failed"].as_u64().map(|index| index as usize),
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
    // A failed page ends the list, but its full count is known only once a
    // read went past the failed entry (`is_exhausted`).
    if paged.exhausted() && (exhausted || paged.failed.is_none()) {
        return Ok(None);
    }
    state["pages"] = paged.counts.into();
    if let Some(failed) = paged.failed {
        state["failed"] = failed.into();
    }
    Ok(Some(state))
}

/// A failed page fetch's entry: its error (see [`crate::FETCH_ERROR`]).
pub(crate) fn fetch_error(entry: &VideoInfo) -> Option<&str> {
    entry.extra.get(crate::FETCH_ERROR).and_then(Value::as_str)
}

/// The paged list's length once a failed page ended it (the count of the
/// pages before it), from its failed entry.
pub(crate) fn fetch_error_count(entries: &[VideoInfo]) -> Option<usize> {
    entries.iter().find_map(|entry| {
        entry
            .extra
            .get(crate::FETCH_ERROR_COUNT)
            .and_then(Value::as_u64)
            .map(|count| count as usize)
    })
}
