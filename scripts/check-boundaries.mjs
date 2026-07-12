// SPDX-License-Identifier: AGPL-3.0-or-later
import { readdir, readFile } from 'node:fs/promises';
import { join, relative } from 'node:path';

const root = new URL('..', import.meta.url).pathname;
const ignored = new Set(['.git', '.trae', 'node_modules', 'dist', 'target', 'AMasterDriver_v1.0.6_unpacked_reverse_bundle', 'src-tauri/resources/plugins']);
const allowed = new Set(['README.md', 'scripts/check-boundaries.mjs']);

// 品牌硬编码：VID/PID、产品型号、品牌名
const forbidden = [/0x3151/i, /0x0e8d/i, /0x402a/i, /0x5007/i, /AM Mouse Tool/i, /Velora/i, /mira\.amaster\s*\)/i];

// UI 与插件协议耦合检查：前端代码不得直接引用 workflow/command/parser/transport。
// 只检查 src/ 下的 .ts/.tsx 文件（排除 .test.ts/.test.tsx 和类型定义）。
const protocolCouplingPatterns = [
  { pattern: /invoke\s*\(\s*['"]read_projection['"]/, label: 'UI directly invoking read_projection' },
  { pattern: /invoke\s*\(\s*['"]read_device_with_package['"]/, label: 'UI directly invoking runtime read' },
  { pattern: /protocol\/workflows\.json/, label: 'UI reading plugin protocol file' },
];
const protocolFilePattern = /^src\/(?!.*\.test\.tsx?$).*\.[tc]sx?$/;

const violations = [];
async function walk(dir) {
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    if (ignored.has(entry.name)) continue;
    const path = join(dir, entry.name);
    const rel = relative(root, path);
    if (entry.isDirectory()) {
      await walk(path);
    } else if (!allowed.has(rel) && !/\.(png|ico|icns|zip|lock)$/.test(rel)) {
      const text = await readFile(path, 'utf8').catch(() => '');
      for (const pattern of forbidden) {
        if (pattern.test(text)) violations.push(`${rel}: brand boundary ${pattern}`);
      }
      // UI 协议耦合检查
      if (protocolFilePattern.test(rel)) {
        for (const { pattern, label } of protocolCouplingPatterns) {
          if (pattern.test(text)) violations.push(`${rel}: ${label}`);
        }
      }
    }
  }
}
await walk(root);
if (violations.length) { console.error(violations.join('\n')); process.exit(1); }
console.log('brand boundary: clean');
