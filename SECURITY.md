# Security policy

## Supported versions

Only the latest released version receives security fixes.

## Reporting a vulnerability

Please do not open a public issue for a security problem. Use GitHub private vulnerability reporting for this
repository (the "Security" tab, "Report a vulnerability") with a description, the affected version and steps to reproduce.
You should get an answer within a week; the fix and the disclosure date are agreed with the reporter.

## Scope notes

- A Holesail connection key is a capability: whoever has a private key can reach the shared port. Keep private keys
  secret, and protect public-mode services with their own authentication.
- Problems in the original JavaScript Holesail or in the HyperDHT protocol should be reported to their projects;
  this repository covers the Rust port and the patched `fork/peeroxide`.
