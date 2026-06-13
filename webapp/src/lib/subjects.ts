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
  return config.nats_subject.trim() === "avenars" || Boolean(config.box_id || config.source_id);
}

export function liveLabJackChannelSubject(config: LabJackSubjectConfig, channel: number): string {
  if (!usesV1Namespace(config)) {
    return `${config.nats_subject}.${padAsset(config.asset_number)}.data.${padChannel(channel)}`;
  }

  const root = sanitizeToken(config.nats_subject);
  const boxId = sanitizeToken(config.box_id || "unknown-box");
  const sourceId = sanitizeToken(
    config.source_id || config.labjack_name || `asset${padAsset(config.asset_number)}`
  );

  return `${root}.v1.${boxId}.${sourceId}.${padChannel(channel)}`;
}

export function liveLabJackChannelPattern(config: LabJackSubjectConfig): string {
  if (!usesV1Namespace(config)) {
    return `${config.nats_subject}.${padAsset(config.asset_number)}.data.ch##`;
  }

  const root = sanitizeToken(config.nats_subject);
  const boxId = sanitizeToken(config.box_id || "unknown-box");
  const sourceId = sanitizeToken(
    config.source_id || config.labjack_name || `asset${padAsset(config.asset_number)}`
  );

  return `${root}.v1.${boxId}.${sourceId}.ch##`;
}

export function labjackConfigKey(config: {
  box_id?: string | null;
  source_id?: string | null;
  labjack_name?: string | null;
}): string {
  const boxId = sanitizeToken(config.box_id || "unknown-box");
  const sourceId = sanitizeToken(config.source_id || config.labjack_name || "unknown-source");
  return `v1.${boxId}.${sourceId}.config`;
}
