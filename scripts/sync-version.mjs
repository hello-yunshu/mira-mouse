#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// 将 [workspace.package].version（Cargo.toml 的单一真源）同步到仍需硬编码
// App 版本号或受其传导影响的位置：
//   1. CITATION.cff（GitHub 学术引用元数据，YAML）
//   2. ROADMAP.md（文档中的「当前版本」标注）
//   3. mira-local-ai 对 mira-protocol 的显式 path 依赖版本
//   4. handlers/mira-battery-handler/Cargo.lock（独立 workspace 的 path 依赖锁）
//   5. README.md / README.en.md（最新版三平台直链）
//
// Mira 本地 AI handler 与模型有独立发布周期，不能在 App 版本同步时改动
// handler 自身的版本号；但 handler 通过 path 依赖 workspace crate，升 workspace
// 版本后必须同步 handler 的 Cargo.lock，否则 CI 的 xtask handler check-lock 会
// 失败。本脚本在 handler 目录跑 `cargo update -p <path deps>` 自动同步。
//
// 用法：node scripts/sync-version.mjs
// 若所有文件已是最新则不做任何写入，退出码 0。

import { readFile, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { spawnSync } from 'node:child_process';

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

/**
 * 在 handlers/mira-battery-handler/ 跑 `cargo update -p <path deps>`，把 handler 独立
 * Cargo.lock 中的 path 依赖（mira-local-ai / mira-protocol）同步到当前 workspace 版本。
 * handler 被 workspace exclude，dependabot 升级 workspace 外部依赖后不会自动同步
 * handler 的 Cargo.lock，若漏跑会导致下游 model-pack 的 `cargo build --locked` 失败。
 * 注意：只更新两个 path 依赖，不执行无约束的 `cargo update`，避免依赖漂移破坏可复现性。
 * cargo 不可用时跳过（不阻塞纯文档同步场景）。
 */
async function syncHandlerLock(version) {
  const handlerDir = 'handlers/mira-battery-handler';
  if (!existsSync(`${handlerDir}/Cargo.toml`)) {
    console.log(`  handler lock: ${handlerDir}/Cargo.toml not found, skipped`);
    return false;
  }

  const cargoCheck = spawnSync('cargo', ['--version'], { encoding: 'utf8' });
  if (cargoCheck.status !== 0) {
    console.log(`  handler lock: cargo not on PATH, skipped`);
    return false;
  }

  const result = spawnSync(
    'cargo',
    ['update', '-p', 'mira-local-ai', '-p', 'mira-protocol'],
    { cwd: handlerDir, encoding: 'utf8' },
  );
  if (result.status !== 0) {
    throw new Error(
      `handler lock: cargo update failed in ${handlerDir}:\n${(result.stderr || '').trim()}`,
    );
  }
  const output = (result.stdout + result.stderr).trim();
  // cargo update 输出 "Updating <pkg> v<x> -> v<y>" 表示发生了版本变化。
  const changed = /Updating\s+\S+\s+v\d+\.\d+\.\d+\s+->\s+v\d/.test(output);
  if (changed) {
    console.log(`  handler lock: ${handlerDir}  →  ${version}`);
    if (output) {
      for (const line of output.split('\n')) {
        console.log(`    ${line}`);
      }
    }
  } else {
    console.log(`  handler lock: ${handlerDir}  (already ${version})`);
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

// 3. publish=false 的 mira-local-ai 仍被独立 handler workspace 作为 path
// 依赖使用。Cargo 对 prerelease 不做隐式 caret 匹配，因此这里必须与 workspace
// 版本精确同步。
changed |= await syncFile(
  'crates/mira-local-ai/Cargo.toml',
  /^(mira-protocol\s*=\s*\{\s*version\s*=\s*")\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?("\s*,\s*path\s*=\s*"\.\.\/mira-protocol"\s*\})/m,
  `$1${version}$2`,
  'mira-local-ai -> mira-protocol',
  version,
);

// 4. handler Cargo.lock —— 同步 path 依赖到当前 workspace 版本
changed |= await syncHandlerLock(version);

// 5. README 直链 —— `releases/latest` 保持标签动态，文件名中的版本号仍需
// 与构建产物一致。中英文各同步三种平台资产。
for (const path of ['README.md', 'README.en.md']) {
  for (const [label, prefix, suffix] of [
    ['macOS download', 'Mira_macOS_', '_aarch64.dmg'],
    ['Windows download', 'Mira_Windows_', '_x64-setup.exe'],
    ['Linux download', 'Mira_Linux_', '_amd64.AppImage'],
  ]) {
    changed |= await syncFile(
      path,
      new RegExp(`(releases/latest/download/${prefix.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')})\\d+\\.\\d+\\.\\d+(?:[-+][0-9A-Za-z.-]+)?(${suffix.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')})`),
      `$1${version}$2`,
      `${label} link`,
      version,
    );
  }
}

if (changed) {
  console.log('sync complete: some files updated');
} else {
  console.log('sync complete: all files already in sync');
}
