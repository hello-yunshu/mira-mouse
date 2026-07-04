<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Unsigned Community Releases

`unsigned-community` means the platform has not established a trusted developer identity. It does not mean notarized, Windows-trusted, GPG-signed, or updater-signed.

Download only from the configured official GitHub Release, verify the separately published SHA-256, and review the first-launch warning.

Plugin packages are signed independently with Ed25519 and verified by the application against a pinned public key. That signature is independent of the platform-level installer warning.
