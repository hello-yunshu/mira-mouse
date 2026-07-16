#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// 将 [workspace.package].version（Cargo.toml 的单一真源）同步到仍需硬编码
// App 版本号的两处文件：
//   1. CITATION.cff（GitHub 学术引用元数据，YAML）
//   2. ROADMAP.md（文档中的「当前版本」标注）
//
// Mira 本地 AI handler 与模型有独立发布周期，不能在 App 版本同步时改动。
//
// 用法：node scripts/sync-version.mjs
// 若所有文件已是最新则不做任何写入，退出码 0。

import { readFile, writeFile } from 'node:fs/promises';

const semver = /^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/;

async function readAppVersion() {
  const cargoToml = await readFile('Cargo.toml', 'utf8');
  const block = cargoToml.match(/\[workspace\.package\]([\s\S]*?)(?:\n\[|$)/);
  const version = block?.[1].match(/^\s*version\s*=\s*"([^"]+)"/m)?.[1];
  if (!version || !semver.test(version)) {
    throw new Error('Cargo.toml [workspace.package].version must be SemVer');
  }
  return version;
}

/** 替换文本中第一个匹配并返回 {text, changed}；无匹配则抛错。 */
function replaceOnce(text, pattern, replacement, label) {
  if (!pattern.test(text)) {
    throw new Error(`${label}: could not find version placeholder to replace`);
  }
  // 不带 g 标志的 replace 只替换第一个匹配。
  const next = text.replace(pattern, replacement);
  return { text: next, changed: next !== text };
}

async function syncFile(path, pattern, replacement, label, version) {
  const text = await readFile(path, 'utf8');
  const { text: next, changed } = replaceOnce(text, pattern, replacement, label);
  if (changed) {
    await writeFile(path, next);
    console.log(`  ${label}: ${path}  →  ${version}`);
  } else {
    console.log(`  ${label}: ${path}  (already ${version})`);
  }
  return changed;
}

const version = await readAppVersion();
console.log(`syncing app version ${version} …`);

let changed = false;

// 1. CITATION.cff —— YAML 顶层 version: x.y.z（无引号）
changed |= await syncFile(
  'CITATION.cff',
  /^(version:\s*)\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?/m,
  `$1${version}`,
  'CITATION.cff',
  version,
);

// 2. ROADMAP.md —— **版本 x.y.z**
changed |= await syncFile(
  'ROADMAP.md',
  /(\*\*版本\s*)\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?(\s*\*\*)/,
  `$1${version}$2`,
  'ROADMAP.md',
  version,
);

if (changed) {
  console.log('sync complete: some files updated');
} else {
  console.log('sync complete: all files already in sync');
}
