#!/usr/bin/env node
// Regression guard for the Rill Stable promotion boundary.
import { readFile } from 'node:fs/promises';

const resolver = await readFile('scripts/resolve-latest-rill-release.mjs', 'utf8');
const stablePointer = '/releases/download/local-ai-stable/stable-index.json';
const immutablePattern = /`\$\{RELEASE_BASE\}\/download\/v\$\{requestedVersion\}\/stable-index\.json`/;

if (!resolver.includes(stablePointer) && !resolver.includes("`${RELEASE_BASE}/download/local-ai-stable/stable-index.json`")) {
  throw new Error('default Rill resolver must use the promoted local-ai-stable pointer');
}
if (resolver.includes('/releases/latest/download/stable-index.json')) {
  throw new Error('default Rill resolver must not use the GitHub latest Release');
}
if (!immutablePattern.test(resolver)) {
  throw new Error('explicit --version must continue using an immutable v<version> index');
}
for (const required of [
  'payload.schemaVersion !== 3',
  "payload.channel !== 'stable'",
  'artifact.runtimeApiVersion !== 2',
  'verifySignedIndex',
  'matchesTarget',
]) {
  if (!resolver.includes(required)) throw new Error(`resolver lost fail-closed guard: ${required}`);
}
console.log('PASS: Rill resolver uses local-ai-stable by default and immutable v<version> for exact mode');
