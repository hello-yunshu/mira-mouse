// SPDX-License-Identifier: AGPL-3.0-or-later
import { createHash } from 'node:crypto';
import { readFileSync, statSync, writeFileSync } from 'node:fs';
import { basename, resolve } from 'node:path';

const [assetsDirArg, version, tag, outputArg] = process.argv.slice(2);
if (!assetsDirArg || !version || !tag || !outputArg) {
  throw new Error('usage: build-local-ai-release-index.mjs ASSETS_DIR VERSION TAG OUTPUT');
}

const assetsDir = resolve(assetsDirArg);
const targets = [
  ['darwin', 'aarch64'],
  ['darwin', 'x86_64'],
  ['linux', 'x86_64'],
  ['windows', 'x86_64'],
];
const artifacts = targets.map(([targetOs, targetArch]) => {
  const path = resolve(assetsDir, `local-ai-bundle-${targetOs}-${targetArch}.zip`);
  const bytes = readFileSync(path);
  if (bytes.length === 0) throw new Error(`empty local AI bundle: ${path}`);
  return {
    kind: 'runtime',
    id: 'local-ai-bundle',
    version,
    runtimeApiVersion: 1,
    targetOs,
    targetArch,
    url: `https://github.com/hello-yunshu/mira-mouse/releases/download/${tag}/${basename(path)}`,
    sha256: createHash('sha256').update(bytes).digest('hex'),
    size: statSync(path).size,
  };
});

writeFileSync(resolve(outputArg), `${JSON.stringify({
  schemaVersion: 1,
  channel: 'stable',
  generatedAt: new Date().toISOString(),
  publisherKeyId: 'mira-rill-2026-001',
  artifacts,
}, null, 2)}\n`);
