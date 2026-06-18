<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Threat Model

## Assets

HID device integrity, plugin/update trust roots, user settings, diagnostic privacy, release credentials, and the application process are protected assets.

## Untrusted inputs

Plugin ZIPs, manifests, protocol workflows, Fixtures, HID responses, unknown-device strings, imported profiles, update metadata, release downloads, filenames, logs, and user-provided issue content are untrusted.

## Primary controls

- Validate package structure before extraction; reject traversal, links, duplicates, bombs, oversized entries, code, remote content, bad coverage, digest, signature, key, API, permission, and evidence.
- Keep HID handles, timing, cancellation, mutual exclusion, and readback in the core; plugins are declarative and bounded.
- Deny writes unless exact hardware evidence and operation policy allow them; preserve unknown fields and display actual state on failure.
- Redact stable identifiers, never upload implicitly, and keep telemetry/accounts/ads/resident network services absent.
- Use atomic state/plugin replacement, immutable locks, protected environments, SHA-pinned Actions, minimal permissions, and clean-job redownload verification.

Residual hardware, platform signing, updater signing, and public repository risks remain listed as `blocked`.

