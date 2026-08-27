#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// 校验 Mira 主仓库与 battery handler 的 rill 依赖版本一致，并与 CI 解析的
// rill runtime release 对齐。
//
// 约束（与 docs/architecture/local-ai-rill-compatibility.md 一致）：
//   host  rill-runtime-protocol major.minor
//        = handler rill-ml major.minor
//        = rill runtime release major.minor（当提供 --runtime-version 时）
// Stable contract remains:
//   runtime IPC API = 2
//   handler ABI    = 1
//
// 用法：
//   node scripts/check-rill-compatibility.mjs
//   node scripts/check-rill-compatibility.mjs --expected-rill-version 1.5.3
//   node scripts/check-rill-compatibility.mjs --expected-rill-version 1.5.3 --runtime-version 1.5.3
// 任一不一致时退出码非 0。

import { readFile } from 'node:fs/promises';

const args = process.argv.slice(2);
let runtimeVersion;
let expectedRillVersion;
for (let index = 0; index < args.length; index += 1) {
  if (args[index] === '--runtime-version') runtimeVersion = args[++index];
  else if (args[index] === '--expected-rill-version') expectedRillVersion = args[++index];
  else throw new Error(`unknown argument: ${args[index]}`);
}

function majorMinor(version) {
  const match = /^(\d+)\.(\d+)\.(\d+)$/.exec(version);
  if (!match) throw new Error(`invalid semver: ${version}`);
  return `${match[1]}.${match[2]}`;
}

function stableVersion(version, label) {
  majorMinor(version);
  if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error(`invalid ${label}: ${version}`);
  return version;
}

function packageVersion(lockText, packageName) {
  const block = lockText.match(
    new RegExp(`name = "${packageName}"\\nversion = "([^"]+)"`),
  );
  if (!block) throw new Error(`${packageName} is missing from Cargo.lock`);
  return block[1];
}

async function readLock(path) {
  try {
    return await readFile(path, 'utf8');
  } catch (error) {
    throw new Error(`cannot read ${path}: ${error.message}`);
  }
}

const rootLock = await readLock('Cargo.lock');
const handlerLock = await readLock('handlers/mira-battery-handler/Cargo.lock');

const rootRillMl = packageVersion(rootLock, 'rill-ml');
const rootProtocol = packageVersion(rootLock, 'rill-runtime-protocol');
const handlerRillMl = packageVersion(handlerLock, 'rill-ml');

const modelManifest = JSON.parse(
  await readFile('local-ai/model-manifest.json', 'utf8'),
);
const handlerManifest = JSON.parse(
  await readFile('handlers/mira-battery-handler/manifest.template.json', 'utf8'),
);
if (expectedRillVersion) stableVersion(expectedRillVersion, 'expected Rill version');
if (runtimeVersion) stableVersion(runtimeVersion, 'runtime version');

const failures = [];

function requireEqual(label, actual, expected) {
  if (actual !== expected) {
    failures.push(`${label}: ${actual} != ${expected}`);
  }
}

requireEqual('host rill-ml minor', majorMinor(rootRillMl), majorMinor(rootProtocol));
requireEqual('handler rill-ml minor', majorMinor(handlerRillMl), majorMinor(rootProtocol));
if (runtimeVersion) {
  requireEqual(
    'rill runtime release minor (--runtime-version)',
    majorMinor(runtimeVersion),
    majorMinor(rootProtocol),
  );
}
if (expectedRillVersion) {
  requireEqual('host rill-ml exact version', rootRillMl, expectedRillVersion);
  requireEqual('host rill-runtime-protocol exact version', rootProtocol, expectedRillVersion);
  requireEqual('handler rill-ml exact version', handlerRillMl, expectedRillVersion);
  if (runtimeVersion) requireEqual('rill runtime exact version', runtimeVersion, expectedRillVersion);
}
if (modelManifest.runtimeApiVersion !== 2) {
  failures.push(
    `model manifest runtimeApiVersion: ${modelManifest.runtimeApiVersion} != 2`,
  );
}
if (handlerManifest.handlerApiVersion !== 1) {
  failures.push(`handler manifest handlerApiVersion: ${handlerManifest.handlerApiVersion} != 1`);
}

if (failures.length > 0) {
  console.error('rill compatibility check FAILED:');
  for (const failure of failures) console.error(`  - ${failure}`);
  process.exit(1);
}

const summary = {
  rillMl: rootRillMl,
  rillRuntimeProtocol: rootProtocol,
  handlerRillMl,
  expectedRillVersion: expectedRillVersion ?? 'not-pinned',
  runtimeRelease: runtimeVersion ?? 'resolved-at-CI-time',
  runtimeApiVersion: modelManifest.runtimeApiVersion,
  handlerApiVersion: handlerManifest.handlerApiVersion,
  compatible: true,
};
console.log(JSON.stringify(summary, null, 2));
