#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-or-later
// Verify that the signed plugin archives bundled with this app can be rendered
// by the declarative host and resolve every user-visible label in both shipped
// languages. This catches app/plugin release skew before packaging a DMG.
import { execFile } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { promisify } from 'node:util';

const run = promisify(execFile);
const languages = ['zh-CN', 'en'];
const lock = JSON.parse(await readFile('plugins.lock.json', 'utf8'));
const hostLocales = new Map(await Promise.all(languages.map(async (language) => [
  language,
  flattenKeys(JSON.parse(await readFile(`src/locales/${language}.json`, 'utf8'))),
])));
const failures = [];
let verified = 0;
const translatableKeyProperties = new Set(['labelKey', 'editTitleKey', 'editLabelKey']);

for (const plugin of lock.plugins.filter((entry) => entry.bundleByDefault)) {
  const manifest = await readArchiveJson(plugin.cachePath, 'plugin.json');
  if (manifest.pluginId !== plugin.pluginId || manifest.version !== plugin.version) {
    failures.push(`${plugin.asset}: lock identity does not match plugin.json`);
  }
  if (manifest.publisherKeyId !== plugin.publisherKeyId) {
    failures.push(`${plugin.asset}: publisher key does not match the lock`);
  }

  const pluginLocales = new Map(await Promise.all(languages.map(async (language) => [
    language,
    await readArchiveJson(plugin.cachePath, `locales/${language}.json`),
  ])));
  const labelKeys = new Set();
  for (const capability of manifest.capabilities ?? []) {
    validateCapability(plugin.asset, capability, failures);
    collectLabelKeys(capability, labelKeys);
  }
  for (const language of languages) {
    const hostKeys = hostLocales.get(language);
    const pluginKeys = pluginLocales.get(language);
    for (const labelKey of labelKeys) {
      if (!Object.hasOwn(pluginKeys, labelKey)
        && !hostKeys.has(labelKey)
        && !hostKeys.has(`plugin.label.${labelKey}`)) {
        failures.push(`${plugin.asset}: ${language} has no translation for ${labelKey}`);
      }
    }
    if (language === 'en') {
      for (const [key, value] of Object.entries(pluginKeys)) {
        if (typeof value === 'string' && /\p{Script=Han}/u.test(value)) {
          failures.push(`${plugin.asset}: English translation ${key} contains Chinese text`);
        }
      }
    }
  }
  verified += 1;
}

if (failures.length) {
  throw new Error(`bundled plugin contract failed:\n${failures.map((failure) => `- ${failure}`).join('\n')}`);
}
console.log(`bundled plugin contract: ${verified} signed plugin archive(s) verified`);

async function readArchiveJson(archive, entry) {
  const { stdout } = await run('unzip', ['-p', archive, entry], { encoding: 'utf8' });
  if (!stdout) throw new Error(`${archive}: missing ${entry}`);
  return JSON.parse(stdout);
}

function flattenKeys(value, prefix = '', output = new Set()) {
  for (const [key, child] of Object.entries(value)) {
    const path = prefix ? `${prefix}.${key}` : key;
    if (child && typeof child === 'object' && !Array.isArray(child)) {
      flattenKeys(child, path, output);
    } else {
      output.add(path);
    }
  }
  return output;
}

function collectLabelKeys(value, output) {
  if (Array.isArray(value)) {
    value.forEach((item) => collectLabelKeys(item, output));
    return;
  }
  if (!value || typeof value !== 'object') return;
  for (const [key, child] of Object.entries(value)) {
    if (translatableKeyProperties.has(key) && typeof child === 'string') output.add(child);
    collectLabelKeys(child, output);
  }
}

function validateCapability(asset, capability, errors) {
  const metadata = capability.metadata ?? {};
  const fields = [
    ...(metadata.fields ?? []),
    ...(metadata.zones ?? []).flatMap((zone) => zone.fields ?? []),
  ];
  if (capability.control === 'DpiStages' && !metadata.stageLayout) {
    errors.push(`${asset}: ${capability.id} is missing metadata.stageLayout`);
  }
  if (capability.control === 'LightingZone' && !metadata.zones) {
    errors.push(`${asset}: ${capability.id} is missing metadata.zones`);
  }
  if (!capability.readOnly
    && !['DpiStages', 'LightingZone'].includes(capability.control)
    && fields.length === 0) {
    errors.push(`${asset}: writable ${capability.id} has no declarative fields`);
  }
  const actionField = metadata.statusDisplay?.onClickField;
  if (actionField && !fields.some((field) => field.id === actionField)) {
    errors.push(`${asset}: ${capability.id} statusDisplay references missing field ${actionField}`);
  }
}
