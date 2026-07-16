// SPDX-License-Identifier: AGPL-3.0-or-later
import { appendFileSync, readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import {
  MIRA_INDEX_PUBLIC_KEY_HEX,
  MIRA_INDEX_PUBLISHER_KEY_ID,
  RILL_INDEX_PUBLIC_KEY_HEX,
  RILL_INDEX_PUBLISHER_KEY_ID,
  compareVersions,
  parseStableVersion,
  verifySignedIndex,
} from './signed-release-index.mjs';

const [currentIndexArg, rillIndexArg, outputArg] = process.argv.slice(2);
if (!currentIndexArg || !rillIndexArg || !outputArg) {
  throw new Error(
    'usage: build-rill-runtime-release-index.mjs CURRENT_MIRA_INDEX RILL_INDEX OUTPUT_PAYLOAD',
  );
}

const currentIndex = JSON.parse(readFileSync(resolve(currentIndexArg), 'utf8'));
const rillIndex = JSON.parse(readFileSync(resolve(rillIndexArg), 'utf8'));
verifySignedIndex(
  currentIndex,
  MIRA_INDEX_PUBLISHER_KEY_ID,
  MIRA_INDEX_PUBLIC_KEY_HEX,
  'current Mira local AI index',
);
verifySignedIndex(
  rillIndex,
  RILL_INDEX_PUBLISHER_KEY_ID,
  RILL_INDEX_PUBLIC_KEY_HEX,
  'latest Rill stable index',
);
if (
  currentIndex.payload.schemaVersion !== 2 ||
  currentIndex.payload.channel !== 'stable' ||
  !Array.isArray(currentIndex.payload.artifacts)
) {
  throw new Error('current Mira local AI index does not use the supported stable schema');
}

const currentRuntimes = currentIndex.payload.artifacts.filter(
  (artifact) => artifact.kind === 'runtime' && artifact.id === 'rill-runtime',
);
const latestRuntimes = rillIndex.payload.artifacts.filter(
  (artifact) => artifact.kind === 'runtime' && artifact.id === 'rill-runtime',
);
const currentVersions = new Set(currentRuntimes.map((artifact) => artifact.version));
const latestVersions = new Set(latestRuntimes.map((artifact) => artifact.version));
if (currentVersions.size !== 1 || latestVersions.size !== 1) {
  throw new Error('release index mixes Rill runtime versions');
}
const [currentVersion] = currentVersions;
const [latestVersion] = latestVersions;
parseStableVersion(currentVersion, 'current Rill runtime version');
parseStableVersion(latestVersion, 'latest Rill runtime version');
if (compareVersions(latestVersion, currentVersion) < 0) {
  throw new Error('Rill latest release would downgrade the current Mira runtime');
}

const targets = [
  ['macos', 'aarch64'],
  ['linux', 'x86_64'],
  ['windows', 'x86_64'],
];
const selectedRuntimes = targets.map(([targetOs, targetArch]) => {
  const matches = latestRuntimes.filter(
    (artifact) => artifact.targetOs === targetOs && artifact.targetArch === targetArch,
  );
  if (matches.length !== 1) {
    throw new Error(`latest Rill index must contain one runtime for ${targetOs}-${targetArch}`);
  }
  const [artifact] = matches;
  if (
    artifact.runtimeApiVersion !== 2 ||
    !artifact.url.startsWith(
      `https://github.com/hello-yunshu/rill-ml/releases/download/v${latestVersion}/`,
    ) ||
    !/^[0-9a-f]{64}$/i.test(artifact.sha256) ||
    !Number.isSafeInteger(artifact.size) ||
    artifact.size <= 0
  ) {
    throw new Error(`latest Rill runtime contract is invalid for ${targetOs}-${targetArch}`);
  }
  return artifact;
});

const models = currentIndex.payload.artifacts.filter(
  (artifact) => artifact.kind === 'model' && artifact.id === 'mira-battery-model',
);
const handlers = currentIndex.payload.artifacts.filter(
  (artifact) => artifact.kind === 'handler' && artifact.id === 'mira.battery.handler',
);
if (models.length !== 1 || handlers.length !== 1) {
  throw new Error('current Mira index must contain one model and one handler');
}
const minimumRuntimeVersion = handlers[0].minRuntimeVersion;
parseStableVersion(minimumRuntimeVersion, 'handler minimum runtime version');
if (compareVersions(latestVersion, minimumRuntimeVersion) < 0) {
  throw new Error('latest Rill runtime is older than the current handler minimum');
}

const updateAvailable = compareVersions(latestVersion, currentVersion) > 0;
const payload = {
  ...currentIndex.payload,
  generatedAt: new Date().toISOString(),
  publisherKeyId: MIRA_INDEX_PUBLISHER_KEY_ID,
  artifacts: [
    ...selectedRuntimes,
    ...currentIndex.payload.artifacts.filter((artifact) => artifact.kind !== 'runtime'),
  ],
};
writeFileSync(resolve(outputArg), `${JSON.stringify(payload, null, 2)}\n`);

const outputs = {
  'current-version': currentVersion,
  'latest-version': latestVersion,
  'latest-tag': `v${latestVersion}`,
  'update-available': String(updateAvailable),
};
if (process.env.GITHUB_OUTPUT) {
  appendFileSync(
    process.env.GITHUB_OUTPUT,
    `${Object.entries(outputs)
      .map(([key, value]) => `${key}=${value}`)
      .join('\n')}\n`,
  );
}
console.log(JSON.stringify(outputs));
