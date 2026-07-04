<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Plugin Security

Mira treats every plugin and Fixture as hostile input. It validates the ZIP structure without writing entries, checks exact content coverage, verifies signatures against explicit trust roots, constrains device matching and report behavior, and executes only bounded declarative operations. Conflicting device matches force read-only behavior. Developer-mode unsigned imports are visibly untrusted and cannot write.

