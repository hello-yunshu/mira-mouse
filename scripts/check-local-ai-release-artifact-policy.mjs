#!/usr/bin/env node
// Regression guard for mutable app-release assets referenced by the signed index.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import {
  materializePublishedArtifact,
  selectPublishedArtifact,
} from './local-ai-release-artifact-policy.mjs';

const candidate = { version: '0.8.7', url: 'https://example.test/app/v1.2.7/handler.rillhandler', sha256: 'new', size: 2 };
const sameUrlPrevious = { ...candidate, sha256: 'old', size: 1 };
const olderPrevious = { ...sameUrlPrevious, version: '0.8.6' };
const newerPrevious = { ...sameUrlPrevious, version: '0.8.8' };
const differentUrlPrevious = { ...sameUrlPrevious, url: 'https://example.test/local-ai-handler-v0.8.7/handler.rillhandler' };

assert.equal(selectPublishedArtifact(candidate, sameUrlPrevious), candidate);
assert.equal(selectPublishedArtifact(candidate, olderPrevious), candidate);
assert.equal(selectPublishedArtifact(candidate, newerPrevious), newerPrevious);
assert.equal(selectPublishedArtifact(candidate, differentUrlPrevious), differentUrlPrevious);

const newerBytes = Buffer.from('published-newer-handler');
const newerPublished = {
  ...newerPrevious,
  version: '0.8.8',
  sha256: createHash('sha256').update(newerBytes).digest('hex'),
  size: newerBytes.length,
  url: 'data:application/octet-stream;base64,' + newerBytes.toString('base64'),
};
const tempDir = mkdtempSync(join(tmpdir(), 'mira-local-ai-policy-'));
try {
  const candidatePath = join(tempDir, 'handler.rillhandler');
  writeFileSync(candidatePath, 'older-candidate');
  assert.equal(await materializePublishedArtifact(candidatePath, candidate, newerPublished), true);
  assert.deepEqual(readFileSync(candidatePath), newerBytes);
} finally {
  rmSync(tempDir, { recursive: true, force: true });
}

console.log('PASS: published-artifact policy keeps mutable digests aligned and materializes newer assets');
