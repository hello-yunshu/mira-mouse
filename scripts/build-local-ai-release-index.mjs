// SPDX-License-Identifier: AGPL-3.0-or-later
import { createHash } from 'node:crypto';
import { readFileSync, statSync, writeFileSync } from 'node:fs';
import { basename, resolve } from 'node:path';

const [assetsDirArg, runtimeVersion, modelPackVersion, tag, outputArg] = process.argv.slice(2);
if (!assetsDirArg || !runtimeVersion || !modelPackVersion || !tag || !outputArg) {
  throw new Error('usage: build-local-ai-release-index.mjs ASSETS_DIR RUNTIME_VERSION MODEL_PACK_VERSION TAG OUTPUT');
}

const assetsDir = resolve(assetsDirArg);
const targets = [
  ['macos', 'aarch64'],
  ['macos', 'x86_64'],
  ['linux', 'x86_64'],
  ['windows', 'x86_64'],
];
const runtimeArtifacts = targets.map(([targetOs, targetArch]) => {
  const path = resolve(assetsDir, `local-ai-bundle-${targetOs}-${targetArch}.zip`);
  const bytes = readFileSync(path);
  if (bytes.length === 0) throw new Error(`empty local AI bundle: ${path}`);
  return {
    kind: 'runtime',
    id: 'local-ai-bundle',
    version: runtimeVersion,
    runtimeApiVersion: 1,
    targetOs,
    targetArch,
    url: `https://github.com/hello-yunshu/mira-mouse/releases/download/${tag}/${basename(path)}`,
    sha256: createHash('sha256').update(bytes).digest('hex'),
    size: statSync(path).size,
  };
});

// Model artifact is platform-independent and only used for version comparison.
// The model pack lives inside the same bundle zip, so url/sha256/size reuse the
// first runtime artifact's values (clients never download the model artifact directly).
const modelArtifact = {
  kind: 'model',
  id: 'local-ai-model-pack',
  version: modelPackVersion,
  runtimeApiVersion: 1,
  url: runtimeArtifacts[0].url,
  sha256: runtimeArtifacts[0].sha256,
  size: runtimeArtifacts[0].size,
};

const artifacts = [...runtimeArtifacts, modelArtifact];

writeFileSync(resolve(outputArg), `${JSON.stringify({
  schemaVersion: 1,
  channel: 'stable',
  generatedAt: new Date().toISOString(),
  publisherKeyId: 'mira-rill-2026-001',
  artifacts,
}, null, 2)}\n`);
