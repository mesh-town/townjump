// JS: livefiles/index.js:getContentType (extension table); unknown extension is `|| 'application/octet-stream'` in serveFile.
/// The `getContentType` table (lowercase extension with a dot → type).
const TABLE: &[(&str, &str)] = &[
    (".html", "text/html"),
    (".css", "text/css"),
    (".js", "application/javascript"),
    (".json", "application/json"),
    (".xml", "application/xml"),
    (".txt", "text/plain"),
    (".md", "text/markdown"),
    (".py", "text/x-python"),
    (".c", "text/x-c"),
    (".h", "text/x-c"),
    (".cpp", "text/x-c++src"),
    (".sh", "text/x-shellscript"),
    (".pdf", "application/pdf"),
    (".jpg", "image/jpeg"),
    (".jpeg", "image/jpeg"),
    (".png", "image/png"),
    (".gif", "image/gif"),
    (".ico", "image/x-icon"),
    (".svg", "image/svg+xml"),
    (".mp3", "audio/mpeg"),
    (".bmp", "image/bmp"),
    (".webp", "image/webp"),
    (".zip", "application/zip"),
    (".rar", "application/x-rar-compressed"),
    (".tar", "application/x-tar"),
    (".gz", "application/x-gzip"),
    (".bz2", "application/x-bzip2"),
    (".7z", "application/x-7z-compressed"),
    (".wav", "audio/x-wav"),
    (".ogg", "audio/ogg"),
    (".flac", "audio/x-flac"),
    (".m4a", "audio/x-m4a"),
    (".mp4", "video/mp4"),
    (".webm", "video/webm"),
    (".ogv", "video/ogg"),
    (".avi", "video/x-msvideo"),
    (".mov", "video/quicktime"),
    (".wmv", "video/x-ms-wmv"),
    (".flv", "video/x-flv"),
    (".mkv", "video/x-matroska"),
    (".exe", "application/x-msdownload"),
];

pub(crate) fn content_type(ext_lower: &str) -> &'static str {
    TABLE
        .iter()
        .find(|(ext, _)| *ext == ext_lower)
        .map_or("application/octet-stream", |(_, mime)| mime)
}

#[cfg(test)]
#[path = "mime_tests.rs"]
mod tests;
