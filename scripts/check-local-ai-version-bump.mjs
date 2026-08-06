#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// 版本门禁：防止「内容已经变化但版本号未递增」再次发生。
//
// 对比当前分支（HEAD）与其合并基线（默认 origin/main，PR 场景由 CI 传入
// GITHUB_BASE_REF），逐一检查三个独立发布体的版本是否在内容发生功能性变化时
// 严格高于基线：
//
//   1. App   （workspace version，Cargo.toml [workspace.package].version）
//   2. Model  （local-ai/model-manifest.json version，本地 AI 模型包）
//   3. Handler（handlers/mira-battery-handler 的 Cargo.toml / manifest 版本）
//
// 触发规则（任一命中即要求对应版本严格大于基线，否则失败）：
//   - Model：local-ai/**、模型 FeatureSchema、模型 schema 哈希 fixture、
//     模型构建脚本 / model-pack workflow。
//   - Handler：handler 源码、WIT、依赖（crates/mira-local-ai、
//     crates/mira-protocol path 依赖）、manifest 及 handler 构建脚本。
//   - App：src/**、src-tauri/**、crates/** 等应用功能性代码，
//     Cargo 锁 / package.json 等版本传导源。
//
// 本脚本只校验，绝不自动修改任何版本号或文件。
//
// 用法：
//   node scripts/check-local-ai-version-bump.mjs
//   node scripts/check-local-ai-version-bump.mjs --base <ref>
// 基线解析顺序：--base 参数 > $GITHUB_BASE_REF > origin/main；本地 ref 缺失时
// 会尝试 `git fetch origin <ref>`。检查不通过时退出码非 0 并打印明确错误。

import { execFileSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';

const semver = /^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/;

function parseVersion(text) {
  if (!text || !semver.test(text)) {
    throw new Error(`invalid semver: ${JSON.stringify(text)}`);
  }
  return text
    .split(/[-+]/)[0]
    .split('.')
    .map((part) => Number.parseInt(part, 10));
}

function compare(a, b) {
  for (let index = 0; index < 3; index += 1) {
    if (a[index] !== b[index]) return a[index] - b[index];
  }
  return 0;
}

function git(args) {
  return execFileSync('git', args, { encoding: 'utf8' }).trim();
}

function resolveCommit(ref) {
  try {
    return git(['rev-parse', `${ref}^{commit}`]);
  } catch {
    return null;
  }
}

function workspaceVersion(text) {
  const match = text.match(/\[workspace\.package\]([\s\S]*?)(?:\n\[|$)/);
  return match?.[1].match(/^\s*version\s*=\s*"([^"]+)"/m)?.[1] ?? null;
}

function jsonVersion(text) {
  return JSON.parse(text).version ?? null;
}

function handlerCargoVersion(text) {
  const match = text.match(/^\[package\][\s\S]*?version\s*=\s*"([^"]+)"/m);
  return match?.[1] ?? null;
}

/** 读取 ref 上某文件的版本；文件或字段缺失时返回 null。 */
function versionAt(ref, path, extract) {
  try {
    const text = git(['show', `${ref}:${path}`]);
    const version = extract(text);
    return version && semver.test(version) ? version : null;
  } catch {
    return null;
  }
}

function resolveBase() {
  const args = process.argv.slice(2);
  for (let index = 0; index < args.length; index += 1) {
    if (args[index] === '--base') return args[++index];
    if (args[index] === '--help' || args[index] === '-h') {
      console.log('usage: node scripts/check-local-ai-version-bump.mjs [--base <ref>]');
      process.exit(0);
    }
    throw new Error(`unknown argument: ${args[index]}`);
  }
  if (process.env.GITHUB_BASE_REF) return process.env.GITHUB_BASE_REF;
  return 'origin/main';
}

/** glob 转正则：** 匹配任意层级，* 匹配单层片段。 */
function globToRegExp(glob) {
  const pattern = glob
    .split('/')
    .map((segment) => {
      if (segment === '**') return '(?:[^/]+/)*';
      return segment.replaceAll('.', '\\.').replaceAll('*', '[^/]*');
    })
    .join('/');
  return new RegExp(`^${pattern}$`);
}

const matchers = new Map([
  [
    'model',
    [
      'local-ai/**',
      'crates/mira-local-ai/src/battery_features.rs',
      'crates/mira-local-ai/tests/fixtures/battery_schema_v1.json',
      'scripts/build-model-manifest.mjs',
      '.github/workflows/model-pack.yml',
    ].map(globToRegExp),
  ],
  [
    'handler',
    [
      'handlers/mira-battery-handler/**',
      'crates/mira-local-ai/**',
      'crates/mira-protocol/**',
      'scripts/build-handler-manifest.mjs',
      'scripts/build-handler-release-index.mjs',
      '.github/workflows/local-ai-handler-release.yml',
    ].map(globToRegExp),
  ],
  [
    'app',
    [
      'src/**',
      'src-tauri/**',
      'crates/**',
      'Cargo.toml',
      'Cargo.lock',
      'package.json',
      'package-lock.json',
      'index.html',
      'vite.config.*',
      'tsconfig*.json',
    ].map(globToRegExp),
  ],
]);

const baseRef = resolveBase();

/** 先试原样 ref，再试 origin/ 前缀（GITHUB_BASE_REF 形如 "main"）。 */
function resolveBaseCommit() {
  for (const candidate of [baseRef, `origin/${baseRef}`]) {
    const commit = resolveCommit(candidate);
    if (commit) return commit;
  }
  return null;
}

let baseCommit = resolveBaseCommit();
if (!baseCommit) {
  try {
    git(['fetch', 'origin', baseRef]);
    baseCommit = resolveBaseCommit();
  } catch {
    baseCommit = null;
  }
}
if (!baseCommit) {
  console.error(
    `version-bump check: cannot resolve base ref "${baseRef}". ` +
      'Provide an existing local ref (git fetch origin) or pass --base <ref>.',
  );
  process.exit(2);
}

const mergeBase = git(['merge-base', baseCommit, 'HEAD']);
const changed = git(['diff', '--name-only', '--diff-filter=ACMR', mergeBase, 'HEAD'])
  .split('\n')
  .filter(Boolean);

const rootCargo = await readFile('Cargo.toml', 'utf8');
const modelManifest = JSON.parse(await readFile('local-ai/model-manifest.json', 'utf8'));
const handlerCargo = await readFile('handlers/mira-battery-handler/Cargo.toml', 'utf8');
const handlerManifest = JSON.parse(
  await readFile('handlers/mira-battery-handler/manifest.template.json', 'utf8'),
);

const artifacts = [
  {
    name: 'model',
    current: modelManifest.version,
    base: versionAt(baseCommit, 'local-ai/model-manifest.json', jsonVersion),
    contentChanged: changed.some((path) => matchers.get('model').some((re) => re.test(path))),
    note: 'local-ai/model-manifest.json',
  },
  {
    name: 'handler',
    current: handlerCargoVersion(handlerCargo),
    base: versionAt(baseCommit, 'handlers/mira-battery-handler/Cargo.toml', handlerCargoVersion),
    contentChanged: changed.some((path) =>
      matchers.get('handler').some((re) => re.test(path)),
    ),
    note: 'handlers/mira-battery-handler（Cargo.toml 与 manifest.template.json）',
  },
  {
    name: 'app',
    current: workspaceVersion(rootCargo),
    base: versionAt(baseCommit, 'Cargo.toml', workspaceVersion),
    contentChanged: changed.some((path) => matchers.get('app').some((re) => re.test(path))),
    note: 'Cargo.toml [workspace.package].version',
  },
];

const failures = [];

for (const artifact of artifacts) {
  const baseText = artifact.base ?? '0.0.0（基线中不存在）';
  const current = parseVersion(artifact.current);
  const greater = artifact.base !== null && compare(current, parseVersion(artifact.base)) > 0;
  const passes = !artifact.contentChanged || greater;
  const reason = artifact.contentChanged
    ? greater
      ? `strictly newer than base (${artifact.base})`
      : `content changed but version ${artifact.current} is NOT higher than base (${baseText})`
    : 'no relevant content change';
  console.log(
    `  [${passes ? 'pass' : 'FAIL'}] ${artifact.name}: current=${artifact.current} base=${baseText} contentChanged=${artifact.contentChanged} (${reason})`,
  );
  if (!passes) {
    failures.push(
      `「${artifact.name}」内容已变化但版本未递增：当前 ${artifact.current}，基线 ${baseText}（版本来源：${artifact.note}）。请本地递增版本、同步并提交，本检查不会自动修改。`,
    );
  }
}

if (handlerCargoVersion(handlerCargo) !== handlerManifest.version) {
  failures.push(
    `handler Cargo.toml 版本 ${handlerCargoVersion(handlerCargo)} 与 manifest.template.json 版本 ${handlerManifest.version} 不一致，请先同步再提交。`,
  );
  console.error(
    `  [FAIL] handler Cargo/manifest 不一致：${handlerCargoVersion(handlerCargo)} != ${handlerManifest.version}`,
  );
}

if (failures.length > 0) {
  console.error('\nversion bump check FAILED:\n  ' + failures.join('\n  '));
  process.exit(1);
}
console.log('version bump check passed: content unchanged or version strictly increased over base.');
