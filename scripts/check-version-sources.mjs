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

// 校验仍需硬编码版本号的文件与 [workspace.package].version 保持一致。
// 这些文件无法通过 workspace = true 派生版本，由 scripts/sync-version.mjs 同步。
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
assertSynced(
  'handlers/mira-battery-handler/Cargo.toml',
  handlerCargo,
  /^\[package\][\s\S]*?version\s*=\s*"([^"]+)"/m,
  appVersion,
);

const citation = await readFile('CITATION.cff', 'utf8');
assertSynced('CITATION.cff', citation, /^version:\s*(\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?)/m, appVersion);

const roadmap = await readFile('ROADMAP.md', 'utf8');
assertSynced('ROADMAP.md', roadmap, /\*\*版本\s*(\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?)\s*\*\*/, appVersion);

console.log(`app version source: Cargo.toml [workspace.package].version = ${appVersion}`);
