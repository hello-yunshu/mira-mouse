// SPDX-License-Identifier: AGPL-3.0-or-later
import { createHash } from 'node:crypto';
import { readFileSync, statSync, writeFileSync } from 'node:fs';
import { basename, resolve } from 'node:path';

const [assetsDirArg, runtimeVersion, modelVersion, handlerVersion, tag, outputArg] = process.argv.slice(2);
if (!assetsDirArg || !runtimeVersion || !modelVersion || !handlerVersion || !tag || !outputArg) {
  throw new Error(
    'usage: build-local-ai-release-index.mjs ASSETS_DIR RUNTIME_VERSION MODEL_VERSION HANDLER_VERSION TAG OUTPUT',
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

async function fetchRuntimeArtifacts() {
  const response = await fetch(RILL_ML_INDEX_URL);
  if (!response.ok) {
    throw new Error(`fetch rill-ml stable-index.json: ${response.status} ${response.statusText}`);
  }
  const index = await response.json();
  const artifacts = index?.payload?.artifacts;
  if (!Array.isArray(artifacts)) {
    throw new Error('rill-ml stable-index.json missing payload.artifacts array');
  }
  const targets = [
    ['macos', 'aarch64'],
    ['macos', 'x86_64'],
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
const artifacts = [
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
    minRuntimeVersion: '0.7.0',
    ...describeLocal(handlerPath),
  },
];

writeFileSync(resolve(outputArg), `${JSON.stringify({
  schemaVersion: 2,
  channel: 'stable',
  generatedAt: new Date().toISOString(),
  publisherKeyId: 'mira-rill-2026-002',
  artifacts,
}, null, 2)}\n`);
