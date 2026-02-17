#!/usr/bin/env node

function parseArgs(argv) {
  const result = {};
  for (let i = 0; i < argv.length; i += 1) {
    const token = argv[i];
    if (!token.startsWith('--')) continue;
    const key = token.slice(2);
    const value = argv[i + 1];
    if (!value || value.startsWith('--')) {
      result[key] = 'true';
      continue;
    }
    result[key] = value;
    i += 1;
  }
  return result;
}

async function parseErrorBody(response) {
  try {
    const parsed = await response.json();
    if (parsed && typeof parsed.error === 'string') return parsed.error;
    return JSON.stringify(parsed);
  } catch {
    return await response.text();
  }
}

function parseIsoMs(value) {
  const ms = new Date(value).getTime();
  return Number.isFinite(ms) ? ms : Number.NaN;
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function fetchCameras(baseUrl, asset) {
  const cameraResponse = await fetch(
    `${baseUrl}/api/video/cameras?asset=${encodeURIComponent(String(asset))}`
  );
  if (!cameraResponse.ok) {
    const detail = await parseErrorBody(cameraResponse);
    throw new Error(`camera endpoint failed (${cameraResponse.status}): ${detail}`);
  }
  return await cameraResponse.json();
}

function getCoverageForCamera(cameraBody, cameraId) {
  const coverage = Array.isArray(cameraBody?.coverage) ? cameraBody.coverage : [];
  return coverage.find((entry) => entry?.camera_id === cameraId);
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const baseUrl = args.base ?? process.env.WEBAPP_BASE_URL ?? 'http://127.0.0.1:5173';
  const assetRaw = args.asset ?? process.env.ASSET_NUMBER ?? '1001';
  const asset = Number(assetRaw);
  if (!Number.isInteger(asset) || asset <= 0) {
    throw new Error(`Invalid asset '${assetRaw}'. Expected positive integer.`);
  }

  console.log(`[smoke] checking cameras via ${baseUrl}/api/video/cameras?asset=${asset}`);
  const cameraBody = await fetchCameras(baseUrl, asset);
  const cameras = Array.isArray(cameraBody?.cameras) ? cameraBody.cameras : [];
  console.log(`[smoke] cameras discovered: ${cameras.join(', ') || '(none)'}`);

  const cameraId = args.camera_id ?? process.env.CAMERA_ID ?? cameras[0];
  const waitCoverageSec = Number(args.wait_coverage_sec ?? process.env.WAIT_COVERAGE_SEC ?? '60');
  const pollSec = Number(args.poll_sec ?? process.env.POLL_SEC ?? '2');
  const useSafeCenter = (args.use_safe_center ?? process.env.USE_SAFE_CENTER ?? '1') !== '0';

  if (!cameraId) {
    console.log('[smoke] skipping clip fetch: no cameras available yet');
    return;
  }

  let centerTime = args.center_time ?? process.env.CENTER_TIME ?? '';
  const deadline = Date.now() + Math.max(0, waitCoverageSec) * 1000;

  while (Date.now() <= deadline) {
    const currentBody = await fetchCameras(baseUrl, asset);
    const coverage = getCoverageForCamera(currentBody, cameraId);
    if (!coverage) {
      if (centerTime) break;
      await sleep(Math.max(0.1, pollSec) * 1000);
      continue;
    }
    const minMs = parseIsoMs(coverage.recommended_center_min);
    const maxMs = parseIsoMs(coverage.recommended_center_max);
    const hasWindow = Number.isFinite(minMs) && Number.isFinite(maxMs) && maxMs >= minMs;
    if (!hasWindow) {
      if (centerTime && !useSafeCenter) break;
      await sleep(Math.max(0.1, pollSec) * 1000);
      continue;
    }

    if (!centerTime) {
      centerTime = coverage.recommended_center_max;
      console.log(
        `[smoke] selected safe center_time=${centerTime} from coverage window ${coverage.recommended_center_min}..${coverage.recommended_center_max}`
      );
      break;
    }

    if (useSafeCenter) {
      const centerMs = parseIsoMs(centerTime);
      if (!Number.isFinite(centerMs) || centerMs > maxMs) {
        centerTime = coverage.recommended_center_max;
        console.log(
          `[smoke] adjusted center_time to coverage max ${centerTime} (upload/processing lag guard)`
        );
      }
    }
    break;
  }

  if (!centerTime) {
    throw new Error(
      `no fetchable center_time available for camera ${cameraId} after waiting ${waitCoverageSec}s`
    );
  }

  const preSec = Number(args.pre_sec ?? process.env.PRE_SEC ?? '5');
  const postSec = Number(args.post_sec ?? process.env.POST_SEC ?? '5');
  const clipPayload = {
    asset,
    center_time: centerTime,
    pre_sec: preSec,
    post_sec: postSec
  };
  if (cameraId) {
    clipPayload.camera_id = cameraId;
  }

  console.log(
    `[smoke] checking clip fetch via ${baseUrl}/api/video/clip center_time=${centerTime} camera=${cameraId ?? '(auto)'}`
  );
  const clipResponse = await fetch(`${baseUrl}/api/video/clip`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(clipPayload)
  });
  if (!clipResponse.ok) {
    const detail = await parseErrorBody(clipResponse);
    throw new Error(`clip endpoint failed (${clipResponse.status}): ${detail}`);
  }

  const clipBytes = await clipResponse.arrayBuffer();
  console.log(
    `[smoke] clip fetch OK: ${clipBytes.byteLength} bytes, content-type=${clipResponse.headers.get('Content-Type') ?? 'unknown'}`
  );
}

main().catch((error) => {
  console.error(`[smoke] FAILED: ${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 1;
});
