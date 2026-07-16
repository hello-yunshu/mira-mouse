// SPDX-License-Identifier: AGPL-3.0-or-later
import { appendFileSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import {
  MIRA_INDEX_PUBLIC_KEY_HEX,
  MIRA_INDEX_PUBLISHER_KEY_ID,
  compareVersions,
  parseStableVersion,
  verifySignedIndex,
} from './signed-release-index.mjs';

const [indexArg] = process.argv.slice(2);
if (!indexArg) throw new Error('usage: inspect-mira-local-ai-index.mjs SIGNED_INDEX');

const signedIndex = JSON.parse(readFileSync(resolve(indexArg), 'utf8'));
verifySignedIndex(
  signedIndex,
  MIRA_INDEX_PUBLISHER_KEY_ID,
  MIRA_INDEX_PUBLIC_KEY_HEX,
  'Mira local AI index',
);
const { payload } = signedIndex;
if (payload.schemaVersion !== 2 || payload.channel !== 'stable' || !Array.isArray(payload.artifacts)) {
  throw new Error('Mira local AI index does not use the supported stable schema');
}
const runtimes = payload.artifacts.filter(
  (artifact) => artifact.kind === 'runtime' && artifact.id === 'rill-runtime',
);
const models = payload.artifacts.filter(
  (artifact) => artifact.kind === 'model' && artifact.id === 'mira-battery-model',
);
const handlers = payload.artifacts.filter(
  (artifact) => artifact.kind === 'handler' && artifact.id === 'mira.battery.handler',
);
if (runtimes.length === 0 || models.length !== 1 || handlers.length !== 1) {
  throw new Error('Mira local AI index is missing its runtime/model/handler contract');
}
const versions = new Set(runtimes.map((artifact) => artifact.version));
if (versions.size !== 1) throw new Error('Mira local AI index mixes runtime versions');
const [runtimeVersion] = versions;
parseStableVersion(runtimeVersion, 'Mira runtime version');
for (const [targetOs, targetArch] of [
  ['macos', 'aarch64'],
  ['linux', 'x86_64'],
  ['windows', 'x86_64'],
]) {
  const matches = runtimes.filter(
    (runtime) => runtime.targetOs === targetOs && runtime.targetArch === targetArch,
  );
  if (matches.length !== 1) {
    throw new Error(`Mira local AI index must contain one runtime for ${targetOs}-${targetArch}`);
  }
  const [runtime] = matches;
  if (
    runtime.runtimeApiVersion !== 2 ||
    !runtime.url.startsWith(
      `https://github.com/hello-yunshu/rill-ml/releases/download/v${runtimeVersion}/`,
    ) ||
    !/^[0-9a-f]{64}$/i.test(runtime.sha256) ||
    !Number.isSafeInteger(runtime.size) ||
    runtime.size <= 0
  ) {
    throw new Error('Mira local AI index contains an invalid Rill runtime artifact');
  }
}
const handler = handlers[0];
for (const artifact of [models[0], handler]) {
  if (
    !artifact.url.startsWith('https://github.com/hello-yunshu/mira-mouse/releases/download/') ||
    !/^[0-9a-f]{64}$/i.test(artifact.sha256) ||
    !Number.isSafeInteger(artifact.size) ||
    artifact.size <= 0
  ) {
    throw new Error('Mira model or handler artifact metadata is invalid');
  }
}
const minimumRuntimeVersion = handler.minRuntimeVersion;
parseStableVersion(minimumRuntimeVersion, 'handler minimum runtime version');
if (compareVersions(runtimeVersion, minimumRuntimeVersion) < 0) {
  throw new Error('Mira handler requires a newer runtime than the stable deployment provides');
}

const result = {
  runtimeVersion,
  runtimeTag: `v${runtimeVersion}`,
  modelVersion: models[0].version,
  handlerVersion: handler.version,
  handlerMinRuntimeVersion: minimumRuntimeVersion,
};
if (process.env.GITHUB_OUTPUT) {
  appendFileSync(
    process.env.GITHUB_OUTPUT,
    [
      `runtime-version=${result.runtimeVersion}`,
      `runtime-tag=${result.runtimeTag}`,
      `model-version=${result.modelVersion}`,
      `handler-version=${result.handlerVersion}`,
      `handler-min-runtime-version=${result.handlerMinRuntimeVersion}`,
      '',
    ].join('\n'),
  );
}
console.log(JSON.stringify(result));
