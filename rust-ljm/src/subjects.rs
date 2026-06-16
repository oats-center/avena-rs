//! NATS subject naming helpers for LabJack live data and exports.
//!
//! The code supports the original asset-number subject layout and the newer
//! structured namespace that includes site, box, and source identity.

#![allow(dead_code)]

/// Converts arbitrary identity text into a NATS subject token.
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
pub fn pad_channel(ch: u8) -> String {
    format!("ch{ch:02}")
}

/// Formats an asset number with the three-digit legacy subject width.
pub fn pad_asset(n: u32) -> String {
    format!("{n:03}")
}

/// Decides whether a config should use the structured subject namespace.
fn uses_structured_namespace(
    nats_subject: &str,
    site_id: Option<&str>,
    box_id: Option<&str>,
    source_id: Option<&str>,
) -> bool {
    nats_subject.trim() == "avenars" || site_id.is_some() || box_id.is_some() || source_id.is_some()
}

/// Builds the live-data subject for one LabJack channel.
///
/// Legacy configs use `<root>.<asset>.data.<channel>`. Structured configs use
/// `<root>.<site>.<box>.<source>.live.<channel>`.
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
    let site_id = sanitize_token(site_id.unwrap_or("unknown-site"));
    let box_id = sanitize_token(box_id.unwrap_or("unknown-box"));
    let source = source_id
        .or(labjack_name)
        .map(str::to_string)
        .unwrap_or_else(|| format!("asset{}", pad_asset(asset)));
    let source_id = sanitize_token(&source);

    let _ = source_type;

    format!(
        "{root}.{site_id}.{box_id}.{source_id}.live.{}",
        pad_channel(channel)
    )
}

/// Builds the JetStream subject wildcard for all live channels from one source.
pub fn live_labjack_stream_subject(
    nats_subject: &str,
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
    let site_id = sanitize_token(site_id.unwrap_or("unknown-site"));
    let box_id = sanitize_token(box_id.unwrap_or("unknown-box"));
    let source_id = sanitize_token(source_id.or(labjack_name).unwrap_or("unknown-source"));

    let _ = source_type;

    format!("{root}.{site_id}.{box_id}.{source_id}.live.*")
}

/// Builds the NATS worker request subject for archive exports.
pub fn archive_export_request_subject(
    nats_subject: &str,
    site_id: Option<&str>,
    box_id: Option<&str>,
    source_type: Option<&str>,
    source_id: Option<&str>,
) -> String {
    let root = sanitize_token(nats_subject);
    let site = sanitize_token(site_id.unwrap_or("unknown-site"));
    let box_id = sanitize_token(box_id.unwrap_or("unknown-box"));
    let source_id = sanitize_token(source_id.unwrap_or("unknown-source"));

    let _ = source_type;

    format!("{root}.{site}.{box_id}.{source_id}.export.request")
}

/// Checks whether an existing stream subject covers a desired namespace.
///
/// This keeps stream reconciliation tolerant of older legacy subject patterns
/// while still detecting incompatible configured streams.
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

    #[test]
    fn legacy_subjects_are_preserved() {
        assert_eq!(
            live_labjack_channel_subject("avenabox", 1456, 11, None, None, None, None, None),
            "avenabox.1456.data.ch11"
        );
        assert_eq!(
            live_labjack_stream_subject("avenabox", None, None, None, None, None),
            "avenabox.*.data.*"
        );
    }

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
                Some("i69"),
                Some("i69-mu1"),
                Some("labjack"),
                Some("i69-lj2"),
            ),
            "avenars.i69.i69-mu1.i69-lj2.export.request"
        );
    }
}
