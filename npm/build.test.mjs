import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdtempSync, mkdirSync, writeFileSync, readFileSync, statSync, readdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { build } from './build.mjs';

const require = createRequire(import.meta.url);
const platforms = require('./platforms.json');
const VERSION = '9.9.9';

function fakeAssets() {
  const root = mkdtempSync(join(tmpdir(), 'tj-npm-'));
  const assets = join(root, 'assets');
  mkdirSync(assets);
  const sums = [];
  for (const p of Object.values(platforms)) {
    const base = `townjump-${VERSION}-${p.target}`;
    const dir = join(root, 'src', base);
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, p.exe || 'townjump'), '#!/bin/sh\necho fake\n');
    writeFileSync(join(dir, 'LICENSE'), 'license\n');
    writeFileSync(join(dir, 'README.md'), 'readme\n');
    const name = `${base}.${p.archive}`;
    if (p.archive === 'zip') execFileSync('zip', ['-qr', join(assets, name), base], { cwd: join(root, 'src') });
    else execFileSync('tar', ['-czf', join(assets, name), base], { cwd: join(root, 'src') });
    const hash = createHash('sha256').update(readFileSync(join(assets, name))).digest('hex');
    sums.push(`${hash}  ${name}`);
  }
  writeFileSync(join(assets, 'SHA256SUMS'), sums.join('\n') + '\n');
  return { root, assets, out: join(root, 'out'), sums };
}

test('build makes eight packages from fake assets', () => {
  const { root, assets, out } = fakeAssets();
  try {
    build(VERSION, assets, out);
    const names = readdirSync(out).sort();
    assert.equal(names.length, 8);
    const main = JSON.parse(readFileSync(join(out, 'townjump', 'package.json'), 'utf8'));
    assert.equal(main.version, VERSION);
    assert.equal(Object.keys(main.optionalDependencies).length, 7);
    for (const key of Object.keys(platforms)) {
      const pkg = JSON.parse(readFileSync(join(out, key, 'package.json'), 'utf8'));
      assert.equal(pkg.name, `@mesh-town/townjump-${key}`);
      assert.equal(pkg.version, VERSION);
      assert.equal(main.optionalDependencies[pkg.name], VERSION);
      const exe = platforms[key].exe || 'townjump';
      assert.equal(statSync(join(out, key, 'bin', exe)).mode & 0o111, 0o111);
    }
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test('build rejects checksum mismatch', () => {
  const { root, assets, out } = fakeAssets();
  try {
    const sums = readFileSync(join(assets, 'SHA256SUMS'), 'utf8');
    writeFileSync(join(assets, 'SHA256SUMS'), sums.replace(/^./, (c) => (c === '0' ? '1' : '0')));
    assert.throws(() => build(VERSION, assets, out), /checksum mismatch/);
    assert.equal(existsSync(out), false);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test('build rejects missing archive', () => {
  const { root, assets, out } = fakeAssets();
  try {
    rmSync(join(assets, `townjump-${VERSION}-aarch64-linux-android.tar.gz`));
    assert.throws(() => build(VERSION, assets, out), /missing archive/);
    assert.equal(existsSync(out), false);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
