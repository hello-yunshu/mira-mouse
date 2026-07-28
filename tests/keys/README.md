# TEST-ONLY plugin signing key

`TEST-ONLY-mira-plugins.seed` is a public, deterministic Ed25519 test seed.
It is not a secret and must never be used to sign production plugin releases.

`npm run build:test-app` uses it only inside an isolated staging copy. The
resulting app enables the `test-plugin-trust` Cargo feature, while ordinary
release builds do not trust this key.
