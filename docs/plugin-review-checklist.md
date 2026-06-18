<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Plugin Review Checklist

- [ ] Device matches include interface conditions and do not conflict.
- [ ] Every capability has range, unit, localization, evidence, and risk.
- [ ] Writes are disabled without model-specific hardware readback evidence.
- [ ] Unknown fields are preserved and rollback behavior is explicit.
- [ ] Package contains no code, remote resource, secret, research source, or manufacturer binary.
- [ ] Fixtures cover success and failure paths and deterministic packing matches.

