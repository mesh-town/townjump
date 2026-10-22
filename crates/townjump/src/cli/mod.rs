// JS: holesail/src/bin/holesail.mjs and holesail/src/lib/*.js. Only declarations and re-exports here.
// What each file is for:
//   run.rs    — `run` and `CliIo`: dispatcher, server, client and `--lookup` modes, exit codes
//   argv.rs   — minimist 1.2.8 port: `Vec<String>` → `JsValue`
//   checks.rs — `validateInput`: input checks, code 2
//   help.rs   — help (`printHelp`) and version
//   banner.rs — banner after `ready` (cli-box frame, colors)
//   cli_box.rs — cli-box frame and barely-colours colors
//   fd_limit.rs — soft RLIMIT_NOFILE up to the limit, like Node
//   qr.rs     — connection string QR (qrcode-terminal, half blocks)
pub mod argv;
pub mod banner;
pub mod checks;
pub mod cli_box;
pub mod fd_limit;
pub mod help;
pub mod qr;
pub mod run;

pub use run::{CliIo, run};
