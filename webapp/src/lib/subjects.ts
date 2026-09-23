/**
 * NATS subject and KV key builders for LabJack live data, configs and exports.
 *
 * These mirror the builders in `rust-ljm/src/subjects.rs`, which the streamer,
 * archiver and exporter use, so the dashboard subscribes and publishes on the same
 * names. Two live-data layouts exist:
 *
 * - Legacy, keyed by asset number: `<root>.<asset>.data.chNN`, for example
 *   `avenabox.1456.data.ch11`. The root is used exactly as configured.
 * - Structured, keyed by identity: `avenars.<site>.<box>.<source>.live.chNN`, for
 *   example `avenars.i69.i69-mu1.i69-lj2.live.ch11`. Every token is sanitized.
 *
 * The structured layout is used when the root is `avenars` or any of site, box or
 * source ID is set.
 *
 * @module
 */

/** Fields of a LabJack config (KV bucket `avenabox`) that name its subjects. */
export interface LabJackSubjectConfig {
  /** Root subject token, such as `avenabox` (legacy) or `avenars` (structured). */
  nats_subject: string;
  /** Asset number. Used in legacy subjects and as the last source ID fallback. */
  asset_number: number;
  /** LabJack name. Used as the source ID when `source_id` is empty. */
  labjack_name?: string;
  /** Site ID, the `<site>` token of structured subjects. */
  site_id?: string | null;
  /** Edge box ID, the `<box>` token of structured subjects. */
  box_id?: string | null;
  /** Source type. Kept for config compatibility; no builder here reads it. */
  source_type?: string | null;
  /** Source ID, the `<source>` token of structured subjects. */
  source_id?: string | null;
}

/**
 * Converts free-form identity text into a single NATS subject token.
 *
 * Trims and lowercases the text, replaces each run of whitespace, `.` and `/` with
 * one `-`, drops every character other than `a-z`, `0-9`, `_` and `-`, and strips
 * leading and trailing `-`. This keeps `.` and the wildcards `*` and `>` out of the
 * token.
 *
 * @remarks
 * The Rust `sanitize_token` turns each such character into its own `-`, so input
 * with consecutive separators (for example `"a  b"`) gives `a-b` here but `a--b`
 * there.
 *
 * @param raw - Identity text such as a site ID, box ID or LabJack name.
 * @returns The token, or `unknown` if nothing usable remains.
 */
function sanitizeToken(raw: string): string {
  const normalized = raw
    .trim()
    .toLowerCase()
    .replace(/[\s./]+/g, "-")
    .replace(/[^a-z0-9_-]/g, "")
    .replace(/^-+|-+$/g, "");

  return normalized || "unknown";
}

/**
 * Formats an asset number with the three-digit legacy subject width.
 *
 * @param asset - Asset number.
 * @returns The number zero-padded to at least three digits, e.g. `7` gives `007`.
 *   Longer numbers are not truncated.
 */
function padAsset(asset: number): string {
  return String(asset).padStart(3, "0");
}

/**
 * Formats a LabJack channel number as the `chNN` token used in subjects.
 *
 * @param channel - LabJack channel number.
 * @returns `ch` followed by the number zero-padded to at least two digits.
 *
 * @example
 * ```ts
 * padChannel(3);   // "ch03"
 * padChannel(120); // "ch120"
 * ```
 */
export function padChannel(channel: number): string {
  return `ch${String(channel).padStart(2, "0")}`;
}

/**
 * Reports whether a config uses the structured site/box/source layout.
 *
 * @param config - LabJack config.
 * @returns `true` if the trimmed root is `avenars` or any of `site_id`, `box_id` or
 *   `source_id` is non-empty.
 */
function usesStructuredNamespace(config: LabJackSubjectConfig): boolean {
  return config.nats_subject.trim() === "avenars" || Boolean(config.site_id || config.box_id || config.source_id);
}

/**
 * Builds the live-data subject for one LabJack channel.
 *
 * Legacy configs give `<root>.<asset>.data.chNN`, with the root used as configured
 * and the asset padded to three digits. Structured configs give
 * `<root>.<site>.<box>.<source>.live.chNN` with every token sanitized. Missing
 * values fall back to `unknown-site` and `unknown-box`, and the source falls back
 * from `source_id` to `labjack_name` to `asset<NNN>`.
 *
 * @param config - LabJack config.
 * @param channel - LabJack channel number.
 * @returns The subject the streamer publishes that channel's FlatBuffer `Scan`
 *   messages on.
 *
 * @example
 * ```ts
 * liveLabJackChannelSubject({ nats_subject: "avenabox", asset_number: 1456 }, 11);
 * // "avenabox.1456.data.ch11"
 * liveLabJackChannelSubject({
 *   nats_subject: "avenars", asset_number: 1456,
 *   site_id: "i69", box_id: "i69-mu1", source_id: "i69-lj2",
 * }, 11);
 * // "avenars.i69.i69-mu1.i69-lj2.live.ch11"
 * ```
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

/**
 * Builds a label describing the live subjects of all channels of one source.
 *
 * Same layout and fallbacks as {@link liveLabJackChannelSubject}, with `ch##` in
 * place of the channel token. `ch##` is not a NATS wildcard, so the result is for
 * display only and cannot be subscribed to.
 *
 * @param config - LabJack config.
 * @returns For example `avenars.i69.i69-mu1.i69-lj2.live.ch##`.
 */
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

/**
 * Builds the key of a LabJack config in the KV bucket `avenabox`.
 *
 * Every token is sanitized. Missing values fall back to `unknown-site`,
 * `unknown-box`, and for the source from `source_id` to `labjack_name` to
 * `unknown-source`. Unlike the subject builders there is no `asset<NNN>` fallback.
 *
 * @param config - Identity fields of the config.
 * @returns `<site>.<box>.<source>.config`, e.g. `i69.i69-mu1.i69-lj2.config`.
 */
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

/**
 * Builds the export request subject for one archived source.
 *
 * Always uses the structured layout, even for legacy configs, with the same token
 * sanitizing and fallbacks as {@link liveLabJackChannelSubject}. The exporter on the
 * edge box listens on this subject.
 *
 * @param config - LabJack config.
 * @returns `<root>.<site>.<box>.<source>.export.request`, e.g.
 *   `avenars.i69.i69-mu1.i69-lj2.export.request`.
 */
export function archiveExportRequestSubject(config: LabJackSubjectConfig): string {
  const root = sanitizeToken(config.nats_subject);
  const siteId = sanitizeToken(config.site_id || "unknown-site");
  const boxId = sanitizeToken(config.box_id || "unknown-box");
  const sourceId = sanitizeToken(
    config.source_id || config.labjack_name || `asset${padAsset(config.asset_number)}`
  );

  return `${root}.${siteId}.${boxId}.${sourceId}.export.request`;
}
