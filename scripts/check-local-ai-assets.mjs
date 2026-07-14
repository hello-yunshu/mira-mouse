// SPDX-License-Identifier: AGPL-3.0-or-later
import { statSync } from 'node:fs';
import { resolve } from 'node:path';

const modelPack = resolve('src-tauri/resources/local-ai/model.rillpack');

let size = 0;
try {
  size = statSync(modelPack).size;
} catch {
  // The common error below also explains how CI/local builds should provide it.
}

if (size === 0) {
  throw new Error(
    'Local AI model pack is missing or empty. Build/download a signed model.rillpack before running a Tauri bundle build.',
  );
}

console.log(`local AI model pack: ${size} bytes`);
