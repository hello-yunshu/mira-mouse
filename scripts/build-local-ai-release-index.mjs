// SPDX-License-Identifier: AGPL-3.0-or-later
import { createHash, createPublicKey, verify } from 'node:crypto';
import { readFileSync, statSync, writeFileSync } from 'node:fs';
import { basename, resolve } from 'node:path';

const [assetsDirArg, runtimeVersion, modelVersion, handlerVersion, tag, outputArg, currentIndexArg] =
  process.argv.slice(2);
if (!assetsDirArg || !runtimeVersion || !modelVersion || !handlerVersion || !tag || !outputArg) {
  throw new Error(
    'usage: build-local-ai-release-index.mjs ASSETS_DIR RUNTIME_VERSION MODEL_VERSION HANDLER_VERSION TAG OUTPUT [CURRENT_INDEX]',
  );
}

const assetsDir = resolve(assetsDirArg);
const miraReleaseUrl = (path) =>
  `https://github.com/hello-yunshu/mira-mouse/releases/download/${tag}/${basename(path)}`;
const describeLocal = (path) => {
  const bytes = readFileSync(path);
  if (bytes.length === 0) throw new Error(`empty local AI artifact: ${path}`);
  return {
    url: miraReleaseUrl(path),
    sha256: createHash('sha256').update(bytes).digest('hex'),
    size: statSync(path).size,
  };
};

// Runtime 二进制由 rill-ml releases 托管，从其 stable-index.json 提取 URL/SHA-256/size。
const RILL_ML_INDEX_URL = `https://github.com/hello-yunshu/rill-ml/releases/download/v${runtimeVersion}/stable-index.json`;
const RILL_INDEX_PUBLISHER_KEY_ID = 'rillml-examples-2026-001';
const RILL_INDEX_PUBLIC_KEY_HEX = '29fd1fc2f22bd7e405aec167ff0a0d8de791f011c415075d4c5f9f64fd93fc2e';
const MIRA_INDEX_PUBLISHER_KEY_ID = 'mira-rill-2026-002';
const MIRA_INDEX_PUBLIC_KEY_HEX = 'ae4633988fd9e02a824bb9072f1dcf470a0b1d74bbc7905aaea00a62139e1479';

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

function verifySignedIndex(index, publisherKeyId, publicKeyHex, label) {
  if (index?.payload?.publisherKeyId !== publisherKeyId) {
    throw new Error(`${label} uses an untrusted publisher`);
  }
  if (typeof index.signature !== 'string' || !/^[0-9a-f]{128}$/i.test(index.signature)) {
    throw new Error(`${label} has an invalid signature encoding`);
  }
  const rawKey = Buffer.from(publicKeyHex, 'hex');
  // RFC 8410 SubjectPublicKeyInfo prefix for a raw 32-byte Ed25519 public key.
  const publicKey = createPublicKey({
    key: Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), rawKey]),
    format: 'der',
    type: 'spki',
  });
  const payload = Buffer.from(JSON.stringify(canonicalize(index.payload)));
  if (!verify(null, payload, publicKey, Buffer.from(index.signature, 'hex'))) {
    throw new Error(`${label} signature verification failed`);
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

const artifactIdentity = (artifact) =>
  [artifact.kind, artifact.id, artifact.targetOs ?? '', artifact.targetArch ?? ''].join('|');

function preserveNewerPublishedArtifacts(candidates, currentIndexPath) {
  if (!currentIndexPath) return candidates;
  const current = JSON.parse(readFileSync(resolve(currentIndexPath), 'utf8'));
  verifySignedIndex(
    current,
    MIRA_INDEX_PUBLISHER_KEY_ID,
    MIRA_INDEX_PUBLIC_KEY_HEX,
    'current Mira local AI index',
  );
  if (current.payload.schemaVersion !== 2 || !Array.isArray(current.payload.artifacts)) {
    throw new Error('current Mira local AI index does not use schema version 2');
  }
  const published = new Map(current.payload.artifacts.map((artifact) => [artifactIdentity(artifact), artifact]));
  return candidates.map((candidate) => {
    const previous = published.get(artifactIdentity(candidate));
    if (!previous) return candidate;
    const previousVersion = parseStableVersion(previous.version, 'published artifact version');
    const candidateVersion = parseStableVersion(candidate.version, 'candidate artifact version');
    // Equal versions are immutable too: preserve the already published URL and digest.
    return compareVersions(previousVersion, candidateVersion) >= 0 ? previous : candidate;
  });
}

async function fetchRuntimeArtifacts() {
  const response = await fetch(RILL_ML_INDEX_URL);
  if (!response.ok) {
    throw new Error(`fetch rill-ml stable-index.json: ${response.status} ${response.statusText}`);
  }
  const index = await response.json();
  verifySignedIndex(
    index,
    RILL_INDEX_PUBLISHER_KEY_ID,
    RILL_INDEX_PUBLIC_KEY_HEX,
    'rill-ml stable-index.json',
  );
  const artifacts = index?.payload?.artifacts;
  if (!Array.isArray(artifacts)) {
    throw new Error('rill-ml stable-index.json missing payload.artifacts array');
  }
  const targets = [
    ['macos', 'aarch64'],
    ['linux', 'x86_64'],
    ['windows', 'x86_64'],
  ];
  return targets.map(([targetOs, targetArch]) => {
    const artifact = artifacts.find(
      (a) =>
        a.kind === 'runtime' &&
        a.id === 'rill-runtime' &&
        a.targetOs === targetOs &&
        a.targetArch === targetArch,
    );
    if (!artifact) {
      throw new Error(
        `rill-ml stable-index.json has no runtime artifact for ${targetOs}-${targetArch}`,
      );
    }
    if (
      artifact.version !== runtimeVersion ||
      artifact.runtimeApiVersion !== 2 ||
      !artifact.url.startsWith(
        `https://github.com/hello-yunshu/rill-ml/releases/download/v${runtimeVersion}/`,
      )
    ) {
      throw new Error(`rill-ml runtime artifact contract mismatch for ${targetOs}-${targetArch}`);
    }
    return {
      kind: 'runtime',
      id: 'rill-runtime',
      version: runtimeVersion,
      runtimeApiVersion: 2,
      targetOs,
      targetArch,
      url: artifact.url,
      sha256: artifact.sha256,
      size: artifact.size,
    };
  });
}

const modelPath = resolve(assetsDir, 'model.rillpack');
const handlerPath = resolve(assetsDir, 'handler.rillhandler');
const runtimeArtifacts = await fetchRuntimeArtifacts();
const candidates = [
  ...runtimeArtifacts,
  {
    kind: 'model',
    id: 'mira-battery-model',
    version: modelVersion,
    runtimeApiVersion: 2,
    ...describeLocal(modelPath),
  },
  {
    kind: 'handler',
    id: 'mira.battery.handler',
    version: handlerVersion,
    runtimeApiVersion: 2,
    handlerApiVersion: 1,
    minRuntimeVersion: '0.7.1',
    ...describeLocal(handlerPath),
  },
];
const artifacts = preserveNewerPublishedArtifacts(candidates, currentIndexArg);

writeFileSync(resolve(outputArg), `${JSON.stringify({
  schemaVersion: 2,
  channel: 'stable',
  generatedAt: new Date().toISOString(),
  publisherKeyId: MIRA_INDEX_PUBLISHER_KEY_ID,
  artifacts,
}, null, 2)}\n`);
