// SPDX-License-Identifier: AGPL-3.0-or-later
import { createHash } from 'node:crypto';
import { readFileSync, statSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

const [templateArg, componentArg, version, outputArg] = process.argv.slice(2);
if (!templateArg || !componentArg || !version || !outputArg) {
  throw new Error('usage: build-handler-manifest.mjs TEMPLATE COMPONENT VERSION OUTPUT');
}

const template = JSON.parse(readFileSync(resolve(templateArg), 'utf8'));
const component = readFileSync(resolve(componentArg));
if (component.length === 0) throw new Error('handler component is empty');
template.version = version;
template.moduleSha256 = createHash('sha256').update(component).digest('hex');
template.moduleSize = statSync(resolve(componentArg)).size;
writeFileSync(resolve(outputArg), `${JSON.stringify(template, null, 2)}\n`);
