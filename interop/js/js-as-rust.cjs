#!/usr/bin/env node
// A fake "Rust binary" for checking the harness itself: the same argv, the same TOWNJUMP_BOOTSTRAP, but the JS CLI inside.
// `node interop/run.mjs --rust-bin interop/js/js-as-rust.cjs` must give all ok on all four pairs:
// this checks the Rust-side launch path (the TOWNJUMP_RUN_WRAPPER wrapper, argv, env) and the whole matrix before Rust exists.
const { pathToFileURL } = require('node:url')
const { CLI } = require('./nm.cjs')

require('./bootstrap-env.cjs')
import(pathToFileURL(CLI).href)
