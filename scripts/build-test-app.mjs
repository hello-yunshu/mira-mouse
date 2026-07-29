#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-or-later
// Reproducible Latest Test App builder.
//
// The build runs from an isolated staging copy, never mutates the source
// checkout, and enables the explicit `test-plugin-trust` Cargo feature only in
// that staging config. The public TEST-ONLY seed is committed under tests/keys.
import { execFileSync } from 'node:child_process';
import {
  copyFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import { basename, dirname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const sourceRoot = fileURLToPath(new URL('..', import.meta.url));
const defaultPluginRepo = resolve(sourceRoot, '..', 'mira-mouse-plugins');
const pluginRepo = resolve(process.env.MIRA_PLUGIN_REPO || defaultPluginRepo);
const keyId = process.env.PLUGIN_KEY_ID || 'TEST-ONLY-mira-plugins';
const testSeedPath = join(sourceRoot, 'tests', 'keys', 'TEST-ONLY-mira-plugins.seed');
const testTrustedKeysPath = join(pluginRepo, 'registry', 'test-trusted-keys.json');
const outputRoot = join(sourceRoot, 'target', 'test-app');
const cargoTargetDir = join(outputRoot, 'cargo');
const manifestPath = join(outputRoot, 'manifest.json');
const stagingParent = mkdtempSync(join(tmpdir(), 'mira-test-app-'));
const hostRoot = join(stagingParent, 'mira-mouse');
const resourcesDir = join(hostRoot, 'src-tauri', 'resources', 'plugins');
const lockPath = join(hostRoot, 'bundled-plugins.lock.json');
const tauriConfPath = join(hostRoot, 'src-tauri', 'tauri.conf.json');

function fail(message) {
  throw new Error(`build:test-app: ${message}`);
}

function sha256File(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex');
}

function gitSourceState(repo, excludedBuildInputs = []) {
  const commitSha = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: repo })
    .toString()
    .trim();
  const changed = execFileSync(
    'git',
    ['ls-files', '-m', '-d', '-o', '--exclude-standard', '-z'],
    { cwd: repo },
  )
    .toString()
    .split('\0')
    .filter(Boolean)
    .sort();
  const buildInputFiles = changed.filter(
    (rel) => !excludedBuildInputs.some((excluded) => rel === excluded || rel.startsWith(`${excluded}/`)),
  );
  const hash = createHash('sha256');
  hash.update(`base:${commitSha}\0`);
  for (const rel of buildInputFiles) {
    const path = join(repo, rel);
    hash.update(`path:${rel}\0`);
    hash.update(existsSync(path) ? readFileSync(path) : Buffer.from('<deleted>'));
    hash.update('\0');
  }
  return {
    commitSha,
    workingTreeDirty: changed.length > 0,
    sourceStateSha256: hash.digest('hex'),
    changedFiles: changed,
    buildInputFiles,
  };
}

function run(command, args, options = {}) {
  const cwd = options.cwd || hostRoot;
  console.log(`> ${command} ${args.join(' ')}`);
  execFileSync(command, args, { stdio: 'inherit', ...options, cwd });
}

function resolveCliPath() {
  if (process.env.MIRA_PLUGIN_CLI) return resolve(process.env.MIRA_PLUGIN_CLI);
  const base = join(sourceRoot, 'target', 'release', 'mira-plugin');
  return process.platform === 'win32' ? `${base}.exe` : base;
}

function ensureCliBuilt(cliPath) {
  if (process.env.MIRA_PLUGIN_CLI) {
    if (!existsSync(cliPath)) fail(`explicit MIRA_PLUGIN_CLI does not exist: ${cliPath}`);
    return;
  }
  console.log(`building mira-plugin CLI from current Host source`);
  run('cargo', ['build', '--release', '-p', 'mira-plugin-cli'], { cwd: sourceRoot });
  if (!existsSync(cliPath)) fail(`CLI build completed but binary is missing: ${cliPath}`);
}

function shouldCopySource(path) {
  if (path === sourceRoot) return true;
  const rel = relative(sourceRoot, path);
  if (rel.startsWith('..')) return true;
  const first = rel.split(sep)[0];
  return !new Set(['.git', 'target', 'node_modules', 'dist']).has(first);
}

function prepareStagingTree() {
  console.log(`staging source at ${hostRoot}`);
  cpSync(sourceRoot, hostRoot, {
    recursive: true,
    filter: shouldCopySource,
    preserveTimestamps: true,
  });
  const sourceModules = join(sourceRoot, 'node_modules');
  const stagedModules = join(hostRoot, 'node_modules');
  if (existsSync(sourceModules)) {
    symlinkSync(sourceModules, stagedModules, 'dir');
  } else {
    run('npm', ['ci']);
  }
}

function discoverPlugins() {
  const pluginsDir = join(pluginRepo, 'plugins');
  if (!existsSync(pluginsDir)) fail(`plugins directory not found: ${pluginsDir}`);
  return readdirSync(pluginsDir, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => join(pluginsDir, entry.name))
    .filter((path) => existsSync(join(path, 'plugin.json')))
    .sort();
}

function readManifest(pluginDir) {
  return JSON.parse(readFileSync(join(pluginDir, 'plugin.json'), 'utf8'));
}

function readTestSeed() {
  const seed = readFileSync(testSeedPath, 'utf8').trim();
  if (!/^[0-9a-f]{64}$/i.test(seed)) {
    fail(`TEST-ONLY seed must contain exactly 32 hexadecimal bytes: ${testSeedPath}`);
  }
  return seed;
}

function packPlugin(cliPath, seed, pluginDir, outputPath) {
  const env = {
    ...process.env,
    MIRA_PLUGIN_CLI: cliPath,
    PLUGIN_KEY_ID: keyId,
    PLUGIN_SIGNING_KEY_HEX: seed,
  };
  run('node', [join(pluginRepo, 'scripts', 'pack-sign.mjs'), pluginDir, outputPath], {
    env,
  });
  run(cliPath, [
    'verify',
    outputPath,
    '--trusted-keys',
    testTrustedKeysPath,
    '--require-signature',
  ]);
}

function writeTestLock(plugins) {
  const requiredStringFields = [
    'pluginId',
    'repository',
    'releaseTag',
    'version',
    'asset',
    'sha256',
    'publisherKeyId',
    'pluginApi',
  ];
  for (const plugin of plugins) {
    for (const field of requiredStringFields) {
      if (typeof plugin[field] !== 'string' || plugin[field].length === 0) {
        fail(`generated lock entry ${plugin.pluginId || '<unknown>'} is missing ${field}`);
      }
    }
    if (typeof plugin.bundleByDefault !== 'boolean') {
      fail(`generated lock entry ${plugin.pluginId} is missing bundleByDefault`);
    }
  }
  const lock = {
    schemaVersion: 1,
    releaseReady: false,
    _comment:
      'Isolated Latest Test App bundle. TEST-ONLY public key; never use for production releases.',
    plugins,
  };
  writeFileSync(lockPath, `${JSON.stringify(lock, null, 2)}\n`);
}

function writeTestTauriConfig(assets) {
  const config = JSON.parse(readFileSync(tauriConfPath, 'utf8'));
  const nonPluginResources = config.bundle.resources.filter(
    (resource) => !resource.startsWith('resources/plugins/mira-'),
  );
  config.bundle.resources = [
    ...nonPluginResources,
    ...assets
      .filter((asset) => asset.bundleByDefault)
      .map((asset) => `resources/plugins/${asset.asset}`),
  ].sort();
  const features = new Set(config.build.features || []);
  features.add('test-plugin-trust');
  config.build.features = [...features].sort();
  writeFileSync(tauriConfPath, `${JSON.stringify(config, null, 2)}\n`);
}

function findFile(root, predicate) {
  if (!existsSync(root)) return undefined;
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const path = join(root, entry.name);
    if (entry.isDirectory()) {
      const nested = findFile(path, predicate);
      if (nested) return nested;
    } else if (entry.isFile() && predicate(path)) {
      return path;
    }
  }
  return undefined;
}

function findBuiltExecutable() {
  const bundleRoot = join(cargoTargetDir, 'release', 'bundle');
  if (process.platform === 'darwin') {
    return findFile(
      bundleRoot,
      (path) => path.includes(`${sep}Contents${sep}MacOS${sep}`) && basename(path).toLowerCase() === 'mira',
    );
  }
  if (process.platform === 'win32') {
    return findFile(bundleRoot, (path) => basename(path).toLowerCase() === 'mira.exe');
  }
  return findFile(bundleRoot, (path) => path.endsWith('.AppImage'));
}

function verifyMacBundle(executablePath) {
  if (process.platform !== 'darwin') return;
  const bundleRoot = dirname(dirname(dirname(executablePath)));
  // Rebuilding over an app that Finder has opened can preserve FinderInfo or
  // resource-fork xattrs on the bundle root. They are not product content and
  // make strict code-signature validation reject an otherwise valid bundle.
  run('xattr', ['-cr', bundleRoot], { cwd: sourceRoot });
  run(
    'codesign',
    ['--verify', '--deep', '--strict', '--verbose=2', bundleRoot],
    { cwd: sourceRoot },
  );
}

function writeBuildManifest(cliPath, cliSha256, installedAssets, executablePath) {
  const appSha256 = sha256File(executablePath);
  const hostState = gitSourceState(sourceRoot, [
    'docs',
    'scripts/smoke-test-built-app.sh',
  ]);
  const pluginState = gitSourceState(pluginRepo);
  const data = {
    schemaVersion: 1,
    testOnly: true,
    releaseReady: false,
    sourceRoot,
    pluginRepo,
    hostSha: hostState.commitSha,
    hostWorkingTreeDirty: hostState.workingTreeDirty,
    hostSourceStateSha256: hostState.sourceStateSha256,
    hostChangedFiles: hostState.changedFiles,
    hostBuildInputFiles: hostState.buildInputFiles,
    pluginSha: pluginState.commitSha,
    pluginWorkingTreeDirty: pluginState.workingTreeDirty,
    pluginSourceStateSha256: pluginState.sourceStateSha256,
    pluginChangedFiles: pluginState.changedFiles,
    pluginBuildInputFiles: pluginState.buildInputFiles,
    cliPath,
    cliSha256,
    executablePath,
    executableSha256: appSha256,
    bundleRoot: dirname(dirname(dirname(executablePath))),
    plugins: installedAssets,
  };
  mkdirSync(outputRoot, { recursive: true });
  writeFileSync(manifestPath, `${JSON.stringify(data, null, 2)}\n`);
  return data;
}

function main() {
  console.log('=== build:test-app start ===');
  if (!existsSync(pluginRepo)) fail(`plugin repository not found: ${pluginRepo}`);
  if (!existsSync(testTrustedKeysPath)) {
    fail(`test trust store not found: ${testTrustedKeysPath}`);
  }

  const cliPath = resolveCliPath();
  ensureCliBuilt(cliPath);
  const cliSha256 = sha256File(cliPath);
  const seed = readTestSeed();
  prepareStagingTree();

  const pluginDirs = discoverPlugins();
  if (pluginDirs.length === 0) fail('no plugins discovered');
  const manifests = pluginDirs.map((dir) => ({ dir, manifest: readManifest(dir) }));
  console.log(`discovered ${manifests.length} plugin(s)`);

  run('npm', ['run', 'build']);
  for (const { dir, manifest } of manifests) {
    console.log(`validating ${manifest.pluginId} v${manifest.version}`);
    run(cliPath, ['validate', dir]);
    run(cliPath, ['test', dir]);
  }

  rmSync(resourcesDir, { recursive: true, force: true });
  mkdirSync(resourcesDir, { recursive: true });
  const packageDir = join(stagingParent, 'packages');
  mkdirSync(packageDir, { recursive: true });
  const originalLock = JSON.parse(
    readFileSync(join(sourceRoot, 'bundled-plugins.lock.json'), 'utf8'),
  );
  const lockedPlugins = new Map(
    originalLock.plugins.map((plugin) => [plugin.pluginId, plugin]),
  );
  const installedAssets = [];

  for (const { dir, manifest } of manifests) {
    const lockedPlugin = lockedPlugins.get(manifest.pluginId);
    if (!lockedPlugin) {
      fail(`plugin ${manifest.pluginId} has no entry in bundled-plugins.lock.json`);
    }
    const asset = `${manifest.pluginId.replace(/\./g, '-')}-${manifest.version}.mira-plugin`;
    const packagePath = join(packageDir, asset);
    packPlugin(cliPath, seed, dir, packagePath);
    const destination = join(resourcesDir, asset);
    copyFileSync(packagePath, destination);
    installedAssets.push({
      pluginId: manifest.pluginId,
      repository: lockedPlugin.repository,
      version: manifest.version,
      asset,
      sha256: sha256File(destination),
      publisherKeyId: keyId,
      pluginApi: manifest.pluginApi || '>=1.1.0, <2.0.0',
      releaseTag: `plugin/${manifest.pluginId.replace(/^mira\./, '')}/v${manifest.version}`,
      bundleByDefault: lockedPlugin.bundleByDefault,
    });
  }

  writeTestLock(installedAssets);
  writeTestTauriConfig(installedAssets);

  const bundle = process.env.TAURI_BUNDLE || (process.platform === 'darwin' ? 'app' : 'app');
  const env = {
    ...process.env,
    CARGO_TARGET_DIR: cargoTargetDir,
  };
  run('npm', ['exec', 'tauri', '--', 'build', '--bundles', bundle], { env });
  const executablePath = findBuiltExecutable();
  if (!executablePath) {
    fail(`built executable not found below ${join(cargoTargetDir, 'release', 'bundle')}`);
  }
  verifyMacBundle(executablePath);
  const result = writeBuildManifest(
    cliPath,
    cliSha256,
    installedAssets,
    executablePath,
  );

  console.log('=== build:test-app summary ===');
  console.log(`CLI SHA-256: ${result.cliSha256}`);
  console.log(`App: ${result.executablePath}`);
  console.log(`App SHA-256: ${result.executableSha256}`);
  console.log(`Manifest: ${manifestPath}`);
}

try {
  main();
} finally {
  rmSync(stagingParent, { recursive: true, force: true });
}
