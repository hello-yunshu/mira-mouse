<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Install on macOS

The community DMG is published on the [GitHub Releases](https://github.com/hello-yunshu/mira-mouse/releases) page with the stable name `Mira_macOS_<version>_aarch64.dmg`. Mira for macOS supports Apple Silicon (ARM64) only; no Intel build is provided.

## Option A: Homebrew (recommended)

```bash
brew tap hello-yunshu/mira
brew install --cask mira
```

Upgrades follow the standard Homebrew flow:

```bash
brew upgrade --cask mira
```

See [install-homebrew.md](install-homebrew.md) for details on the tap, the unsigned-app caveats, and the available variables.

## Option B: Direct DMG download

1. Download `Mira_macOS_<version>_aarch64.dmg` from the latest release.
2. Verify the SHA-256 published next to the asset on the release page.
3. Mount the DMG and drag `Mira.app` to `/Applications`.

## First-launch warning

Mira is built with ad-hoc signing (`signingIdentity: "-"`) and is **not** notarized. The first launch is blocked by Gatekeeper. To proceed, do one of the following:

- Right-click `Mira.app` → **Open** → confirm in the Gatekeeper dialog.
- Open **System Settings → Privacy & Security** → click **Open Anyway** next to the Mira block.
- Run once in Terminal:

  ```bash
  xattr -dr com.apple.quarantine /Applications/Mira.app
  ```

## Permissions

After launching, grant HID access in **System Settings → Privacy & Security → Input Monitoring** if Mira needs to communicate with the mouse.
