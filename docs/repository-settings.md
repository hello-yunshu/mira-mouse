<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Repository Settings

All items are `blocked` until a real GitHub repository and authorized owner are configured:

- Enable Private Vulnerability Reporting.
- Require the core CI, security, and dependency review jobs with a Ruleset.
- Require CODEOWNERS review for workflows, trust roots, locks, and release code.
- Create protected `release`, `plugin-release`, and optional notarization/signing environments.
- Configure minimal GitHub App or fine-grained cross-repository credentials.
- Configure production updater/plugin keys and platform credentials as environment secrets.
- Set appropriate log/artifact retention; keep formal Release assets indefinitely.
- Enable artifact attestations and immutable releases where supported.

