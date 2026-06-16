/** Configuration fields used to build LabJack NATS subjects and KV keys. */
export interface LabJackSubjectConfig {
  /** Root NATS subject, such as `avenabox` or `avenars`. */
  nats_subject: string;
  /** Numeric asset identifier used by the legacy subject layout. */
  asset_number: number;
  /** Human-readable LabJack name used as a fallback source id. */
  labjack_name?: string;
  /** Site identifier used by the structured subject layout. */
  site_id?: string | null;
  /** Edge box identifier used by the structured subject layout. */
  box_id?: string | null;
  /** Optional source type retained for config compatibility. */
  source_type?: string | null;
  /** Source identifier used by the structured subject layout. */
  source_id?: string | null;
}

/** Normalizes free-form config text into a NATS subject token. */
function sanitizeToken(raw: string): string {
  const normalized = raw
    .trim()
    .toLowerCase()
    .replace(/[\s./]+/g, "-")
    .replace(/[^a-z0-9_-]/g, "")
    .replace(/^-+|-+$/g, "");

  return normalized || "unknown";
}

/** Formats an asset number with the three-digit legacy subject width. */
function padAsset(asset: number): string {
  return String(asset).padStart(3, "0");
}

/** Formats a LabJack channel number as the `chNN` token used in subjects. */
export function padChannel(channel: number): string {
  return `ch${String(channel).padStart(2, "0")}`;
}

/** Returns true when config should use the structured site/box/source namespace. */
function usesStructuredNamespace(config: LabJackSubjectConfig): boolean {
  return config.nats_subject.trim() === "avenars" || Boolean(config.site_id || config.box_id || config.source_id);
}

/**
 * Builds the live-data subject for one LabJack channel.
 *
 * Legacy configs use `<root>.<asset>.data.<channel>`. Structured configs use
 * `<root>.<site>.<box>.<source>.live.<channel>`.
 */
export function liveLabJackChannelSubject(config: LabJackSubjectConfig, channel: number): string {
  if (!usesStructuredNamespace(config)) {
    return `${config.nats_subject}.${padAsset(config.asset_number)}.data.${padChannel(channel)}`;
  }

  const root = sanitizeToken(config.nats_subject);
  const siteId = sanitizeToken(config.site_id || "unknown-site");
  const boxId = sanitizeToken(config.box_id || "unknown-box");
  const sourceId = sanitizeToken(
    config.source_id || config.labjack_name || `asset${padAsset(config.asset_number)}`
  );

  return `${root}.${siteId}.${boxId}.${sourceId}.live.${padChannel(channel)}`;
}

/** Builds a display pattern for all live channels from one LabJack source. */
export function liveLabJackChannelPattern(config: LabJackSubjectConfig): string {
  if (!usesStructuredNamespace(config)) {
    return `${config.nats_subject}.${padAsset(config.asset_number)}.data.ch##`;
  }

  const root = sanitizeToken(config.nats_subject);
  const siteId = sanitizeToken(config.site_id || "unknown-site");
  const boxId = sanitizeToken(config.box_id || "unknown-box");
  const sourceId = sanitizeToken(
    config.source_id || config.labjack_name || `asset${padAsset(config.asset_number)}`
  );

  return `${root}.${siteId}.${boxId}.${sourceId}.live.ch##`;
}

/** Builds the dashboard configuration KV key for one site, box, and source. */
export function labjackConfigKey(config: {
  site_id?: string | null;
  box_id?: string | null;
  source_id?: string | null;
  labjack_name?: string | null;
}): string {
  const siteId = sanitizeToken(config.site_id || "unknown-site");
  const boxId = sanitizeToken(config.box_id || "unknown-box");
  const sourceId = sanitizeToken(config.source_id || config.labjack_name || "unknown-source");
  return `${siteId}.${boxId}.${sourceId}.config`;
}

/** Builds the NATS exporter worker request subject for one archived source. */
export function archiveExportRequestSubject(config: LabJackSubjectConfig): string {
  const root = sanitizeToken(config.nats_subject);
  const siteId = sanitizeToken(config.site_id || "unknown-site");
  const boxId = sanitizeToken(config.box_id || "unknown-box");
  const sourceId = sanitizeToken(
    config.source_id || config.labjack_name || `asset${padAsset(config.asset_number)}`
  );

  return `${root}.${siteId}.${boxId}.${sourceId}.export.request`;
}
