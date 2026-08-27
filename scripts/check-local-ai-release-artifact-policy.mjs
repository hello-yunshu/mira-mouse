#!/usr/bin/env node
// Regression guard for mutable app-release assets referenced by the signed index.
import assert from 'node:assert/strict';
import { selectPublishedArtifact } from './local-ai-release-artifact-policy.mjs';

const candidate = { version: '0.8.7', url: 'https://example.test/app/v1.2.7/handler.rillhandler', sha256: 'new', size: 2 };
const sameUrlPrevious = { ...candidate, sha256: 'old', size: 1 };
const olderPrevious = { ...sameUrlPrevious, version: '0.8.6' };
const newerPrevious = { ...sameUrlPrevious, version: '0.8.8' };
const differentUrlPrevious = { ...sameUrlPrevious, url: 'https://example.test/local-ai-handler-v0.8.7/handler.rillhandler' };

assert.equal(selectPublishedArtifact(candidate, sameUrlPrevious), candidate);
assert.equal(selectPublishedArtifact(candidate, olderPrevious), candidate);
assert.equal(selectPublishedArtifact(candidate, newerPrevious), newerPrevious);
assert.equal(selectPublishedArtifact(candidate, differentUrlPrevious), differentUrlPrevious);
console.log('PASS: published-artifact policy keeps mutable same-URL digests aligned');
