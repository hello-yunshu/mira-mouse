#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-or-later
import { readFile } from 'node:fs/promises';

function assertNoOwnVersion(name, object) {
  if (Object.hasOwn(object, 'version')) {
    throw new Error(`${name} must not define the app version; edit Cargo.toml [workspace.package].version only`);
  }
}

function workspacePackageBlock(cargoToml) {
  const match = cargoToml.match(/\[workspace\.package\]([\s\S]*?)(?:\n\[|$)/);
  if (!match) throw new Error('Cargo.toml is missing [workspace.package]');
  return match[1];
}

function workspaceMembers(cargoToml) {
  const match = cargoToml.match(/members\s*=\s*\[([\s\S]*?)\]/);
  if (!match) throw new Error('Cargo.toml is missing workspace members');
  return [...match[1].matchAll(/"([^"]+)"/g)].map((member) => member[1]);
}

const semver = /^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/;
const cargoToml = await readFile('Cargo.toml', 'utf8');
const appVersion = workspacePackageBlock(cargoToml).match(/^\s*version\s*=\s*"([^"]+)"/m)?.[1];
if (!appVersion || !semver.test(appVersion)) {
  throw new Error('Cargo.toml [workspace.package].version must be SemVer');
}

assertNoOwnVersion('package.json', JSON.parse(await readFile('package.json', 'utf8')));

const lock = JSON.parse(await readFile('package-lock.json', 'utf8'));
assertNoOwnVersion('package-lock.json packages[""]', lock.packages?.[''] ?? {});

const tauriConfig = JSON.parse(await readFile('src-tauri/tauri.conf.json', 'utf8'));
assertNoOwnVersion('src-tauri/tauri.conf.json', tauriConfig);

for (const member of workspaceMembers(cargoToml)) {
  const manifest = await readFile(`${member}/Cargo.toml`, 'utf8');
  if (!/^\s*version\.workspace\s*=\s*true\s*$/m.test(manifest)) {
    throw new Error(`${member}/Cargo.toml must use version.workspace = true`);
  }
}

const appVersionScript = await readFile('scripts/app-version.mjs', 'utf8');
if (!appVersionScript.includes("readFile('Cargo.toml'") && !appVersionScript.includes('readFile("Cargo.toml"')) {
  throw new Error('scripts/app-version.mjs must read the app version from Cargo.toml');
}

const workflow = await readFile('.github/workflows/pipeline.yml', 'utf8');
if (!workflow.includes('current_version=$(node scripts/app-version.mjs)')) {
  throw new Error('.github/workflows/pipeline.yml must derive release version via scripts/app-version.mjs');
}
if (/package\.json['"]?\)\.version|package\.json['"]?\]\.version/.test(workflow)) {
  throw new Error('.github/workflows/pipeline.yml must not read the app version from package.json');
}

const modelPackWorkflow = await readFile('.github/workflows/model-pack.yml', 'utf8');
const previewWorkflow = await readFile('.github/workflows/preview.yml', 'utf8');
const handlerReleaseWorkflow = await readFile(
  '.github/workflows/local-ai-handler-release.yml',
  'utf8',
);
const runtimeSyncWorkflow = await readFile('.github/workflows/sync-rill-runtime.yml', 'utf8');
const rillWorkflows = [
  workflow,
  previewWorkflow,
  modelPackWorkflow,
  handlerReleaseWorkflow,
  runtimeSyncWorkflow,
].join('\n');
if (/repository:\s*hello-yunshu\/rill-ml[\s\S]{0,120}?ref:\s*v\d+\.\d+\.\d+/.test(rillWorkflows)) {
  throw new Error('Rill workflow checkouts must use a verified resolver output, not a fixed tag');
}
if (!workflow.includes('resolve-latest-rill-release.mjs')) {
  throw new Error('.github/workflows/pipeline.yml must resolve the latest signed Rill release');
}
const xtask = await readFile('xtask/src/main.rs', 'utf8');
if (!xtask.includes('/releases/latest/download/stable-index.json')) {
  throw new Error('xtask dist-sidecar must default to the latest signed Rill stable index');
}
for (const forbidden of [
  'macos-15-intel',
  'universal-apple-darwin',
  'x86_64-apple-darwin',
  'darwin-x86_64',
  '_universal.dmg',
]) {
  if (rillWorkflows.includes(forbidden)) {
    throw new Error(`Mira macOS releases are ARM64-only; remove ${forbidden}`);
  }
}
if (/\bmapfile\b/.test(runtimeSyncWorkflow) || /\bmapfile\b/.test(handlerReleaseWorkflow)) {
  throw new Error('Local AI workflows must remain compatible with the macOS system Bash');
}
for (const script of [
  'scripts/resolve-latest-rill-release.mjs',
  'scripts/build-local-ai-release-index.mjs',
  'scripts/build-rill-runtime-release-index.mjs',
  'scripts/inspect-mira-local-ai-index.mjs',
]) {
  const source = await readFile(script, 'utf8');
  if (source.includes("['macos', 'x86_64']")) {
    throw new Error(`${script} must not publish or require an Intel macOS Runtime`);
  }
}
if (xtask.includes('"x86_64-apple-darwin" =>')) {
  throw new Error('xtask must reject Intel macOS sidecar downloads');
}

// App 元数据仍由 scripts/sync-version.mjs 同步；本地 AI 模型和 handler
// 使用独立版本，只校验各自来源内部一致性。
function assertSynced(label, text, pattern, expected) {
  const match = text.match(pattern);
  if (!match) {
    throw new Error(`${label}: could not find version field; run \`npm run sync:version\``);
  }
  if (match[1] !== expected) {
    throw new Error(
      `${label} version is ${match[1]} but expected ${expected}; run \`npm run sync:version\``,
    );
  }
}

const handlerCargo = await readFile('handlers/mira-battery-handler/Cargo.toml', 'utf8');
const handlerVersion = handlerCargo.match(/^\[package\][\s\S]*?version\s*=\s*"([^"]+)"/m)?.[1];
const handlerManifest = JSON.parse(
  await readFile('handlers/mira-battery-handler/manifest.template.json', 'utf8'),
);
if (!handlerVersion || !semver.test(handlerVersion)) {
  throw new Error('handlers/mira-battery-handler/Cargo.toml must define a SemVer version');
}
if (handlerManifest.version !== handlerVersion) {
  throw new Error(
    `handler manifest version is ${handlerManifest.version} but Cargo.toml is ${handlerVersion}; bump both for a handler release`,
  );
}

const modelManifest = JSON.parse(await readFile('local-ai/model-manifest.json', 'utf8'));
if (!semver.test(modelManifest.version)) {
  throw new Error('local-ai/model-manifest.json must define a SemVer version');
}

const citation = await readFile('CITATION.cff', 'utf8');
assertSynced('CITATION.cff', citation, /^version:\s*(\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?)/m, appVersion);

const roadmap = await readFile('ROADMAP.md', 'utf8');
assertSynced('ROADMAP.md', roadmap, /\*\*版本\s*(\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?)\s*\*\*/, appVersion);

console.log(`app version source: Cargo.toml [workspace.package].version = ${appVersion}`);
