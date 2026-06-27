<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Plugin Publishing

Stable publication requires deterministic packing, complete Fixtures, compatible Plugin API, exact permissions, protected-environment signing, SHA-256, SBOM, evidence summary, draft-release redownload verification, and cross-platform import checks. Without a production key only an `unsigned-preview` may be produced.

## Lock synchronization

Do not hand-edit plugin SHA-256 values in `plugins.lock.json`. The plugin
repository release is the source of truth for `.mira-plugin` assets and matching
`.sha256` files. After a plugin release is created or overwritten, the plugin
release workflow dispatches registry publication and Mira lock synchronization.
When `MIRA_APP_TOKEN` is configured, that synchronization validates the lock
update and commits it directly to the Mira app repository `main` branch.

Maintainers can refresh the lock locally with:

```bash
cargo run --package xtask -- plugins update-lock --release-tag release/vYYYY-MM-DD
```

CI checks the lock before Tauri bundle jobs with:

```bash
cargo run --package xtask -- plugins check-lock
```

If this check fails, rerun the plugin lock sync workflow or use `update-lock`
locally instead of copying a SHA by hand.
