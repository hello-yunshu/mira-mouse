#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-or-later
import { readFile } from 'node:fs/promises';

const cargoToml = await readFile('Cargo.toml', 'utf8');
const workspacePackage = cargoToml.match(/\[workspace\.package\]([\s\S]*?)(?:\n\[|$)/);
const version = workspacePackage?.[1].match(/^\s*version\s*=\s*"([^"]+)"/m)?.[1];
if (!version) {
  throw new Error('Cargo.toml [workspace.package] version is missing');
}
console.log(version);
