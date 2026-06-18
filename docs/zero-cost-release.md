<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Zero-Cost Release Guide

This project can be built and released without spending money on code-signing certificates or Apple Developer memberships. The trade-off is that users will see first-launch security warnings on macOS and Windows.

> This guide is for maintainers. If you are a user, see [unsigned-release-security.md](unsigned-release-security.md) and the platform install notes below.

## What is free vs. what costs money

| Trust layer | Free option | Paid option | User impact if free |
|---|---|---|---|
| Plugin package signature | Self-generated Ed25519 key | Same; no CA needed | None — the app verifies the key it ships with |
| Tauri auto-updater signature | Self-generated Ed25519 key | Same; no CA needed | None if public key is embedded |
| macOS app/DMG signature | Ad-hoc (`signingIdentity: "-"`) | Apple Developer ID + notarization | Gatekeeper still warns; user clicks “Open Anyway” in Privacy & Security or runs `xattr -cr` |
| Windows installer signature | Self-signed certificate | OV/EV code signing certificate | SmartScreen / Unknown Publisher warning |
| Linux package signature | Self-generated GPG key | Same; no CA needed | Users may need to import your GPG key or ignore |

## 1. Generate keys (do this once)

### Plugin signing key (Ed25519)

Run this in the **plugin repository** root:

```bash
openssl genpkey -algorithm ed25519 -out mira-plugins-prod.key.pem
openssl pkey -in mira-plugins-prod.key.pem -pubout -outform DER | tail -c 32 > mira-plugins-prod.pub.raw
```

- `mira-plugins-prod.key.pem` is the **private key**.
- `mira-plugins-prod.pub.raw` is the **32-byte raw public key** that the application uses to verify plugins.

### Tauri updater key (optional)

If you want auto-updates:

```bash
cargo tauri signer generate
```

This produces `tauri.key` (private) and `tauri.key.pub` (public). Keep `tauri.key` secret and put `tauri.key.pub` into `src-tauri/tauri.conf.json`.

### Windows self-signed certificate (optional)

```powershell
New-SelfSignedCertificate `
  -Subject "CN=Mira Open Source Project" `
  -Type CodeSigningCert `
  -CertStoreLocation Cert:\CurrentUser\My
```

Export the PFX and add it to GitHub Secrets as `WINDOWS_PFX`. The zero-cost app workflow does not use it by default; you can extend the workflow if you want.

## 2. Configure GitHub Secrets

In the **plugin repository**, go to `Settings > Environments > plugin-release > Environment secrets` and add:

| Secret | Value |
|---|---|
| `MIRA_PLUGIN_SIGNING_KEY` | Full content of `mira-plugins-prod.key.pem` |
| `MIRA_PLUGIN_KEY_ID` | A stable identifier, e.g., `mira-plugins-2026` |

In the **main repository**, if you use the Tauri updater, add:

| Secret | Value |
|---|---|
| `TAURI_PRIVATE_KEY` | Full content of `tauri.key` |

If you created a Windows PFX, add it as a base64-encoded secret named `WINDOWS_PFX` and add the password as `WINDOWS_PFX_PASSWORD`.

## 3. Embed the plugin public key in the app

The application needs to know which public key to trust. The simplest zero-cost approach is to ship the raw public key as a resource or hard-code it in the plugin runtime trust store. Replace any test-only key with the contents of `mira-plugins-prod.pub.raw` before release.

## 4. Release a plugin

Tag the plugin repository using the directory name:

```bash
cd mira-mouse-plugins
git tag plugin/example-mock/v1.0.0
git push origin plugin/example-mock/v1.0.0
```

The `Plugin Release` workflow will:

1. Run `npm run validate && npm test`.
2. Pack and sign the plugin with `scripts/pack-sign.mjs` using `secrets.MIRA_PLUGIN_SIGNING_KEY`.
3. Publish the `.mira-plugin`, its `.sha256`, and build attestation to a **draft** GitHub Release.

## 5. Release the application

1. Ensure `plugins.lock.json` is `releaseReady: true` for every plugin marked `bundleByDefault: true`. Until `mira.amaster` has real release metadata, the gate will block.
2. Push a tag in the main repository:

   ```bash
   cd mira-mouse
   git tag app/v0.1.0
   git push origin app/v0.1.0
   ```

3. The `Release` workflow builds DMG, NSIS installer, AppImage, DEB, and RPM packages with ad-hoc macOS signing and attaches them to a draft release.

## 6. First-launch warnings for users

Because these packages are unsigned at the platform level, users must bypass OS warnings.

### macOS

The app is built with ad-hoc signing (`signingIdentity: "-"`), which avoids the scarier “App is damaged” dialog but still triggers Gatekeeper.

1. Try to open the app.
2. Open **System Settings > Privacy & Security**.
3. Scroll down and click **Open Anyway** next to the Mira block.
4. Or run once in Terminal:

```bash
xattr -cr /Applications/Mira.app
```

### Windows

Click **More info** on the SmartScreen dialog, then **Run anyway**.

### Linux

AppImage: make it executable and run:

```bash
chmod +x Mira_*.AppImage
./Mira_*.AppImage
```

DEB/RPM: install normally; no signature warning if you do not enable repository-level signature checks.

## 7. Verify a plugin before installing

A user or maintainer can verify the plugin package independently:

```bash
sha256sum mira-example-mock-1.0.0.mira-plugin
# compare with the .sha256 file from the release

# Verify Ed25519 signature if you have the public key
node scripts/verify-test.mjs mira-example-mock-1.0.0.mira-plugin mira-plugins-prod.pub.raw
```

## Security notes

- Treat `mira-plugins-prod.key.pem` and `tauri.key` like passwords. Never commit them.
- TEST-ONLY keys (the ones generated locally by `scripts/pack-sign.mjs`) must never be used for releases.
- The plugin signature is independent of platform signing. Even if macOS/Windows warn about the app, the plugin package inside is still integrity-protected.
