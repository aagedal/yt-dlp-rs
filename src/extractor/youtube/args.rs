//! `--extractor-args youtube:...` as `YoutubeIE._configuration_arg` reads
//! them (`CONFIGURATION_ARG_KEY = 'youtube'`).

/// `_configuration_arg(key)`: the values (lowercased), `[]` when absent.
pub(super) fn list(key: &str) -> Vec<String> {
    crate::manifest_args::configuration_arg("youtube", key, false).unwrap_or_default()
}

/// `_configuration_arg(key, casesense=True)`.
pub(super) fn list_case(key: &str) -> Vec<String> {
    crate::manifest_args::configuration_arg("youtube", key, true).unwrap_or_default()
}

/// `_configuration_arg(key, [None])[0]`: the first value, `None` when the
/// argument is absent (an empty value is kept, as upstream's `or` sees it).
pub(super) fn first(key: &str) -> Option<String> {
    list(key).into_iter().next()
}

/// [`first`], case-sensitive.
pub(super) fn first_case(key: &str) -> Option<String> {
    list_case(key).into_iter().next()
}

/// `VALUE in self._configuration_arg(key)`.
pub(super) fn contains(key: &str, value: &str) -> bool {
    list(key).iter().any(|item| item == value)
}

/// `_configuration_arg(key, [''])[0] or None`: a non-empty first value.
pub(super) fn non_empty(key: &str) -> Option<String> {
    first(key).filter(|value| !value.is_empty())
}

/// [`non_empty`], case-sensitive.
pub(super) fn non_empty_case(key: &str) -> Option<String> {
    first_case(key).filter(|value| !value.is_empty())
}
