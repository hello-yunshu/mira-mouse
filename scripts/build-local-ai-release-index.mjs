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
const releaseUrl = (path) =>
  `https://github.com/hello-yunshu/mira-mouse/releases/download/${tag}/${basename(path)}`;
const describe = (path) => {
  const bytes = readFileSync(path);
  if (bytes.length === 0) throw new Error(`empty local AI artifact: ${path}`);
  return {
    url: releaseUrl(path),
    sha256: createHash('sha256').update(bytes).digest('hex'),
    size: statSync(path).size,
  };
};

const targets = [
  ['macos', 'aarch64', 'rill-runtime-macos-aarch64'],
  ['macos', 'x86_64', 'rill-runtime-macos-x86_64'],
  ['linux', 'x86_64', 'rill-runtime-linux-x86_64'],
  ['windows', 'x86_64', 'rill-runtime-windows-x86_64.exe'],
];
const runtimeArtifacts = targets.map(([targetOs, targetArch, filename]) => ({
  kind: 'runtime',
  id: 'rill-runtime',
  version: runtimeVersion,
  runtimeApiVersion: 2,
  targetOs,
  targetArch,
  ...describe(resolve(assetsDir, filename)),
}));

const modelPath = resolve(assetsDir, 'model.rillpack');
const handlerPath = resolve(assetsDir, 'handler.rillhandler');
const artifacts = [
  ...runtimeArtifacts,
  {
    kind: 'model',
    id: 'mira-battery-model',
    version: modelVersion,
    runtimeApiVersion: 2,
    ...describe(modelPath),
  },
  {
    kind: 'handler',
    id: 'mira.battery.handler',
    version: handlerVersion,
    runtimeApiVersion: 2,
    handlerApiVersion: 1,
    minRuntimeVersion: '0.7.0',
    ...describe(handlerPath),
  },
];

writeFileSync(resolve(outputArg), `${JSON.stringify({
  schemaVersion: 2,
  channel: 'stable',
  generatedAt: new Date().toISOString(),
  publisherKeyId: 'mira-rill-2026-002',
  artifacts,
}, null, 2)}\n`);
