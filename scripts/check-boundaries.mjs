// SPDX-License-Identifier: AGPL-3.0-or-later
import { readdir, readFile } from 'node:fs/promises';
import { join, relative } from 'node:path';

const root = new URL('..', import.meta.url).pathname;
const ignored = new Set(['.git', 'node_modules', 'dist', 'target', 'AMasterDriver_v1.0.6_unpacked_reverse_bundle', 'src-tauri/resources/plugins']);
const allowed = new Set(['Mira_AI_IDE_Implementation_Prompt.md', 'docs/execution-plan.md', 'docs/spec-traceability.md', 'docs/evidence-status.md', 'README.md', 'scripts/check-boundaries.mjs']);
const forbidden = [/0x3151/i, /0x0e8d/i, /0x402a/i, /0x5007/i, /AM Mouse Tool/i, /Velora/i, /mira\.amaster\s*\)/i];
const violations = [];
async function walk(dir) {
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    if (ignored.has(entry.name)) continue;
    const path = join(dir, entry.name); const rel = relative(root, path);
    if (entry.isDirectory()) await walk(path);
    else if (!allowed.has(rel) && !/\.(png|ico|icns|zip|lock)$/.test(rel)) {
      const text = await readFile(path, 'utf8').catch(() => '');
      for (const pattern of forbidden) if (pattern.test(text)) violations.push(`${rel}: ${pattern}`);
    }
  }
}
await walk(root);
if (violations.length) { console.error(violations.join('\n')); process.exit(1); }
console.log('brand boundary: clean');
