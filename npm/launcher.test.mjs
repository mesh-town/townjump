import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const require = createRequire(import.meta.url);
const here = dirname(fileURLToPath(import.meta.url));
const launcher = require('./townjump/bin/townjump.js');
const platforms = require('./platforms.json');

test('platform key maps to package', () => {
  assert.equal(launcher.platformKey('darwin', 'arm64'), 'darwin-arm64');
  assert.equal(launcher.packageName('darwin-arm64'), '@mesh-town/townjump-darwin-arm64');
  assert.equal(launcher.binaryName('darwin-arm64', platforms), 'townjump');
  assert.equal(launcher.binaryName('win32-x64', platforms), 'townjump.exe');
  assert.equal(launcher.resolveBinary('plan9-mips', platforms), null);
});

test('unsupported platform prints install hint', () => {
  const script = join(here, 'townjump', 'bin', 'townjump.js');
  // Pretend to be an unsupported platform: override the properties the launcher reads.
  const preload = "Object.defineProperty(process,'platform',{value:'plan9'});" +
    "Object.defineProperty(process,'arch',{value:'mips'});";
  const r = spawnSync(process.execPath, ['-e',
    preload + `process.argv[1]=${JSON.stringify(script)};require('module')._load(${JSON.stringify(script)},null,true)`,
  ], { encoding: 'utf8' });
  assert.equal(r.status, 1);
  assert.match(r.stderr, /github\.com\/mesh-town\/townjump#install/);
  assert.match(r.stderr, /darwin-arm64/);
});
