export interface LabJackSubjectConfig {
  nats_subject: string;
  asset_number: number;
  labjack_name?: string;
  site_id?: string | null;
  box_id?: string | null;
  source_type?: string | null;
  source_id?: string | null;
}

function sanitizeToken(raw: string): string {
  const normalized = raw
    .trim()
    .toLowerCase()
    .replace(/[\s./]+/g, "-")
    .replace(/[^a-z0-9_-]/g, "")
    .replace(/^-+|-+$/g, "");

  return normalized || "unknown";
}

function padAsset(asset: number): string {
  return String(asset).padStart(3, "0");
}

export function padChannel(channel: number): string {
  return `ch${String(channel).padStart(2, "0")}`;
}

function usesV1Namespace(config: LabJackSubjectConfig): boolean {
  return config.nats_subject.trim() === "avenars" || Boolean(config.site_id || config.box_id);
}

export function liveLabJackChannelSubject(config: LabJackSubjectConfig, channel: number): string {
  if (!usesV1Namespace(config)) {
    return `${config.nats_subject}.${padAsset(config.asset_number)}.data.${padChannel(channel)}`;
  }

  const root = sanitizeToken(config.nats_subject);
  const siteId = sanitizeToken(config.site_id || "unknown-site");
  const boxId = sanitizeToken(config.box_id || "unknown-box");
  const sourceType = sanitizeToken(config.source_type || "labjack");
  const sourceId = sanitizeToken(
    config.source_id || config.labjack_name || `asset${padAsset(config.asset_number)}`
  );

  return `${root}.v1.${siteId}.${boxId}.live.${sourceType}.${sourceId}.sample.${padChannel(channel)}`;
}

export function liveLabJackChannelPattern(config: LabJackSubjectConfig): string {
  if (!usesV1Namespace(config)) {
    return `${config.nats_subject}.${padAsset(config.asset_number)}.data.ch##`;
  }

  const root = sanitizeToken(config.nats_subject);
  const siteId = sanitizeToken(config.site_id || "unknown-site");
  const boxId = sanitizeToken(config.box_id || "unknown-box");
  const sourceType = sanitizeToken(config.source_type || "labjack");
  const sourceId = sanitizeToken(
    config.source_id || config.labjack_name || `asset${padAsset(config.asset_number)}`
  );

  return `${root}.v1.${siteId}.${boxId}.live.${sourceType}.${sourceId}.sample.ch##`;
}
