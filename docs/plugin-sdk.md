<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Plugin SDK

Plugins declare device matching, topology, capabilities, fields, protocol workflows, localized labels, narrow Linux permission metadata, and offline Fixtures. They cannot provide UI layout, CSS, HTML, JavaScript, native code, scripts, network access, or filesystem access.

```bash
cargo xtask plugin new mira.example ./example
cargo xtask plugin validate ./example
cargo xtask plugin test ./example
cargo xtask plugin pack ./example --output example.mira-plugin
cargo xtask plugin inspect example.mira-plugin
```

Production signing requires an externally protected key and a release review. The CLI intentionally refuses `sign` when no configured signing provider exists.

