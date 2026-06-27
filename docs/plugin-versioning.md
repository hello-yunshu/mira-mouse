<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Plugin Versioning

Plugin packages use SemVer. Manifest, filename, lock entry, and release metadata must agree. Breaking manifest/DSL changes require a Plugin API major version.

Per-plugin versioned releases are immutable and must not silently replace an existing same-version asset. The unified `release/v*` bundle used for the current latest plugin set may be overwritten intentionally; when that happens, the release workflow must regenerate registry metadata and the Mira `plugins.lock.json` entries from the published `.sha256` files. Maintainers should use `xtask plugins update-lock` or the generated plugin-sync PR rather than editing SHA-256 values by hand.
