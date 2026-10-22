// JS: holesail/src/lib/stdout.js:stdout (cli-box 6.0.10, barely-colours); format — cli.md, section 6.
// See also: qr.rs (QR under the server frame), facade.rs (`HolesailInfo`); the filemanager banner has no QR.
use crate::cli::{cli_box::cli_box, qr::qr_small};
use crate::facade::HolesailInfo;
use townjump_filemanager::FilemanagerInfo;

const NOTE: &str = "   NOTE: TREAT PRIVATE CONNECTION STRINGS HOW YOU WOULD TREAT SSH KEY, DO NOT SHARE IT WITH ANYONE YOU DO NOT TRUST    ";
const NOTICE: &str = "   NOTICE: TREAT PUBLIC STRING LIKE YOU WOULD TREAT A DOMAIN NAME ON PUBLIC SERVER, IF THERE IS ANYTHING PRIVATE ON IT, IT IS YOUR RESPONSIBILITY TO PASSWORD PROTECT IT OR USE PRIVATE MODE   \n";

/// `barely-colours`: `\e[<code>m` + text + `\e[0m`.
fn paint(code: u8, text: &str) -> String {
    format!("\u{1b}[{code}m{text}\u{1b}[0m")
}

/// All output of `stdout(...)`: the frame, for a server — a QR; ANSI is always written.
pub fn banner(info: &HolesailInfo) -> String {
    let mut out = cli_box(&box_text(info));
    out.push('\n');
    if info.kind == "server" {
        out.push_str(&qr_small(&info.url));
    }
    out
}

fn box_text(info: &HolesailInfo) -> String {
    [
        heading(info),
        mode(info),
        address(info),
        key_line(info),
        note(info),
    ]
    .join("\n")
}

fn heading(info: &HolesailInfo) -> String {
    let kind = if info.kind == "server" {
        "Server"
    } else {
        "Client"
    };
    title_line(&format!(
        "Holesail {} {kind} Started",
        info.protocol.to_uppercase()
    ))
}

fn title_line(title: &str) -> String {
    format!("{} ⛵️", paint(36, &paint(4, &paint(1, title))))
}

/// `stdout(protocol, 'filemanager', secure, host, port, url, { username, password, role })`: no QR.
pub fn filemanager_banner(info: &HolesailInfo, files: &FilemanagerInfo) -> String {
    let mut out = cli_box(&files_box_text(info, files));
    out.push('\n');
    out
}

fn files_box_text(info: &HolesailInfo, files: &FilemanagerInfo) -> String {
    let login = format!(
        " Username: {} Password: {} Role: {} ",
        files.username.to_js_string(),
        files.password.to_js_string(),
        files.role
    );
    [
        title_line("Holesail Filemanager Started"),
        mode(info),
        address(info),
        login,
        format!("Connected to key: {}", paint(2, &info.url)),
        note(info),
    ]
    .join("\n")
}

fn mode(info: &HolesailInfo) -> String {
    let label = paint(35, "Connection Mode: ");
    if info.secure {
        return label + &paint(32, "Private Connection String");
    }
    label + &paint(33, "Public Connection String")
}

fn address(info: &HolesailInfo) -> String {
    let (host, port) = (info.host.to_js_string(), info.port.to_js_string());
    if info.kind == "server" {
        return paint(35, "Holesail is now listening on ") + &format!("{host}:{port}");
    }
    paint(35, &format!("Access application on http://{host}:{port}/"))
}

fn key_line(info: &HolesailInfo) -> String {
    let label = if info.kind == "server" {
        "Connect with key: "
    } else {
        "Connected to key: "
    };
    format!("{label}{}", paint(2, &info.url))
}

fn note(info: &HolesailInfo) -> String {
    if info.secure {
        return paint(2, NOTE);
    }
    paint(2, NOTICE)
}

#[cfg(test)]
#[path = "banner_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "banner_fm_tests.rs"]
mod fm_tests;
