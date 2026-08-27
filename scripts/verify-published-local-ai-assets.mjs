#!/usr/bin/env node
// Verify the public app-release bytes before the local-ai-stable pointer is moved.
import { createHash } from 'node:crypto';
import { readFileSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';
import {
  MIRA_INDEX_PUBLIC_KEY_HEX,
  MIRA_INDEX_PUBLISHER_KEY_ID,
  verifySignedIndex,
} from './signed-release-index.mjs';

const [indexPathArg, assetsDirArg] = process.argv.slice(2);
if (!indexPathArg || !assetsDirArg) {
  throw new Error('usage: verify-published-local-ai-assets.mjs SIGNED_INDEX ASSETS_DIR');
}

const index = JSON.parse(readFileSync(resolve(indexPathArg), 'utf8'));
verifySignedIndex(
  index,
  MIRA_INDEX_PUBLISHER_KEY_ID,
  MIRA_INDEX_PUBLIC_KEY_HEX,
  'published Mira local AI index',
);
const artifacts = index.payload.artifacts;
if (index.payload.schemaVersion !== 3 || index.payload.channel !== 'stable' || !Array.isArray(artifacts)) {
  throw new Error('published Mira local AI index does not use stable schema v3');
}

for (const [kind, id, fileName] of [
  ['model', 'mira-battery-model', 'model.rillpack'],
  ['handler', 'mira.battery.handler', 'handler.rillhandler'],
]) {
  const matches = artifacts.filter((artifact) => artifact.kind === kind && artifact.id === id);
  if (matches.length !== 1) throw new Error(`published index must contain one ${kind} artifact`);
  const artifact = matches[0];
  const path = join(resolve(assetsDirArg), fileName);
  const size = statSync(path).size;
  const sha256 = createHash('sha256').update(readFileSync(path)).digest('hex');
  if (size !== artifact.size || sha256 !== artifact.sha256) {
    throw new Error(
      `${fileName} does not match the published signed index: ` +
        `actual ${size}/${sha256}, expected ${artifact.size}/${artifact.sha256}`,
    );
  }
  console.log(`${fileName}: ${size} bytes, ${sha256}`);
}
console.log('PASS: public app-release local AI assets match the signed index');
