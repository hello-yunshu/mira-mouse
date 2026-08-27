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

function compareVersions(left, right) {
  for (let index = 0; index < 3; index += 1) {
    if (left[index] !== right[index]) return left[index] - right[index];
  }
  return 0;
}
