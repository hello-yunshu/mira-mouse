// SPDX-License-Identifier: AGPL-3.0-or-later
import { statSync } from 'node:fs';
import { resolve } from 'node:path';

const modelPack = resolve('src-tauri/resources/local-ai/model.rillpack');
const handlerPack = resolve('src-tauri/resources/local-ai/handler.rillhandler');

function requiredSize(path, message) {
  let size = 0;
  try {
    size = statSync(path).size;
  } catch {
    // The common error below also explains how CI/local builds should provide it.
  }
  if (size === 0) throw new Error(message);
  return size;
}

const modelSize = requiredSize(
  modelPack,
  'Local AI model pack is missing or empty. Build/download a signed model.rillpack before running a Tauri bundle build.',
);
const handlerSize = requiredSize(
  handlerPack,
  'Local AI handler pack is missing or empty. Build/download a signed handler.rillhandler before running a Tauri bundle build.',
);

console.log(`local AI model pack: ${modelSize} bytes`);
console.log(`local AI handler pack: ${handlerSize} bytes`);
