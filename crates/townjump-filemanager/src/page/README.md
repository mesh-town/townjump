# Resources of the `--filemanager` page

Source: livefiles 1.1.0 (`index.js`, `logo.js`), GPLv3; compatible with AGPL-3.0-only via GPLv3 §13.

The files are copied verbatim from the livefiles source and are not edited by hand:

- `page.html` — directory page; placeholders `{{LOGO}}`, `{{TITLE_PATH}}`, `{{ROWS}}`, `{{FORM}}`;
- `form.html` — create form (admin only); placeholders `{{ACTION}}`, `{{OPTIONS}}`;
- `logo.txt` — logo data URI (base64 PNG, no trailing newline);
- `icon-folder.svg`, `icon-file.svg` — table row icons.

Placeholders are substituted by `page/mod.rs` in one pass; values are not parsed again.
