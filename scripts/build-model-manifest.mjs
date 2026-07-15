// SPDX-License-Identifier: AGPL-3.0-or-later
import { readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

const [templateArg, version, outputArg] = process.argv.slice(2);
if (!templateArg || !version || !outputArg) {
  throw new Error('usage: build-model-manifest.mjs TEMPLATE VERSION OUTPUT');
}

const manifest = JSON.parse(readFileSync(resolve(templateArg), 'utf8'));
manifest.version = version;
writeFileSync(resolve(outputArg), `${JSON.stringify(manifest, null, 2)}\n`);
