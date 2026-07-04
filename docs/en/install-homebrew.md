<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Install via Homebrew (macOS)

Mira is distributed as a Homebrew Cask through a self-hosted tap at [`hello-yunshu/homebrew-mira`](https://github.com/hello-yunshu/homebrew-mira). The Cask wraps the unsigned community DMG published on the main repository's GitHub Releases page.

## Install

```bash
brew tap hello-yunshu/mira
brew trust hello-yunshu/mira
brew install --cask mira
```

`brew trust` is required by Homebrew 4.x for third-party taps. It marks the tap as a trusted source so its Casks can be loaded. Without it, `brew install --cask mira` fails with `Refusing to load cask ... from untrusted tap`.

`brew install --cask mira` mounts the DMG, copies `Mira.app` into `/Applications`, and warns about the unsigned-app caveats described below.

## Upgrade

```bash
brew update
brew upgrade --cask mira
```

The CI pipeline pushes a fresh `Casks/mira.rb` to the tap repository within a few minutes of each release, so `brew upgrade` reaches the latest version without manual intervention.

## Uninstall

```bash
brew uninstall --cask mira
brew untap hello-yunshu/mira
```

## First-launch warning (unsigned and not notarized)

Mira is built with ad-hoc signing (`signingIdentity: "-"`) and is **not** notarized. Homebrew installs the app with the macOS quarantine attribute set, so the first launch is blocked by Gatekeeper. To proceed, do one of the following:

- Right-click `Mira.app` → **Open** → confirm in the Gatekeeper dialog.
- Open **System Settings → Privacy & Security** → click **Open Anyway** next to the Mira block.
- Run once in Terminal:

  ```bash
  xattr -dr com.apple.quarantine /Applications/Mira.app
  ```

You can also pass `--no-quarantine` at install time to skip the attribute entirely (use only if you trust the source):

```bash
brew install --cask --no-quarantine mira
```

## Verify the SHA-256

The Cask pins the SHA-256 of `Mira_macOS_<version>_universal.dmg`. Homebrew verifies it automatically during install. To check it manually:

```bash
brew info --cask mira
shasum -a 256 /Applications/Mira.app/..  # compare with the value printed by brew info
```

Or compare against the checksum published next to the asset on the [release page](https://github.com/hello-yunshu/mira-mouse/releases).

## HID permission

After launching, grant HID access in **System Settings → Privacy & Security → Input Monitoring** if Mira needs to communicate with the mouse.

## How the tap stays in sync

The `homebrew-tap` job in [`.github/workflows/pipeline.yml`](../.github/workflows/pipeline.yml) runs after each successful release. It downloads the DMG published by the `release-publish` job, computes its SHA-256, renders [`homebrew/Casks/mira.rb`](../homebrew/Casks/mira.rb) with the new `version` and `sha256`, and pushes the result to `hello-yunshu/homebrew-mira`.

The job authenticates with the `HOMEBREW_TAP_TOKEN` repository secret, which must be a Personal Access Token (classic) with `repo` scope on `hello-yunshu/homebrew-mira`. If the secret is missing, the job is skipped and the tap is not updated until the next release that has the secret configured.

## Manual tap update (maintainers)

If the CI job is unavailable, a maintainer can update the tap by hand:

```bash
git clone https://github.com/hello-yunshu/homebrew-mira.git
cd homebrew-mira
VERSION=0.5.2  # replace with the target version
SHA256=$(curl -sSL "https://github.com/hello-yunshu/mira-mouse/releases/download/app/v${VERSION}/Mira_macOS_${VERSION}_universal.dmg" | shasum -a 256 | awk '{print $1}')
sed -i.bak -e "s/^  version .*/  version \"${VERSION}\"/" \
           -e "s/^  sha256 .*/  sha256 \"${SHA256}\"/" Casks/mira.rb
rm Casks/mira.rb.bak
git commit -am "Bump mira to ${VERSION}"
git push
```
