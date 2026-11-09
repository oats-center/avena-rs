#!/usr/bin/env node

import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import transportPkg from "../webapp/node_modules/@nats-io/transport-node/lib/mod.js";
import corePkg from "../webapp/node_modules/@nats-io/nats-core/lib/mod.js";

const { connect } = transportPkg;
const { createInbox, credsAuthenticator } = corePkg;

function usage() {
  console.error(`Usage: scripts/request-nats-export.mjs \\
  --subject <export.request subject> --asset <number> --channels <11,13> \\
  --start <RFC3339> --end <RFC3339> --output <file.csv> [options]

Options:
  --servers <comma-separated URLs>  default: nats1.oats,nats2.oats
  --creds <path>                    default: /etc/avena-rs/apt.creds
  --report <path>                   save the JSON result as well
  --timeout-seconds <number>        default: 120`);
}

function parseArgs(argv) {
  const options = {};
  for (let index = 0; index < argv.length; index += 2) {
    const name = argv[index];
    const value = argv[index + 1];
    if (!name?.startsWith("--") || value === undefined) {
      usage();
      process.exit(2);
    }
    options[name.slice(2)] = value;
  }
  return options;
}

const options = parseArgs(process.argv.slice(2));
for (const required of ["subject", "asset", "channels", "start", "end", "output"]) {
  if (!options[required]) {
    console.error(`Missing --${required}`);
    usage();
    process.exit(2);
  }
}

const asset = Number(options.asset);
const channels = options.channels.split(",").map(Number);
const timeoutSeconds = Number(options["timeout-seconds"] ?? "120");
if (!Number.isInteger(asset) || channels.some((channel) => !Number.isInteger(channel))) {
  throw new Error("asset and channels must be integers");
}

const servers = (options.servers ?? "nats://nats1.oats:4222,nats://nats2.oats:4222")
  .split(",")
  .map((server) => server.trim())
  .filter(Boolean);
const credsPath = options.creds ?? "/etc/avena-rs/apt.creds";
const nc = await connect({
  servers,
  authenticator: credsAuthenticator(readFileSync(credsPath)),
  name: "avena-nats-export-check",
});

const reply = createInbox();
const ackSubject = createInbox();
const subscription = nc.subscribe(reply);
const chunks = [];
let metadata = null;
let summary = null;
let byteCount = 0;
let chunkCount = 0;

const request = {
  asset,
  channels,
  start: options.start,
  end: options.end,
  format: "csv",
  download_name: options.output.split("/").at(-1),
  ack_subject: ackSubject,
};

const timeout = setTimeout(() => {
  subscription.unsubscribe();
}, timeoutSeconds * 1_000);

try {
  nc.publish(
    options.subject,
    new TextEncoder().encode(JSON.stringify(request)),
    { reply },
  );
  await nc.flush();

  for await (const message of subscription) {
    const frame = message.headers?.get("Avena-Export-Frame") ?? "chunk";
    if (frame === "chunk") {
      const chunk = Buffer.from(message.data);
      chunks.push(chunk);
      byteCount += chunk.length;
      chunkCount += 1;
      nc.publish(ackSubject);
      await nc.flush();
      continue;
    }

    const decoded = JSON.parse(new TextDecoder().decode(message.data));
    if (frame === "meta") metadata = decoded;
    if (frame === "summary") summary = decoded;
    if (frame === "error") throw new Error(decoded.message);
    if (frame === "complete") break;
  }
} finally {
  clearTimeout(timeout);
  subscription.unsubscribe();
  await nc.drain();
}

if (!summary) {
  throw new Error(`export did not complete within ${timeoutSeconds} seconds`);
}

const csv = Buffer.concat(chunks);
mkdirSync(dirname(options.output), { recursive: true });
writeFileSync(options.output, csv);
const newlineCount = csv.reduce((count, byte) => count + (byte === 10 ? 1 : 0), 0);
const report = {
  requestSubject: options.subject,
  request: { ...request, ack_subject: "generated NATS inbox" },
  metadata,
  summary,
  bytesReceived: byteCount,
  chunks: chunkCount,
  csvRowsExcludingHeader: Math.max(0, newlineCount - 1),
  output: options.output,
};

if (options.report) {
  mkdirSync(dirname(options.report), { recursive: true });
  writeFileSync(options.report, `${JSON.stringify(report, null, 2)}\n`);
}
console.log(JSON.stringify(report, null, 2));
