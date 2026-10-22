//! Upstream `YoutubeIE._get_comments` (`--write-comments`): the watch page's
//! comment section through the `next` API, run as the video's
//! `__post_extractor` after the match filters.
use super::dict::{get_count, parse_count, parse_time_text};
use super::metadata::text as get_text;
use super::tab::{at, continuation_query, extract_continuation, ucid_or_none};
use super::watch;
use crate::{Error, Result};
use base64::Engine;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

/// What `extract_comments(ytcfg, video_id, contents, webpage)` captured.
struct Pending {
    ytcfg: Map<String, Value>,
    contents: Value,
}

/// Comment sections awaiting `post_extract`, by video ID.
static PENDING: Mutex<Option<HashMap<String, Pending>>> = Mutex::new(None);

/// Keep a video's comment section for [`post_extract`].
pub(super) fn defer(video_id: &str, ytcfg: Map<String, Value>, contents: Value) {
    if let Ok(mut pending) = PENDING.lock() {
        pending
            .get_or_insert_with(HashMap::new)
            .insert(video_id.to_owned(), Pending { ytcfg, contents });
    }
}

/// `YoutubeDL.post_extract`: the video's `comments` and `comment_count`, when
/// its extraction deferred them.
pub(crate) fn post_extract(client: &Client, info: &mut crate::VideoInfo) -> Result<()> {
    let pending = PENDING
        .lock()
        .ok()
        .and_then(|mut pending| pending.as_mut()?.remove(&info.id));
    let Some(pending) = pending else {
        return Ok(());
    };
    let (comments, count) = extract(client, &info.id, &pending)?;
    crate::metadata_parser::set_field(info, "comments", comments);
    crate::metadata_parser::set_field(info, "comment_count", count);
    Ok(())
}

/// `_generate_comment_continuation`.
fn generated_continuation(video_id: &str) -> String {
    let token = format!(
        "\x12\r\x12\x0b{video_id}\x18\x062'\"\x11\"\x0b{video_id}0\x00x\x020\x00B\x10comments-section"
    );
    base64::engine::general_purpose::STANDARD.encode(token)
}

/// The `max_comments` extractor argument: total, parents, replies, replies
/// per thread and depth (unlimited by default).
struct Limits {
    comments: usize,
    parents: usize,
    replies: usize,
    replies_per_thread: usize,
    depth: usize,
}

fn limits() -> Limits {
    let values = crate::manifest_args::configuration_arg("youtube", "max_comments", false)
        .unwrap_or_default();
    let value = |index: usize| {
        values
            .get(index)
            .and_then(|value| value.trim().parse::<i64>().ok())
            .map_or(usize::MAX, |value| usize::try_from(value).unwrap_or(0))
    };
    Limits {
        comments: value(0),
        parents: value(1),
        replies: value(2),
        replies_per_thread: value(3),
        depth: value(4),
    }
}

/// `_comment_entries`' tracker.
#[derive(Default)]
struct Tracker {
    running_total: usize,
    est_total: Option<i64>,
    current_page_thread: usize,
    total_parent_comments: usize,
    total_reply_comments: usize,
    seen: HashSet<String>,
    pinned: HashSet<String>,
}

/// A generator's early end: `return` from the level it happened in, an
/// `itertools.islice` (by its level) that has its items, `CommentsDisabled`
/// or an error.
enum Stop {
    Return,
    Slice(usize),
    Disabled,
    Error(Error),
}

struct Extraction<'a> {
    client: &'a Client,
    video_id: &'a str,
    ytcfg: &'a Map<String, Value>,
    limits: Limits,
    sort_new: bool,
    tracker: Tracker,
    comments: Vec<Value>,
    /// The remaining items of the enclosing `islice`s, outermost first.
    slices: Vec<usize>,
}

impl Extraction<'_> {
    /// `yield comment` through every enclosing `islice`.
    fn emit(&mut self, comment: Value) -> std::result::Result<(), Stop> {
        self.comments.push(comment);
        for remaining in &mut self.slices {
            *remaining = remaining.saturating_sub(1);
        }
        match self.slices.iter().position(|remaining| *remaining == 0) {
            Some(level) => Err(Stop::Slice(level)),
            None => Ok(()),
        }
    }

    /// `extract_header`: the expected count and the sort's continuation.
    fn extract_header(&mut self, contents: &[Value]) -> Option<Map<String, Value>> {
        for content in contents {
            let header = content.get("commentsHeaderRenderer");
            let expected = header.and_then(|header| {
                get_count(header.get("countText"))
                    .or_else(|| get_count(header.get("commentsCount")))
            });
            if let Some(expected) = expected {
                self.tracker.est_total = Some(expected);
                crate::screen::screen(&format!("[youtube] Downloading ~{expected} comments"));
            }
            let index = usize::from(self.sort_new);
            let item = header
                .and_then(|header| {
                    at(
                        header,
                        &["sortMenu", "sortFilterSubMenuRenderer", "subMenuItems"],
                    )?
                    .as_array()?
                    .get(index)
                })
                .filter(|item| item.is_object())
                .cloned()
                .unwrap_or_else(|| json!({}));
            let endpoint = item
                .get("serviceEndpoint")
                .cloned()
                .unwrap_or_else(|| json!({}));
            // `_extract_continuation_ep_data`, else `_extract_continuation`.
            let continuation = at(&endpoint, &["continuationCommand", "token"])
                .and_then(Value::as_str)
                .filter(|token| !token.is_empty())
                .map(|token| {
                    continuation_query(
                        token,
                        endpoint.get("clickTrackingParams").and_then(Value::as_str),
                    )
                })
                .or_else(|| extract_continuation(&item));
            let Some(continuation) = continuation else {
                continue;
            };
            let sort = item
                .get("title")
                .and_then(Value::as_str)
                .filter(|title| !title.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    if self.sort_new {
                        "newest first"
                    } else {
                        "top comments"
                    }
                    .to_owned()
                });
            crate::screen::screen(&format!(
                "[youtube] Sorting comments by {}",
                sort.to_lowercase()
            ));
            return Some(continuation);
        }
        None
    }

    /// `extract_thread`: the comments of `contents` and their replies.
    fn extract_thread(
        &mut self,
        contents: &[Value],
        entities: &[Value],
        parent: Option<&str>,
        depth: usize,
    ) -> std::result::Result<(), Stop> {
        if parent.is_none() {
            self.tracker.current_page_thread = 0;
        }
        if self.limits.depth < depth {
            return Ok(());
        }
        for content in contents {
            if parent.is_none() && self.tracker.total_parent_comments >= self.limits.parents {
                return Err(Stop::Return);
            }
            let thread = content.get("commentThreadRenderer");
            let comment = if entities.is_empty() {
                // The old comment format.
                let renderer = thread
                    .and_then(|thread| {
                        thread
                            .get("commentRenderer")
                            .or_else(|| at(thread, &["comment", "commentRenderer"]))
                    })
                    .or_else(|| {
                        content
                            .get("commentRenderer")
                            .or_else(|| at(content, &["comment", "commentRenderer"]))
                    })
                    .filter(|renderer| renderer.is_object())
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                comment_old(&renderer, parent)
            } else {
                let view = thread
                    .and_then(|thread| at(thread, &["commentViewModel", "commentViewModel"]))
                    .or_else(|| content.get("commentViewModel"))
                    .filter(|view| view.is_object());
                let keys: Vec<&str> = ["commentKey", "toolbarStateKey"]
                    .iter()
                    .filter_map(|key| view.and_then(|view| view.get(*key)).and_then(Value::as_str))
                    .collect();
                if keys.is_empty() {
                    continue;
                }
                let found: Vec<&Value> = entities
                    .iter()
                    .filter(|entity| {
                        entity
                            .get("entityKey")
                            .and_then(Value::as_str)
                            .is_some_and(|key| keys.contains(&key))
                    })
                    .collect();
                comment_new(&found, parent).map(|mut comment| {
                    let pinned = view
                        .and_then(|view| view.get("pinnedText"))
                        .is_some_and(Value::is_string);
                    comment.insert("is_pinned".into(), pinned.into());
                    comment
                })
            };
            let Some(comment) = comment else {
                continue;
            };
            let id = comment["id"].as_str().unwrap_or_default().to_owned();
            let pinned = comment.get("is_pinned") == Some(&Value::Bool(true));
            if pinned {
                self.tracker.pinned.insert(id.clone());
            }
            // YouTube may loop comments; pinned ones may repeat once.
            if self.tracker.seen.contains(&id) {
                if self.tracker.pinned.contains(&id) && !pinned {
                    continue;
                }
                crate::screen::warning(&format!(
                    "[youtube] Detected YouTube comments looping. Stopping comment extraction {}as we probably cannot get any more.",
                    if parent.is_some() {
                        "for this thread "
                    } else {
                        ""
                    }
                ));
                return Err(Stop::Return);
            }
            self.tracker.seen.insert(id.clone());
            self.tracker.running_total += 1;
            if parent.is_some() {
                self.tracker.total_reply_comments += 1;
            } else {
                self.tracker.total_parent_comments += 1;
            }
            self.emit(Value::Object(comment))?;
            // The replies.
            let Some(replies) = thread
                .and_then(|thread| at(thread, &["replies", "commentRepliesRenderer"]))
                .filter(|replies| replies.is_object())
            else {
                continue;
            };
            let subthreads: Vec<Value> = replies
                .get("subThreads")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|subthread| subthread.is_object())
                .cloned()
                .collect();
            let threads: Vec<Value> = subthreads
                .iter()
                .filter(|subthread| subthread.get("commentThreadRenderer").is_some())
                .cloned()
                .collect();
            if !threads.is_empty() {
                // A nested `return` or looping only ends that thread.
                match self.extract_thread(&threads, entities, Some(&id), depth + 1) {
                    Ok(()) | Err(Stop::Return) => {}
                    Err(stop) => return Err(stop),
                }
                if !subthreads
                    .iter()
                    .any(|subthread| subthread.get("continuationItemRenderer").is_some())
                {
                    continue;
                }
            }
            self.tracker.current_page_thread += 1;
            let limit = self.limits.replies_per_thread.min(
                self.limits
                    .replies
                    .saturating_sub(self.tracker.total_reply_comments),
            );
            if limit == 0 {
                continue;
            }
            self.slices.push(limit);
            let level = self.slices.len() - 1;
            let result = self.comment_entries(replies, Some(&id), depth + 1);
            self.slices.truncate(level);
            match result {
                Ok(()) | Err(Stop::Return) => {}
                Err(Stop::Slice(stopped)) if stopped == level => {}
                Err(stop) => return Err(stop),
            }
        }
        Ok(())
    }

    /// `_comment_entries`: a comment section's (or reply thread's) pages.
    fn comment_entries(
        &mut self,
        root: &Value,
        parent: Option<&str>,
        depth: usize,
    ) -> std::result::Result<(), Stop> {
        if self.limits.depth < depth {
            return Ok(());
        }
        let mut continuation = extract_continuation(root);
        let mut first = parent.is_none();
        let mut forced = false;
        if first && continuation.is_none() {
            // A generated continuation may find comments YouTube reports
            // disabled (such as stories).
            continuation = Some(continuation_query(
                &generated_continuation(self.video_id),
                None,
            ));
            forced = true;
        }
        let mut response: Option<Value> = None;
        let mut page = 0;
        while let Some(query) = continuation.take() {
            let progress = format!(
                "({}/~{})",
                self.tracker.running_total,
                self.tracker
                    .est_total
                    .map_or_else(|| "None".to_owned(), |total| total.to_string())
            );
            let note = if page == 0 {
                if first {
                    "Downloading comment section API JSON".to_owned()
                } else {
                    format!(
                        "    Downloading comment API JSON reply thread {} {progress}",
                        self.tracker.current_page_thread
                    )
                }
            } else {
                format!(
                    "{}Downloading comment{} API JSON page {page} {progress}",
                    if parent.is_some() { "       " } else { "" },
                    if parent.is_some() { " replies" } else { "" },
                )
            };
            // Incomplete data is retried unless YouTube reported no comments.
            let check =
                !forced && !(self.tracker.est_total == Some(0) && self.tracker.running_total == 0);
            let visitor = response
                .as_ref()
                .and_then(watch::visitor_from)
                .or_else(|| watch::visitor_from(&Value::Object(self.ytcfg.clone())));
            let fetched = self.request(query, &note, visitor.as_deref(), check, first)?;
            page += 1;
            forced = false;
            let Some(fetched) = fetched else {
                break;
            };
            let entities: Vec<Value> = at(
                &fetched,
                &["frameworkUpdates", "entityBatchUpdate", "mutations"],
            )
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|mutation| mutation.is_object())
            .cloned()
            .collect();
            for items in continuation_items(&fetched) {
                if first {
                    continuation = self.extract_header(&items);
                    first = false;
                    if continuation.is_some() {
                        break;
                    }
                    continue;
                }
                match self.extract_thread(&items, &entities, parent, depth) {
                    Ok(()) => {}
                    Err(Stop::Return) => return Ok(()),
                    Err(stop) => return Err(stop),
                }
                continuation = extract_continuation(&json!({"contents": items}));
                if continuation.is_some() {
                    break;
                }
            }
            response = Some(fetched);
        }
        // `CommentsDisabled`: YouTube's message and no comments.
        if parent.is_none() && self.tracker.running_total == 0 {
            let message = root
                .get("contents")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find_map(|content| at(content, &["messageRenderer", "text"]))
                .and_then(|text| {
                    text.get("simpleText")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .or_else(|| {
                            // `max_runs=1`.
                            at(text, &["runs", "0", "text"])
                                .and_then(Value::as_str)
                                .map(str::to_owned)
                        })
                })
                .filter(|message| !message.is_empty());
            if let Some(message) = message {
                crate::screen::warning_once(&format!(
                    "[youtube] {}: Youtube said: {message}",
                    self.video_id
                ));
                return Err(Stop::Disabled);
            }
        }
        Ok(())
    }

    /// `_extract_response(ep='next', check_get_keys=...)`: an incomplete
    /// response is retried three times, then left out.
    fn request(
        &self,
        query: Map<String, Value>,
        note: &str,
        visitor: Option<&str>,
        check: bool,
        header: bool,
    ) -> std::result::Result<Option<Value>, Stop> {
        const RETRIES: u32 = 3;
        for attempt in 0..=RETRIES {
            let response = watch::innertube_api(
                self.client,
                "next",
                "youtube",
                "",
                self.ytcfg,
                visitor,
                query.clone(),
                note,
            )
            .map_err(Stop::Error)?;
            let complete = !check
                || continuation_items(&response).iter().flatten().any(|item| {
                    if header {
                        item.get("commentsHeaderRenderer").is_some()
                    } else {
                        [
                            "commentThreadRenderer",
                            "commentViewModel",
                            "commentRenderer",
                        ]
                        .iter()
                        .any(|key| item.get(*key).is_some())
                    }
                });
            if complete {
                return Ok(Some(response));
            }
            if attempt < RETRIES {
                crate::screen::warning(&format!(
                    "[youtube] Incomplete data received. Retrying ({}/{RETRIES})...",
                    attempt + 1
                ));
            } else {
                crate::screen::warning(&format!(
                    "[youtube] Incomplete data received. Giving up after {RETRIES} retries"
                ));
            }
        }
        Ok(None)
    }
}

/// `('onResponseReceivedEndpoints', ..., ('reloadContinuationItemsCommand',
/// 'appendContinuationItemsAction'), 'continuationItems')`.
fn continuation_items(response: &Value) -> Vec<Vec<Value>> {
    response
        .get("onResponseReceivedEndpoints")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|endpoint| {
            [
                "reloadContinuationItemsCommand",
                "appendContinuationItemsAction",
            ]
            .iter()
            .filter_map(|key| at(endpoint, &[key, "continuationItems"]))
            .filter_map(|items| items.as_array().cloned())
            .collect::<Vec<_>>()
        })
        .collect()
}

fn author_url(endpoint: Option<&Value>) -> Option<String> {
    let endpoint = endpoint?;
    let path = at(endpoint, &["browseEndpoint", "canonicalBaseUrl"])
        .or_else(|| at(endpoint, &["commandMetadata", "webCommandMetadata", "url"]))
        .and_then(Value::as_str)?;
    url::Url::parse("https://www.youtube.com")
        .ok()?
        .join(path)
        .ok()
        .map(String::from)
}

/// `_extract_comment`: a comment from its entity payloads.
fn comment_new(entities: &[&Value], parent: Option<&str>) -> Option<Map<String, Value>> {
    let payload = entities
        .iter()
        .find_map(|entity| at(entity, &["payload", "commentEntityPayload"]))
        .filter(|payload| payload.is_object())?;
    let id = at(payload, &["properties", "commentId"]).and_then(Value::as_str)?;
    let toolbar = entities
        .iter()
        .find_map(|entity| at(entity, &["payload", "engagementToolbarStateEntityPayload"]))
        .filter(|toolbar| toolbar.is_object());
    let time_text = at(payload, &["properties", "publishedTime"])
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let mut comment = Map::new();
    comment.insert("id".into(), id.into());
    comment.insert("parent".into(), parent.unwrap_or("root").into());
    let mut field = |key: &str, value: Option<Value>| {
        if let Some(value) = value {
            comment.insert(key.into(), value);
        }
    };
    field(
        "text",
        at(payload, &["properties", "content", "content"])
            .filter(|v| v.is_string())
            .cloned(),
    );
    field(
        "like_count",
        at(payload, &["toolbar", "likeCountA11y"])
            .and_then(Value::as_str)
            .and_then(parse_count)
            .map(Value::from),
    );
    field(
        "author_id",
        at(payload, &["author", "channelId"])
            .and_then(Value::as_str)
            .and_then(ucid_or_none)
            .map(Value::from),
    );
    field(
        "author",
        at(payload, &["author", "displayName"])
            .filter(|v| v.is_string())
            .cloned(),
    );
    field(
        "author_thumbnail",
        at(payload, &["author", "avatarThumbnailUrl"])
            .and_then(Value::as_str)
            .filter(|url| url.contains("://"))
            .map(Value::from),
    );
    field(
        "author_is_uploader",
        at(payload, &["author", "isCreator"])
            .filter(|v| v.is_boolean())
            .cloned(),
    );
    field(
        "author_is_verified",
        at(payload, &["author", "isVerified"])
            .filter(|v| v.is_boolean())
            .cloned(),
    );
    field(
        "author_url",
        author_url(at(
            payload,
            &["author", "channelCommand", "innertubeCommand"],
        ))
        .map(Value::from),
    );
    comment.insert(
        "is_favorited".into(),
        toolbar.map_or(Value::Null, |toolbar| {
            (toolbar.get("heartState").and_then(Value::as_str)
                == Some("TOOLBAR_HEART_STATE_HEARTED"))
            .into()
        }),
    );
    comment.insert("_time_text".into(), time_text.clone().into());
    comment.insert(
        "timestamp".into(),
        (!time_text.is_empty())
            .then(|| parse_time_text(&time_text))
            .flatten()
            .into(),
    );
    Some(comment)
}

/// `_extract_comment_old`: a `commentRenderer`.
fn comment_old(renderer: &Value, parent: Option<&str>) -> Option<Map<String, Value>> {
    let id = renderer
        .get("commentId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())?;
    let mut comment = Map::new();
    comment.insert("id".into(), id.into());
    comment.insert(
        "text".into(),
        renderer.get("contentText").and_then(get_text).into(),
    );
    comment.insert(
        "like_count".into(),
        get_count(renderer.get("voteCount")).into(),
    );
    comment.insert(
        "author_id".into(),
        at(renderer, &["authorEndpoint", "browseEndpoint", "browseId"])
            .and_then(Value::as_str)
            .and_then(ucid_or_none)
            .into(),
    );
    comment.insert(
        "author".into(),
        renderer.get("authorText").and_then(get_text).into(),
    );
    comment.insert(
        "author_thumbnail".into(),
        renderer
            .get("authorThumbnail")
            .and_then(|thumbnail| thumbnail.get("thumbnails"))
            .and_then(Value::as_array)
            .and_then(|thumbnails| thumbnails.last())
            .and_then(|thumbnail| thumbnail.get("url"))
            .and_then(Value::as_str)
            .filter(|url| url.contains("://"))
            .into(),
    );
    comment.insert("parent".into(), parent.unwrap_or("root").into());
    let time_text = renderer
        .get("publishedTimeText")
        .and_then(get_text)
        .unwrap_or_default();
    comment.insert("_time_text".into(), time_text.clone().into());
    comment.insert(
        "timestamp".into(),
        (!time_text.is_empty())
            .then(|| parse_time_text(&time_text))
            .flatten()
            .into(),
    );
    comment.insert(
        "author_url".into(),
        author_url(renderer.get("authorEndpoint")).into(),
    );
    if let Some(owner) = renderer
        .get("authorIsChannelOwner")
        .filter(|v| !v.is_null())
    {
        comment.insert("author_is_uploader".into(), owner.clone());
    }
    if let Some(buttons) = at(renderer, &["actionButtons", "commentActionButtonsRenderer"])
        .filter(|buttons| buttons.is_object())
    {
        comment.insert(
            "is_favorited".into(),
            buttons.get("creatorHeart").is_some().into(),
        );
    }
    let verified = at(
        renderer,
        &[
            "authorCommentBadge",
            "authorCommentBadgeRenderer",
            "icon",
            "iconType",
        ],
    )
    .and_then(Value::as_str)
    .is_some_and(|icon| {
        matches!(
            icon,
            "CHECK_CIRCLE_THICK" | "OFFICIAL_ARTIST_BADGE" | "CHECK"
        )
    });
    if verified {
        comment.insert("author_is_verified".into(), true.into());
    }
    if renderer
        .get("pinnedCommentBadge")
        .is_some_and(|badge| !badge.is_null())
    {
        comment.insert("is_pinned".into(), true.into());
    }
    Some(comment)
}

/// `extract_comments`' extractor: every comment (within the limits), then
/// `Extracted N comments`; disabled comments give `None`s.
fn extract(client: &Client, video_id: &str, pending: &Pending) -> Result<(Value, Value)> {
    let limits = limits();
    let sort_new = crate::manifest_args::configuration_arg("youtube", "comment_sort", false)
        .and_then(|values| values.first().cloned())
        .is_none_or(|sort| sort != "top");
    let comments_limit = limits.comments;
    let mut extraction = Extraction {
        client,
        video_id,
        ytcfg: &pending.ytcfg,
        limits,
        sort_new,
        tracker: Tracker::default(),
        comments: Vec::new(),
        slices: vec![comments_limit],
    };
    if comments_limit > 0 {
        // `_real_comment_extract`: the `comment-item-section` renderer.
        let section = pending
            .contents
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|content| content.get("itemSectionRenderer"))
            .find(|section| {
                section.get("sectionIdentifier").and_then(Value::as_str)
                    == Some("comment-item-section")
            })
            .cloned()
            .unwrap_or(Value::Null);
        match extraction.comment_entries(&section, None, 1) {
            Ok(()) | Err(Stop::Return | Stop::Slice(_)) => {}
            Err(Stop::Disabled) => return Ok((Value::Null, Value::Null)),
            Err(Stop::Error(error)) => return Err(error),
        }
    }
    let count = extraction.comments.len();
    crate::screen::screen(&format!("[youtube] Extracted {count} comments"));
    Ok((Value::Array(extraction.comments), count.into()))
}
