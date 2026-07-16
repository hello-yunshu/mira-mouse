// SPDX-License-Identifier: AGPL-3.0-or-later
import { createHash, createPublicKey, verify } from 'node:crypto';
import { readFileSync, statSync, writeFileSync } from 'node:fs';
import { basename, resolve } from 'node:path';

const [currentIndexArg, handlerArg, handlerVersion, releaseTag, outputArg] = process.argv.slice(2);
if (!currentIndexArg || !handlerArg || !handlerVersion || !releaseTag || !outputArg) {
  throw new Error(
    'usage: build-handler-release-index.mjs CURRENT_INDEX HANDLER VERSION RELEASE_TAG OUTPUT',
  );
}

const MIRA_INDEX_PUBLISHER_KEY_ID = 'mira-rill-2026-002';
const MIRA_INDEX_PUBLIC_KEY_HEX = 'ae4633988fd9e02a824bb9072f1dcf470a0b1d74bbc7905aaea00a62139e1479';
const HANDLER_ID = 'mira.battery.handler';
const MODEL_ID = 'mira-battery-model';
const handlerTemplate = JSON.parse(
  readFileSync(resolve('handlers/mira-battery-handler/manifest.template.json'), 'utf8'),
);
const MIN_RUNTIME_VERSION = handlerTemplate.minRuntimeVersion;

const canonicalize = (value) => {
  if (Array.isArray(value)) return value.map(canonicalize);
  if (value && typeof value === 'object') {
    return Object.fromEntries(
      Object.entries(value)
        .sort(([left], [right]) => (left < right ? -1 : left > right ? 1 : 0))
        .map(([key, item]) => [key, canonicalize(item)]),
    );
  }
  return value;
};

function verifySignedIndex(index) {
  if (index?.payload?.publisherKeyId !== MIRA_INDEX_PUBLISHER_KEY_ID) {
    throw new Error('current local AI index uses an untrusted publisher');
  }
  if (typeof index.signature !== 'string' || !/^[0-9a-f]{128}$/i.test(index.signature)) {
    throw new Error('current local AI index has an invalid signature encoding');
  }
  const publicKey = createPublicKey({
    key: Buffer.concat([
      Buffer.from('302a300506032b6570032100', 'hex'),
      Buffer.from(MIRA_INDEX_PUBLIC_KEY_HEX, 'hex'),
    ]),
    format: 'der',
    type: 'spki',
  });
  const payload = Buffer.from(JSON.stringify(canonicalize(index.payload)));
  if (!verify(null, payload, publicKey, Buffer.from(index.signature, 'hex'))) {
    throw new Error('current local AI index signature verification failed');
  }
}

function parseStableVersion(value, label) {
  const match = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.exec(value);
  if (!match) throw new Error(`${label} must be a stable semantic version`);
  return match.slice(1).map(Number);
}

function compareVersions(left, right) {
  for (let index = 0; index < 3; index += 1) {
    if (left[index] !== right[index]) return left[index] - right[index];
  }
  return 0;
}

const currentIndex = JSON.parse(readFileSync(resolve(currentIndexArg), 'utf8'));
verifySignedIndex(currentIndex);
if (currentIndex.payload.schemaVersion !== 2 || !Array.isArray(currentIndex.payload.artifacts)) {
  throw new Error('current local AI index does not use schema version 2');
}

const handlerPath = resolve(handlerArg);
const handlerBytes = readFileSync(handlerPath);
if (handlerBytes.length === 0) throw new Error('handler package is empty');

const runtimeArtifacts = currentIndex.payload.artifacts.filter(
  (artifact) => artifact.kind === 'runtime' && artifact.id === 'rill-runtime',
);
const modelArtifacts = currentIndex.payload.artifacts.filter(
  (artifact) => artifact.kind === 'model' && artifact.id === MODEL_ID,
);
const handlerArtifacts = currentIndex.payload.artifacts.filter(
  (artifact) => artifact.kind === 'handler' && artifact.id === HANDLER_ID,
);
if (runtimeArtifacts.length === 0 || modelArtifacts.length !== 1 || handlerArtifacts.length !== 1) {
  throw new Error('current local AI index is missing the complete runtime/model/handler contract');
}
const minimumRuntime = parseStableVersion(MIN_RUNTIME_VERSION, 'handler minimum runtime version');
for (const runtime of runtimeArtifacts) {
  if (compareVersions(parseStableVersion(runtime.version, 'runtime version'), minimumRuntime) < 0) {
    throw new Error('current local AI index uses a runtime older than the handler minimum');
  }
}

const nextVersion = parseStableVersion(handlerVersion, 'handler version');
const currentVersion = parseStableVersion(handlerArtifacts[0].version, 'current handler version');
if (compareVersions(nextVersion, currentVersion) <= 0) {
  throw new Error('handler-only releases must increase the published handler version');
}
if (releaseTag !== `local-ai-handler-v${handlerVersion}`) {
  throw new Error('handler release tag must be local-ai-handler-v<version>');
}

const handlerName = basename(handlerPath);
const nextHandler = {
  kind: 'handler',
  id: HANDLER_ID,
  version: handlerVersion,
  runtimeApiVersion: 2,
  targetOs: null,
  targetArch: null,
  handlerApiVersion: 1,
  minRuntimeVersion: MIN_RUNTIME_VERSION,
  url: `https://github.com/hello-yunshu/mira-mouse/releases/download/${releaseTag}/${handlerName}`,
  sha256: createHash('sha256').update(handlerBytes).digest('hex'),
  size: statSync(handlerPath).size,
};

const artifacts = currentIndex.payload.artifacts.map((artifact) =>
  artifact.kind === 'handler' && artifact.id === HANDLER_ID ? nextHandler : artifact,
);
const payload = {
  ...currentIndex.payload,
  generatedAt: new Date().toISOString(),
  publisherKeyId: MIRA_INDEX_PUBLISHER_KEY_ID,
  artifacts,
};
writeFileSync(resolve(outputArg), `${JSON.stringify(payload, null, 2)}\n`);
