// SPDX-License-Identifier: AGPL-3.0-or-later
import { createPublicKey, verify } from 'node:crypto';

export const RILL_INDEX_PUBLISHER_KEY_ID = 'rillml-examples-2026-001';
export const RILL_INDEX_PUBLIC_KEY_HEX =
  '29fd1fc2f22bd7e405aec167ff0a0d8de791f011c415075d4c5f9f64fd93fc2e';
export const MIRA_INDEX_PUBLISHER_KEY_ID = 'mira-rill-2026-002';
export const MIRA_INDEX_PUBLIC_KEY_HEX =
  'ae4633988fd9e02a824bb9072f1dcf470a0b1d74bbc7905aaea00a62139e1479';

export function canonicalize(value) {
  if (Array.isArray(value)) return value.map(canonicalize);
  if (value && typeof value === 'object') {
    return Object.fromEntries(
      Object.entries(value)
        .sort(([left], [right]) => (left < right ? -1 : left > right ? 1 : 0))
        .map(([key, item]) => [key, canonicalize(item)]),
    );
  }
  return value;
}

export function verifySignedIndex(index, publisherKeyId, publicKeyHex, label) {
  if (index?.payload?.publisherKeyId !== publisherKeyId) {
    throw new Error(`${label} uses an untrusted publisher`);
  }
  if (typeof index.signature !== 'string' || !/^[0-9a-f]{128}$/i.test(index.signature)) {
    throw new Error(`${label} has an invalid signature encoding`);
  }
  if (!/^[0-9a-f]{64}$/i.test(publicKeyHex)) {
    throw new Error(`${label} verifier has an invalid public key`);
  }
  const publicKey = createPublicKey({
    key: Buffer.concat([
      Buffer.from('302a300506032b6570032100', 'hex'),
      Buffer.from(publicKeyHex, 'hex'),
    ]),
    format: 'der',
    type: 'spki',
  });
  const payload = Buffer.from(JSON.stringify(canonicalize(index.payload)));
  if (!verify(null, payload, publicKey, Buffer.from(index.signature, 'hex'))) {
    throw new Error(`${label} signature verification failed`);
  }
}

export function parseStableVersion(value, label) {
  const match = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.exec(value);
  if (!match) throw new Error(`${label} must be a stable semantic version`);
  return match.slice(1).map(Number);
}

export function compareVersions(left, right) {
  const leftParts = Array.isArray(left) ? left : parseStableVersion(left, 'left version');
  const rightParts = Array.isArray(right) ? right : parseStableVersion(right, 'right version');
  for (let index = 0; index < 3; index += 1) {
    if (leftParts[index] !== rightParts[index]) return leftParts[index] - rightParts[index];
  }
  return 0;
}

// Mira 支持的 Rill runtime 目标平台集合。
// release-index schema v3 在 Linux 上同时发布 gnu / musl 变体（musl 的 id 为
// ``rill-runtime-musl``），Mira 只消费 gnu 变体，因此对 Linux 显式声明 targetLibc。
export const MIRA_RUNTIME_TARGETS = Object.freeze([
  { targetOs: 'macos', targetArch: 'aarch64' },
  { targetOs: 'linux', targetArch: 'x86_64', targetLibc: 'gnu' },
  { targetOs: 'windows', targetArch: 'x86_64' },
]);

// 判断 runtime artifact 是否命中目标平台。带 targetLibc 的目标要求 artifact
// 显式匹配该 libc；不带 targetLibc 的目标要求 artifact 不得携带 libc（macOS /
// Windows / FreeBSD 等非 Linux 产物）。
export function matchesTarget(artifact, target) {
  if (artifact.targetOs !== target.targetOs || artifact.targetArch !== target.targetArch) {
    return false;
  }
  if (target.targetLibc !== undefined) return artifact.targetLibc === target.targetLibc;
  return artifact.targetLibc === undefined;
}
