// SPDX-License-Identifier: AGPL-3.0-or-later
import { existsSync, readFileSync, statSync } from 'node:fs';
import { resolve } from 'node:path';
import { spawnSync } from 'node:child_process';

const modelPack = resolve('src-tauri/resources/local-ai/model.rillpack');
const handlerPack = resolve('src-tauri/resources/local-ai/handler.rillhandler');
const modelManifest = JSON.parse(readFileSync(resolve('local-ai/model-manifest.json'), 'utf8'));
const handlerManifest = JSON.parse(
  readFileSync(resolve('handlers/mira-battery-handler/manifest.template.json'), 'utf8'),
);
const workspaceVersion = readFileSync(resolve('Cargo.toml'), 'utf8').match(
  /\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"/m,
)?.[1];
if (!workspaceVersion) throw new Error('Cargo.toml is missing the workspace package version.');
const modelTrustKey =
  'mira-rill-2026-002=ae4633988fd9e02a824bb9072f1dcf470a0b1d74bbc7905aaea00a62139e1479';
const handlerTrustKey =
  'mira-handler-2026-001=cefbe96db58196e4b3a8455f427ca75efaedacb68792ae82d7d06dd8c86f193e';

function requiredSize(path, message) {
  let size = 0;
  try {
    size = statSync(path).size;
  } catch {
    // The common error below also explains how CI/local builds should provide it.
  }
  if (size === 0) throw new Error(message);
  return size;
}

const modelSize = requiredSize(
  modelPack,
  'Local AI model pack is missing or empty. Build/download a signed model.rillpack before running a Tauri bundle build.',
);
const handlerSize = requiredSize(
  handlerPack,
  'Local AI handler pack is missing or empty. Build/download a signed handler.rillhandler before running a Tauri bundle build.',
);

function runtimeCandidates() {
  if (process.env.RILL_RUNTIME_PATH) return [resolve(process.env.RILL_RUNTIME_PATH)];
  const candidates = [];
  if (process.platform === 'darwin') {
    if (process.arch !== 'arm64') {
      throw new Error('Mira macOS builds support Apple Silicon (ARM64) only.');
    }
    candidates.push(resolve('src-tauri/binaries/rill-runtime-aarch64-apple-darwin'));
  } else if (process.platform === 'linux' && process.arch === 'x64') {
    candidates.push(resolve('src-tauri/binaries/rill-runtime-x86_64-unknown-linux-gnu'));
  } else if (process.platform === 'win32' && process.arch === 'x64') {
    candidates.push(resolve('src-tauri/binaries/rill-runtime-x86_64-pc-windows-msvc.exe'));
  }
  return candidates;
}

const runtime = runtimeCandidates().find(existsSync);
if (!runtime || statSync(runtime).size === 0) {
  throw new Error(
    'Rill runtime sidecar is missing or empty. Run npm run sidecar:build before a Tauri bundle build.',
  );
}

function inspect(args, label) {
  const result = spawnSync(runtime, args, { encoding: 'utf8' });
  if (result.status !== 0) {
    throw new Error(
      `${label} verification failed with the bundled Rill runtime: ${(result.stderr || result.stdout).trim()}`,
    );
  }
  return JSON.parse(result.stdout);
}

function parseStableVersion(value, label) {
  const match = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.exec(value);
  if (!match) throw new Error(`${label} must be a stable semantic version.`);
  return match.slice(1).map(Number);
}

function versionAtLeast(value, minimum) {
  const left = parseStableVersion(value, 'Runtime version');
  const right = parseStableVersion(minimum, 'Minimum Runtime version');
  for (let index = 0; index < 3; index += 1) {
    if (left[index] !== right[index]) return left[index] > right[index];
  }
  return true;
}

const model = inspect(
  ['inspect-pack', '--pack', modelPack, '--trust-key', modelTrustKey],
  'Local AI model pack',
);
const handler = inspect(
  ['inspect-handler', '--handler', handlerPack, '--handler-trust-key', handlerTrustKey],
  'Local AI handler pack',
);
if (
  model.id !== 'mira-battery-model' ||
  model.version !== modelManifest.version ||
  model.publisherKeyId !== 'mira-rill-2026-002' ||
  model.signatureVerified !== true
) {
  throw new Error('Local AI model pack identity or publisher does not match Mira production trust.');
}
if (
  handler.id !== 'mira.battery.handler' ||
  handler.version !== handlerManifest.version ||
  handler.publisherKeyId !== 'mira-handler-2026-001' ||
  handler.handlerApiVersion !== 1 ||
  typeof handler.minRuntimeVersion !== 'string' ||
  handler.signatureVerified !== true
) {
  throw new Error('Local AI handler pack identity, API, minimum runtime or publisher is invalid.');
}

const runtimeVersion = spawnSync(runtime, ['--version'], { encoding: 'utf8' });
const runtimeVersionMatch = runtimeVersion.stdout.trim().match(/^rill-runtime (\d+\.\d+\.\d+)$/);
if (runtimeVersion.status !== 0 || !runtimeVersionMatch) {
  throw new Error('Bundled Rill runtime must report a stable semantic version.');
}
const runtimeSemver = runtimeVersionMatch[1];
if (
  !versionAtLeast(runtimeSemver, handler.minRuntimeVersion) ||
  !versionAtLeast(runtimeSemver, handlerManifest.minRuntimeVersion) ||
  !versionAtLeast(runtimeSemver, modelManifest.minRuntimeVersion)
) {
  throw new Error('Bundled Rill runtime is older than the model or handler minimum version.');
}
const handshakeResult = spawnSync(
  runtime,
  [
    'serve',
    '--pack',
    modelPack,
    '--handler',
    handlerPack,
    '--trust-key',
    modelTrustKey,
    '--handler-trust-key',
    handlerTrustKey,
  ],
  {
    encoding: 'utf8',
    input: `${JSON.stringify({
      method: 'handshake',
      requestId: 'asset-check',
      apiVersion: 2,
      clientName: 'mira-build',
      clientVersion: workspaceVersion,
    })}\n`,
    timeout: 10_000,
  },
);
if (handshakeResult.status !== 0) {
  throw new Error(
    `Local AI runtime/model/handler handshake failed: ${(handshakeResult.stderr || handshakeResult.stdout).trim()}`,
  );
}
const handshake = JSON.parse(handshakeResult.stdout.trim());
if (
  handshake.kind !== 'handshake' ||
  handshake.runtimeVersion !== runtimeSemver ||
  handshake.modelPackId !== model.id ||
  handshake.modelPackVersion !== model.version ||
  handshake.handlerId !== handler.id ||
  handshake.handlerVersion !== handler.version ||
  handshake.handlerApiVersion !== 1 ||
  !handshake.effectiveCapabilities?.includes('batteryUsage')
) {
  throw new Error('Local AI runtime/model/handler handshake contract does not match Mira.');
}

console.log(`local AI model pack: ${modelSize} bytes`);
console.log(`local AI handler pack: ${handlerSize} bytes`);
console.log(
  `local AI contract: runtime ${runtimeVersion.stdout.trim()}, model ${model.version}, handler ${handler.version}`,
);
