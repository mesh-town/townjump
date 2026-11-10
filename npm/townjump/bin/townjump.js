#!/usr/bin/env node
'use strict';

const { spawn } = require('node:child_process');
const path = require('node:path');

const INSTALL_URL = 'https://github.com/mesh-town/townjump#install';

function readPlatforms() {
  // Published layout: platforms.json sits next to bin/; in the repository it is one level higher.
  for (const rel of ['../platforms.json', '../../platforms.json']) {
    try {
      return require(path.join(__dirname, rel));
    } catch (e) {
      if (e.code !== 'MODULE_NOT_FOUND') throw e;
    }
  }
  throw new Error('platforms.json not found');
}

function platformKey(platform = process.platform, arch = process.arch) {
  return `${platform}-${arch}`;
}

function packageName(key) {
  return `@mesh-town/townjump-${key}`;
}

function binaryName(key, platforms = readPlatforms()) {
  return (platforms[key] && platforms[key].exe) || 'townjump';
}

function unsupportedMessage(key, platforms = readPlatforms()) {
  return (
    `townjump: no prebuilt binary for ${key}.\n` +
    `Supported platforms: ${Object.keys(platforms).join(', ')}.\n` +
    `Other ways to install: ${INSTALL_URL}\n`
  );
}

function resolveBinary(key, platforms = readPlatforms()) {
  if (!platforms[key]) return null;
  try {
    const pkg = require.resolve(`${packageName(key)}/package.json`);
    return path.join(path.dirname(pkg), 'bin', binaryName(key, platforms));
  } catch (e) {
    if (e.code !== 'MODULE_NOT_FOUND') throw e;
    return null;
  }
}

function run(argv = process.argv.slice(2)) {
  const key = platformKey();
  const platforms = readPlatforms();
  const bin = resolveBinary(key, platforms);
  if (!bin) {
    const reason = platforms[key]
      ? `the package ${packageName(key)} is not installed (was it skipped with --omit=optional?).\n`
      : '';
    process.stderr.write(reason + unsupportedMessage(key, platforms));
    process.exit(1);
  }
  const child = spawn(bin, argv, { stdio: 'inherit' });
  for (const sig of ['SIGINT', 'SIGTERM']) {
    process.on(sig, () => child.kill(sig));
  }
  child.on('error', (err) => {
    process.stderr.write(`townjump: cannot run ${bin}: ${err.message}\n`);
    process.exit(1);
  });
  child.on('exit', (code, signal) => {
    if (signal) {
      process.removeAllListeners(signal);
      process.kill(process.pid, signal);
      return;
    }
    process.exit(code === null ? 1 : code);
  });
}

module.exports = { platformKey, packageName, binaryName, unsupportedMessage, resolveBinary };

if (require.main === module) run();
