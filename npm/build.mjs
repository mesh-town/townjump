#!/usr/bin/env node
// Usage: node npm/build.mjs <version> <assets-dir> <out-dir>
// Builds one npm package per platform plus the main package from the release archives.
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import {
  chmodSync, copyFileSync, cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync,
  readdirSync, renameSync, rmSync, writeFileSync,
} from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const REPOSITORY = { type: 'git', url: 'git+https://github.com/mesh-town/townjump.git' };
const HOMEPAGE = 'https://mesh-town.github.io/townjump/';

function sha256(file) {
  return createHash('sha256').update(readFileSync(file)).digest('hex');
}

function readSums(assets) {
  const file = join(assets, 'SHA256SUMS');
  if (!existsSync(file)) throw new Error(`missing ${file}`);
  const sums = new Map();
  for (const line of readFileSync(file, 'utf8').split('\n')) {
    const m = /^([0-9a-fA-F]{64}) [ *](.+)$/.exec(line.trim());
    if (m) sums.set(m[2].replace(/^\.\//, ''), m[1].toLowerCase());
  }
  return sums;
}

function extract(archive, dest) {
  mkdirSync(dest, { recursive: true });
  if (archive.endsWith('.zip')) execFileSync('unzip', ['-q', archive, '-d', dest]);
  else execFileSync('tar', ['-xzf', archive, '-C', dest]);
}

function writeJson(file, value) {
  writeFileSync(file, JSON.stringify(value, null, 2) + '\n');
}

function verifyArchives(version, assets, platforms) {
  const sums = readSums(assets);
  const archives = {};
  for (const [key, p] of Object.entries(platforms)) {
    const name = `townjump-${version}-${p.target}.${p.archive}`;
    const file = join(assets, name);
    if (!existsSync(file)) throw new Error(`missing archive ${name}`);
    if (!sums.has(name)) throw new Error(`${name} is not listed in SHA256SUMS`);
    if (sums.get(name) !== sha256(file)) throw new Error(`checksum mismatch for ${name}`);
    archives[key] = file;
  }
  return archives;
}

function buildPlatform(version, key, p, archive, stage, work) {
  const unpacked = join(work, key);
  extract(archive, unpacked);
  const inner = join(unpacked, `townjump-${version}-${p.target}`);
  const exe = p.exe || 'townjump';
  if (!existsSync(join(inner, exe))) throw new Error(`${archive} has no ${exe}`);
  const dir = join(stage, key);
  mkdirSync(join(dir, 'bin'), { recursive: true });
  copyFileSync(join(inner, exe), join(dir, 'bin', exe));
  chmodSync(join(dir, 'bin', exe), 0o755);
  copyFileSync(join(inner, 'LICENSE'), join(dir, 'LICENSE'));
  writeJson(join(dir, 'package.json'), {
    name: `@mesh-town/townjump-${key}`,
    version,
    description: `townjump binary for ${key}`,
    license: 'AGPL-3.0-only',
    repository: REPOSITORY,
    homepage: HOMEPAGE,
    os: [p.os],
    cpu: [p.cpu],
    files: ['bin/', 'LICENSE'],
  });
}

function buildMain(version, platforms, stage) {
  const dir = join(stage, 'townjump');
  cpSync(join(here, 'townjump'), dir, { recursive: true });
  copyFileSync(join(here, 'platforms.json'), join(dir, 'platforms.json'));
  copyFileSync(join(here, '..', 'LICENSE'), join(dir, 'LICENSE'));
  const pkg = JSON.parse(readFileSync(join(dir, 'package.json'), 'utf8'));
  pkg.version = version;
  pkg.optionalDependencies = Object.fromEntries(
    Object.keys(platforms).map((key) => [`@mesh-town/townjump-${key}`, version]),
  );
  writeJson(join(dir, 'package.json'), pkg);
}

export function build(version, assetsArg, outArg) {
  const platforms = JSON.parse(readFileSync(join(here, 'platforms.json'), 'utf8'));
  const assets = resolve(assetsArg);
  const out = resolve(outArg);
  const archives = verifyArchives(version, assets, platforms);
  mkdirSync(out, { recursive: true });
  // Build next to the output and move into place at the end, so a failure leaves nothing half-written.
  const work = mkdtempSync(join(out, '.build-'));
  try {
    const stage = join(work, 'stage');
    for (const [key, p] of Object.entries(platforms)) {
      buildPlatform(version, key, p, archives[key], stage, join(work, 'unpack'));
    }
    buildMain(version, platforms, stage);
    for (const name of readdirSync(stage)) {
      rmSync(join(out, name), { recursive: true, force: true });
      renameSync(join(stage, name), join(out, name));
    }
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [version, assets, out] = process.argv.slice(2);
  if (!version || !assets || !out) {
    console.error('usage: node npm/build.mjs <version> <assets-dir> <out-dir>');
    process.exit(2);
  }
  try {
    build(version, assets, out);
  } catch (e) {
    console.error(`build: ${e.message}`);
    process.exit(1);
  }
}
