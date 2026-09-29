//! NATS subject builders for LabJack live data and archive exports.
//!
//! The streamer publishes each channel's scans on a live subject, the archiver
//! consumes the same subjects from local JetStream and writes Parquet, and the exporter
//! listens on an export request subject. All of them build their subject names here
//! so the layouts stay in one place.
//!
//! Two layouts are supported:
//!
//! * Legacy, keyed by asset number: `avenabox.<asset>.data.chNN`, for example
//!   `avenabox.1456.data.ch11`. The root is used exactly as configured.
//! * Structured, keyed by identity: `avenars.<site>.<box>.<source>.live.chNN`, for
//!   example `avenars.i69.i69-mu1.i69-lj2.live.ch11`. Every token is passed through
//!   [`sanitize_token`].
//!
//! The structured layout is chosen when the root is `avenars` or any of site, box or
//! source ID is set (see [`uses_structured_namespace`]).

#![allow(dead_code)]

/// Converts arbitrary identity text into a single NATS subject token.
///
/// Leading and trailing whitespace is trimmed. ASCII letters and digits are
/// lowercased, `-` and `_` are kept, and whitespace, `.` and `/` become `-`. Every
/// other character is dropped. Leading and trailing `-` are then stripped. This keeps
/// `.` (the subject separator) and the wildcards `*` and `>` out of the token.
///
/// # Arguments
///
/// * `raw` - Identity text such as a site ID, box ID or LabJack name.
///
/// # Returns
///
/// The sanitized token, or `unknown` if nothing usable remains.
///
/// # Examples
///
/// ```text
/// sanitize_token("I69 MU1")   -> "i69-mu1"
/// sanitize_token("site.a/b")  -> "site-a-b"
/// sanitize_token(" *> ")      -> "unknown"
/// ```
fn sanitize_token(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch.to_ascii_lowercase());
        } else if ch.is_whitespace() || ch == '.' || ch == '/' {
            out.push('-');
        }
    }

    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "unknown".to_string()
    } else {
        out
    }
}

/// Formats a LabJack channel number as the `chNN` token used in subjects.
///
/// The number is zero-padded to at least two digits.
///
/// # Arguments
///
/// * `ch` - LabJack channel number.
///
/// # Examples
///
/// ```text
/// pad_channel(3)   -> "ch03"
/// pad_channel(11)  -> "ch11"
/// pad_channel(120) -> "ch120"
/// ```
pub fn pad_channel(ch: u8) -> String {
    format!("ch{ch:02}")
}

/// Formats an asset number with the three-digit legacy subject width.
///
/// The number is zero-padded to at least three digits; longer numbers are not
/// truncated.
///
/// # Arguments
///
/// * `n` - Asset number.
///
/// # Examples
///
/// ```text
/// pad_asset(7)    -> "007"
/// pad_asset(1456) -> "1456"
/// ```
pub fn pad_asset(n: u32) -> String {
    format!("{n:03}")
}

/// Treats an empty identity field the same as a missing one.
///
/// The webapp builds the same subjects with JavaScript `||` fallbacks, where an empty
/// string counts as missing. Doing the same here keeps both sides on one subject when
/// a config has `""` in an identity field.
///
/// # Arguments
///
/// * `value` - Identity field from the config or environment.
///
/// # Returns
///
/// `None` if `value` is `None` or an empty string, otherwise `value`.
fn present(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.is_empty())
}

/// Picks the source token shared by every structured subject.
///
/// The first of `source_id`, `labjack_name` and `asset<NNN>` that is present is used,
/// passed through [`sanitize_token`]. Live channel subjects, the stream wildcard and
/// the export request subject all go through this function, so a config without
/// identity fields still produces a stream that matches its own channels and an
/// export subject that the webapp can find. The webapp (`webapp/src/lib/subjects.ts`)
/// uses the same order. `unknown-source` is only used when there is no asset number
/// either, which can happen for the exporter when neither `SOURCE_ID`, `LABJACK_NAME`
/// nor `ASSET_NUMBER` is set.
///
/// # Arguments
///
/// * `source_id` - Configured source ID, if any.
/// * `labjack_name` - LabJack name, if any.
/// * `asset` - Asset number, if known.
///
/// # Examples
///
/// ```text
/// (Some("i69-lj2"), Some("Unit A"), Some(42)) -> "i69-lj2"
/// (None, Some("Unit A"), Some(42))            -> "unit-a"
/// (None, None, Some(42))                      -> "asset042"
/// (None, None, None)                          -> "unknown-source"
/// ```
fn source_token(source_id: Option<&str>, labjack_name: Option<&str>, asset: Option<u32>) -> String {
    match (present(source_id).or(present(labjack_name)), asset) {
        (Some(source), _) => sanitize_token(source),
        (None, Some(asset)) => format!("asset{}", pad_asset(asset)),
        (None, None) => "unknown-source".to_string(),
    }
}

/// Decides whether a config should use the structured subject namespace.
///
/// # Arguments
///
/// * `nats_subject` - Configured subject root, for example `avenabox` or `avenars`.
/// * `site_id` - Configured site ID, if any.
/// * `box_id` - Configured box ID, if any.
/// * `source_id` - Configured source ID, if any.
///
/// # Returns
///
/// `true` if the trimmed root is exactly `avenars` or any of the three IDs is set and
/// not empty. A LabJack name alone does not switch to the structured layout.
fn uses_structured_namespace(
    nats_subject: &str,
    site_id: Option<&str>,
    box_id: Option<&str>,
    source_id: Option<&str>,
) -> bool {
    nats_subject.trim() == "avenars"
        || present(site_id).is_some()
        || present(box_id).is_some()
        || present(source_id).is_some()
}

/// Builds the live-data subject for one LabJack channel.
///
/// Legacy configs use `<root>.<asset>.data.<channel>`, with the root used as given.
/// Structured configs use `<root>.<site>.<box>.<source>.live.<channel>`, with every
/// token passed through [`sanitize_token`]. In the structured layout a missing or empty
/// site or box becomes `unknown-site` or `unknown-box`, and the source is chosen by
/// [`source_token`]: the first of `source_id`, `labjack_name` or `asset<NNN>` that is
/// set.
///
/// # Arguments
///
/// * `nats_subject` - Subject root, for example `avenabox` or `avenars`.
/// * `asset` - Asset number. Used in the legacy layout and as the last-resort source
///   token in the structured layout.
/// * `channel` - LabJack channel number, formatted by [`pad_channel`].
/// * `site_id` - Site ID for the structured layout.
/// * `box_id` - Box ID for the structured layout.
/// * `labjack_name` - LabJack name, used as the source when `source_id` is `None`.
/// * `source_type` - Accepted for config compatibility; not used in the subject.
/// * `source_id` - Source ID for the structured layout.
///
/// # Examples
///
/// ```text
/// ("avenabox", 1456, 11, None, None, None, None, None)
///     -> "avenabox.1456.data.ch11"
/// ("avenars", 1456, 11, Some("i69"), Some("i69-mu1"), Some("i69-lj2"), None, None)
///     -> "avenars.i69.i69-mu1.i69-lj2.live.ch11"
/// ("avenars", 42, 3, None, None, None, None, None)
///     -> "avenars.unknown-site.unknown-box.asset042.live.ch03"
/// ```
pub fn live_labjack_channel_subject(
    nats_subject: &str,
    asset: u32,
    channel: u8,
    site_id: Option<&str>,
    box_id: Option<&str>,
    labjack_name: Option<&str>,
    source_type: Option<&str>,
    source_id: Option<&str>,
) -> String {
    if !uses_structured_namespace(nats_subject, site_id, box_id, source_id) {
        return format!(
            "{}.{}.data.{}",
            nats_subject,
            pad_asset(asset),
            pad_channel(channel)
        );
    }

    let root = sanitize_token(nats_subject);
    let site_id = sanitize_token(present(site_id).unwrap_or("unknown-site"));
    let box_id = sanitize_token(present(box_id).unwrap_or("unknown-box"));
    let source_id = source_token(source_id, labjack_name, Some(asset));

    let _ = source_type;

    format!(
        "{root}.{site_id}.{box_id}.{source_id}.live.{}",
        pad_channel(channel)
    )
}

/// Builds the JetStream subject wildcard for all live channels from one source.
///
/// Legacy configs get `<root>.*.data.*`, which matches every asset under the root.
/// Structured configs get `<root>.<site>.<box>.<source>.live.*`, with the same tokens
/// and fallbacks as [`live_labjack_channel_subject`], so the wildcard always matches
/// the channel subjects built from the same config.
///
/// # Arguments
///
/// * `nats_subject` - Subject root, for example `avenabox` or `avenars`.
/// * `asset` - Asset number, the last-resort source token in the structured layout.
///   Not used in the legacy layout, whose wildcard covers every asset.
/// * `site_id` - Site ID for the structured layout.
/// * `box_id` - Box ID for the structured layout.
/// * `labjack_name` - LabJack name, used as the source when `source_id` is `None`.
/// * `source_type` - Accepted for config compatibility; not used in the subject.
/// * `source_id` - Source ID for the structured layout.
///
/// # Examples
///
/// ```text
/// ("avenabox", 1456, None, None, None, None, None)
///     -> "avenabox.*.data.*"
/// ("avenars", 1456, Some("i69"), Some("i69-mu1"), Some("i69-lj2"), None, None)
///     -> "avenars.i69.i69-mu1.i69-lj2.live.*"
/// ("avenars", 42, None, None, None, None, None)
///     -> "avenars.unknown-site.unknown-box.asset042.live.*"
/// ```
pub fn live_labjack_stream_subject(
    nats_subject: &str,
    asset: u32,
    site_id: Option<&str>,
    box_id: Option<&str>,
    labjack_name: Option<&str>,
    source_type: Option<&str>,
    source_id: Option<&str>,
) -> String {
    if !uses_structured_namespace(nats_subject, site_id, box_id, source_id) {
        return format!("{nats_subject}.*.data.*");
    }

    let root = sanitize_token(nats_subject);
    let site_id = sanitize_token(present(site_id).unwrap_or("unknown-site"));
    let box_id = sanitize_token(present(box_id).unwrap_or("unknown-box"));
    let source_id = source_token(source_id, labjack_name, Some(asset));

    let _ = source_type;

    format!("{root}.{site_id}.{box_id}.{source_id}.live.*")
}

/// Builds the NATS request subject that the exporter serves archive exports on.
///
/// Always uses the structured layout `<root>.<site>.<box>.<source>.export.request`,
/// whatever the root, with every token passed through [`sanitize_token`]. Missing or
/// empty IDs become `unknown-site` and `unknown-box`, and the source is chosen by
/// [`source_token`] exactly as for the live subjects, so the webapp reaches the
/// exporter with the subject it derives from the source's config.
///
/// # Arguments
///
/// * `nats_subject` - Subject root, for example `avenars`.
/// * `asset` - Asset number, the last-resort source token, if known.
/// * `site_id` - Site ID.
/// * `box_id` - Box ID.
/// * `labjack_name` - LabJack name, used as the source when `source_id` is not set.
/// * `source_type` - Accepted for config compatibility; not used in the subject.
/// * `source_id` - Source ID.
///
/// # Examples
///
/// ```text
/// ("avenars", None, Some("i69"), Some("i69-mu1"), None, Some("labjack"), Some("i69-lj2"))
///     -> "avenars.i69.i69-mu1.i69-lj2.export.request"
/// ("avenars", Some(42), None, None, None, None, None)
///     -> "avenars.unknown-site.unknown-box.asset042.export.request"
/// ```
pub fn archive_export_request_subject(
    nats_subject: &str,
    asset: Option<u32>,
    site_id: Option<&str>,
    box_id: Option<&str>,
    labjack_name: Option<&str>,
    source_type: Option<&str>,
    source_id: Option<&str>,
) -> String {
    let root = sanitize_token(nats_subject);
    let site = sanitize_token(present(site_id).unwrap_or("unknown-site"));
    let box_id = sanitize_token(present(box_id).unwrap_or("unknown-box"));
    let source_id = source_token(source_id, labjack_name, asset);

    let _ = source_type;

    format!("{root}.{site}.{box_id}.{source_id}.export.request")
}

/// Checks whether an existing stream subject is compatible with a desired one.
///
/// An exact match is compatible. When the desired subject is a legacy wildcard
/// `<prefix>.*.data.*`, an existing subject is also compatible if it starts with
/// `<prefix>.` and ends with `.data.*`, so an older per-asset pattern such as
/// `avenabox.1456.data.*` is accepted. Anything else is incompatible.
///
/// # Arguments
///
/// * `existing` - A subject already configured on the JetStream stream.
/// * `desired_namespace` - The subject the current config wants, typically from
///   [`live_labjack_stream_subject`].
///
/// # Examples
///
/// ```text
/// ("avenabox.1456.data.*", "avenabox.*.data.*")         -> true
/// ("avenabox.*.data.*", "avenabox.*.data.*")            -> true
/// ("avenars.i69.a.b.live.*", "avenars.i69.a.c.live.*")  -> false
/// ```
pub fn stream_subject_is_compatible(existing: &str, desired_namespace: &str) -> bool {
    if existing == desired_namespace {
        return true;
    }

    if let Some(prefix) = desired_namespace.strip_suffix(".*.data.*") {
        return existing.starts_with(&format!("{prefix}.")) && existing.ends_with(".data.*");
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Legacy configs keep the `<root>.<asset>.data.chNN` layout.
    #[test]
    fn legacy_subjects_are_preserved() {
        assert_eq!(
            live_labjack_channel_subject("avenabox", 1456, 11, None, None, None, None, None),
            "avenabox.1456.data.ch11"
        );
        assert_eq!(
            live_labjack_stream_subject("avenabox", 1456, None, None, None, None, None),
            "avenabox.*.data.*"
        );
    }

    /// Structured configs produce site, box and source tokens in every subject kind.
    #[test]
    fn structured_subjects_include_site_box_source_and_channel() {
        assert_eq!(
            live_labjack_channel_subject(
                "avenars",
                1456,
                11,
                Some("i69"),
                Some("i69-mu1"),
                Some("i69-lj2"),
                None,
                None,
            ),
            "avenars.i69.i69-mu1.i69-lj2.live.ch11"
        );
        assert_eq!(
            live_labjack_stream_subject(
                "avenars",
                1456,
                Some("i69"),
                Some("i69-mu1"),
                Some("i69-lj2"),
                None,
                None,
            ),
            "avenars.i69.i69-mu1.i69-lj2.live.*"
        );
        assert_eq!(
            archive_export_request_subject(
                "avenars",
                Some(1456),
                Some("i69"),
                Some("i69-mu1"),
                Some("Unit A"),
                Some("labjack"),
                Some("i69-lj2"),
            ),
            "avenars.i69.i69-mu1.i69-lj2.export.request"
        );
    }

    /// Returns whether a NATS subject matches a pattern with `*` and `>` wildcards.
    fn nats_subject_matches(pattern: &str, subject: &str) -> bool {
        let mut pattern = pattern.split('.');
        let mut subject = subject.split('.');
        loop {
            match (pattern.next(), subject.next()) {
                (Some(">"), Some(_)) => return true,
                (Some("*"), Some(_)) => {}
                (Some(p), Some(s)) if p == s => {}
                (None, None) => return true,
                _ => return false,
            }
        }
    }

    /// Without identity fields, the stream wildcard still covers every channel subject
    /// and all three subjects share the `asset<NNN>` source token.
    #[test]
    fn stream_wildcard_matches_channels_without_identity_fields() {
        let stream = live_labjack_stream_subject("avenars", 42, None, None, None, None, None);
        assert_eq!(stream, "avenars.unknown-site.unknown-box.asset042.live.*");
        for channel in [0, 3, 11, 120] {
            let live =
                live_labjack_channel_subject("avenars", 42, channel, None, None, None, None, None);
            assert!(
                nats_subject_matches(&stream, &live),
                "{stream} should match {live}"
            );
        }
        assert_eq!(
            archive_export_request_subject("avenars", Some(42), None, None, None, None, None),
            "avenars.unknown-site.unknown-box.asset042.export.request"
        );
    }

    /// Empty identity fields fall back the same way as missing ones, like the webapp.
    #[test]
    fn empty_identity_fields_fall_back_like_missing_ones() {
        let stream =
            live_labjack_stream_subject("avenars", 7, Some(""), Some(""), Some(""), None, Some(""));
        let live = live_labjack_channel_subject(
            "avenars",
            7,
            5,
            Some(""),
            Some(""),
            Some(""),
            None,
            Some(""),
        );
        let export = archive_export_request_subject(
            "avenars",
            Some(7),
            Some(""),
            Some(""),
            Some(""),
            None,
            Some(""),
        );
        assert_eq!(stream, "avenars.unknown-site.unknown-box.asset007.live.*");
        assert_eq!(live, "avenars.unknown-site.unknown-box.asset007.live.ch05");
        assert!(nats_subject_matches(&stream, &live));
        assert_eq!(
            export,
            "avenars.unknown-site.unknown-box.asset007.export.request"
        );

        // With only a LabJack name, every subject uses it as the source.
        let stream =
            live_labjack_stream_subject("avenars", 7, None, None, Some("Unit A"), None, None);
        let live =
            live_labjack_channel_subject("avenars", 7, 5, None, None, Some("Unit A"), None, None);
        let export = archive_export_request_subject(
            "avenars",
            Some(7),
            None,
            None,
            Some("Unit A"),
            None,
            None,
        );
        assert!(nats_subject_matches(&stream, &live));
        assert_eq!(
            export,
            "avenars.unknown-site.unknown-box.unit-a.export.request"
        );

        // An empty legacy config stays in the legacy layout.
        assert_eq!(
            live_labjack_channel_subject("avenabox", 7, 5, Some(""), None, None, None, None),
            "avenabox.007.data.ch05"
        );
    }

    /// The exporter falls back to `unknown-source` only when it has no asset number.
    #[test]
    fn export_subject_without_any_source_uses_unknown_source() {
        assert_eq!(
            archive_export_request_subject(
                "avenars",
                None,
                Some("i69"),
                Some("i69-mu1"),
                None,
                None,
                None
            ),
            "avenars.i69.i69-mu1.unknown-source.export.request"
        );
    }
}
