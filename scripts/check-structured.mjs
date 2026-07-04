// SPDX-License-Identifier: AGPL-3.0-or-later
import { readFile, readdir, stat } from 'node:fs/promises';
import { join } from 'node:path';
import YAML from 'yaml';
const jsonFiles = ['package.json', 'tsconfig.json', 'tsconfig.app.json', 'tsconfig.node.json', 'plugins.lock.json', 'schemas/plugin-manifest-v1.schema.json', 'schemas/plugins-lock-v1.schema.json', 'src-tauri/tauri.conf.json', 'src-tauri/capabilities/default.json'];
for (const file of jsonFiles) JSON.parse(await readFile(file, 'utf8'));
const required = ['README.md', 'LICENSE', 'LICENSES/AGPL-3.0-or-later.txt', 'LICENSES/CC-BY-SA-4.0.txt', 'NOTICE', 'CITATION.cff'];
for (const file of required) if ((await stat(file)).size === 0) throw new Error(`${file} is empty`);
const lock = JSON.parse(await readFile('plugins.lock.json', 'utf8'));
if (lock.releaseReady && JSON.stringify(lock).includes('BLOCKED_')) throw new Error('release-ready lock contains unresolved metadata');
const yamlFiles = [];
async function collectYaml(dir) {
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) await collectYaml(path);
    else if (/\.ya?ml$/.test(entry.name)) yamlFiles.push(path);
  }
}
await collectYaml('.github');
for (const file of [...yamlFiles, 'CITATION.cff']) YAML.parse(await readFile(file, 'utf8'));
for (const file of yamlFiles.filter((file) => file.includes('/workflows/'))) {
  const text = await readFile(file, 'utf8');
  for (const match of text.matchAll(/uses:\s*([^\s#]+)/g)) {
    if (/@[0-9a-f]{40}$/.test(match[1])) throw new Error(`${file} has SHA-pinned action ${match[1]}`);
    if (!/@[^@\s]+$/.test(match[1])) throw new Error(`${file} has action without an explicit ref ${match[1]}`);
  }
}
console.log(`structured files: parseable, non-empty, ${yamlFiles.length} YAML files checked`);
