#![allow(dead_code)]

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

pub fn pad_channel(ch: u8) -> String {
    format!("ch{ch:02}")
}

pub fn pad_asset(n: u32) -> String {
    format!("{n:03}")
}

fn uses_v1_namespace(nats_subject: &str, site_id: Option<&str>, box_id: Option<&str>) -> bool {
    nats_subject.trim() == "avenars" || site_id.is_some() || box_id.is_some()
}

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
    if !uses_v1_namespace(nats_subject, site_id, box_id) {
        return format!(
            "{}.{}.data.{}",
            nats_subject,
            pad_asset(asset),
            pad_channel(channel)
        );
    }

    let root = sanitize_token(nats_subject);
    let site = sanitize_token(site_id.unwrap_or("unknown-site"));
    let box_id = sanitize_token(box_id.unwrap_or("unknown-box"));
    let source_type = sanitize_token(source_type.unwrap_or("labjack"));
    let source = source_id
        .or(labjack_name)
        .map(str::to_string)
        .unwrap_or_else(|| format!("asset{}", pad_asset(asset)));
    let source_id = sanitize_token(&source);

    format!(
        "{root}.v1.{site}.{box_id}.live.{source_type}.{source_id}.sample.{}",
        pad_channel(channel)
    )
}

pub fn live_labjack_stream_subject(
    nats_subject: &str,
    site_id: Option<&str>,
    box_id: Option<&str>,
    labjack_name: Option<&str>,
    source_type: Option<&str>,
    source_id: Option<&str>,
) -> String {
    if !uses_v1_namespace(nats_subject, site_id, box_id) {
        return format!("{nats_subject}.*.data.*");
    }

    let root = sanitize_token(nats_subject);
    let site = sanitize_token(site_id.unwrap_or("unknown-site"));
    let box_id = sanitize_token(box_id.unwrap_or("unknown-box"));
    let source_type = sanitize_token(source_type.unwrap_or("labjack"));
    let source_id = sanitize_token(source_id.or(labjack_name).unwrap_or("unknown-source"));

    format!("{root}.v1.{site}.{box_id}.live.{source_type}.{source_id}.sample.*")
}

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
    fn v1_subjects_include_site_box_source_and_channel() {
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
            "avenars.v1.i69.i69-mu1.live.labjack.i69-lj2.sample.ch11"
        );
    }
}
