<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Plugin Package Format

`.mira-plugin` is a deterministic ZIP container of declarative JSON, documentation, fixtures, checksums, and one optional Ed25519 signature. Paths must be normalized relative UTF-8 paths; absolute paths, `..`, backslashes, duplicate entries, symbolic links, executable/script/web extensions, remote resources, and files outside the whitelist are rejected before extraction.

The whitelist permits top-level `plugin.json`, `checksums.json`, `devices.json`, `capabilities.json`, `README.md`, `LICENSE`, and `META-INF/signature.ed25519`, plus `.json` files under the `protocol/`, `locales/`, `tests/fixtures/`, and `models/` prefixes. The `models/` directory is the reserved parent folder for per-model adapter overrides: future plugins may ship model-specific JSON (for example `models/<model>/capabilities.json`) without changing the package format.

`checksums.json` schema 1 maps every payload path except itself and `META-INF/signature.ed25519` to lowercase SHA-256. Coverage must be exact. The signature message is canonical JSON for `plugin.json`, one LF byte, then canonical JSON for `checksums.json`. Canonical JSON recursively sorts object keys, preserves array order and JSON scalar values, and emits compact UTF-8 without insignificant whitespace. The key is selected only by the manifest key ID and the configured trust store.

Current limits are 512 files, 4 MiB per file, and 32 MiB total uncompressed bytes. Verification fails closed on limit, schema, digest, coverage, key, signature, ID, API, permission, or evidence errors.

