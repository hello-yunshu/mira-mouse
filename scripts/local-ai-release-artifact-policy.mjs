import { createHash } from 'node:crypto';
import { writeFileSync } from 'node:fs';
import { parseStableVersion } from './signed-release-index.mjs';

export function selectPublishedArtifact(candidate, previous) {
  if (!previous) return candidate;

  const previousVersion = parseStableVersion(previous.version, 'published artifact version');
  const candidateVersion = parseStableVersion(candidate.version, 'candidate artifact version');
  const comparison = compareVersions(previousVersion, candidateVersion);

  // A newer published artifact remains authoritative. Equal-version artifacts at the
  // same URL are replaceable because app releases are intentionally overwritten by
  // the release workflow; keep the candidate digest in sync with those bytes.
  if (comparison > 0) return previous;
  if (comparison === 0 && previous.url !== candidate.url) return previous;
  return candidate;
}

export async function materializePublishedArtifact(candidatePath, candidate, selected, fetchImpl = fetch) {
  if (
    candidate.url === selected.url &&
    candidate.sha256 === selected.sha256 &&
    candidate.size === selected.size
  ) {
    return false;
  }

  const response = await fetchImpl(selected.url);
  if (!response.ok) {
    throw new Error(`fetch published local AI artifact: ${response.status} ${response.statusText}`);
  }
  const bytes = Buffer.from(await response.arrayBuffer());
  const sha256 = createHash('sha256').update(bytes).digest('hex');
  if (bytes.length !== selected.size || sha256 !== selected.sha256) {
    throw new Error(
      `published local AI artifact does not match its index: ` +
        `actual ${bytes.length}/${sha256}, expected ${selected.size}/${selected.sha256}`,
    );
  }
  // The signed index may intentionally retain a newer independently published
  // artifact. Publish the exact same bytes under the app release asset name so
  // the app-release verifier and downstream consumers see one coherent pair.
  writeFileSync(candidatePath, bytes);
  return true;
}

function compareVersions(left, right) {
  for (let index = 0; index < 3; index += 1) {
    if (left[index] !== right[index]) return left[index] - right[index];
  }
  return 0;
}
