// SPDX-License-Identifier: AGPL-3.0-or-later
import { appendFileSync, readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import {
  RILL_INDEX_PUBLIC_KEY_HEX,
  RILL_INDEX_PUBLISHER_KEY_ID,
  parseStableVersion,
  verifySignedIndex,
} from './signed-release-index.mjs';

const RELEASE_BASE = 'https://github.com/hello-yunshu/rill-ml/releases';
const LATEST_INDEX_URL = `${RELEASE_BASE}/latest/download/stable-index.json`;
const TARGETS = [
  ['macos', 'aarch64'],
  ['linux', 'x86_64'],
  ['windows', 'x86_64'],
];

const args = process.argv.slice(2);
let indexPath;
let outputPath;
let requestedVersion;
for (let index = 0; index < args.length; index += 1) {
  if (args[index] === '--index') indexPath = args[++index];
  else if (args[index] === '--output') outputPath = args[++index];
  else if (args[index] === '--version') requestedVersion = args[++index];
  else throw new Error(`unknown argument: ${args[index]}`);
}

if (requestedVersion) parseStableVersion(requestedVersion, 'requested Rill version');
const indexUrl = requestedVersion
  ? `${RELEASE_BASE}/download/v${requestedVersion}/stable-index.json`
  : LATEST_INDEX_URL;

const raw = indexPath
  ? readFileSync(resolve(indexPath))
  : Buffer.from(await fetchIndex(indexUrl));
const signedIndex = JSON.parse(raw.toString('utf8'));
verifySignedIndex(
  signedIndex,
  RILL_INDEX_PUBLISHER_KEY_ID,
  RILL_INDEX_PUBLIC_KEY_HEX,
  'Rill latest stable index',
);

const payload = signedIndex.payload;
if (payload.schemaVersion !== 2 || payload.channel !== 'stable' || !Array.isArray(payload.artifacts)) {
  throw new Error('Rill latest index does not use the supported stable schema');
}
const runtimeArtifacts = payload.artifacts.filter(
  (artifact) => artifact.kind === 'runtime' && artifact.id === 'rill-runtime',
);
const versions = new Set(runtimeArtifacts.map((artifact) => artifact.version));
if (versions.size !== 1) throw new Error('Rill latest index mixes runtime versions');
const [version] = versions;
parseStableVersion(version, 'Rill runtime version');
if (requestedVersion && version !== requestedVersion) {
  throw new Error('requested Rill version does not match its signed stable index');
}
const expectedPrefix = `${RELEASE_BASE}/download/v${version}/`;
for (const [targetOs, targetArch] of TARGETS) {
  const matches = runtimeArtifacts.filter(
    (artifact) => artifact.targetOs === targetOs && artifact.targetArch === targetArch,
  );
  if (matches.length !== 1) {
    throw new Error(`Rill latest index must contain one runtime for ${targetOs}-${targetArch}`);
  }
  const [artifact] = matches;
  if (
    artifact.runtimeApiVersion !== 2 ||
    !artifact.url.startsWith(expectedPrefix) ||
    !/^[0-9a-f]{64}$/i.test(artifact.sha256) ||
    !Number.isSafeInteger(artifact.size) ||
    artifact.size <= 0
  ) {
    throw new Error(`Rill runtime contract is invalid for ${targetOs}-${targetArch}`);
  }
}

if (outputPath) writeFileSync(resolve(outputPath), raw);
const result = {
  version,
  tag: `v${version}`,
  indexUrl,
};
if (process.env.GITHUB_OUTPUT) {
  appendFileSync(
    process.env.GITHUB_OUTPUT,
    `version=${result.version}\ntag=${result.tag}\nindex-url=${result.indexUrl}\n`,
  );
}
console.log(JSON.stringify(result));

async function fetchIndex(url) {
  const response = await fetch(url, {
    headers: { 'user-agent': 'mira-rill-release-resolver/1' },
    redirect: 'follow',
  });
  if (!response.ok) {
    throw new Error(`fetch Rill stable index: ${response.status} ${response.statusText}`);
  }
  return response.arrayBuffer();
}
