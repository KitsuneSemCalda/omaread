//! File and folder pickers, through xdg-desktop-portal (CONTEXT.md §12). The
//! dialog is the desktop's own, and under a sandbox the portal is also what
//! grants access to whatever was picked. Showing a book in its folder and
//! trashing one go the same way.
//!
//! Spoken to over D-Bus directly: zbus is already in the tree for AccessKit,
//! and `ashpd` or `rfd` would bring an async runtime to wrap one method call
//! and one signal.

use std::collections::HashMap;
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::{Fd, OwnedValue, Value};

/// Ask for a book, or with `folder` a folder. Blocks until the dialog closes,
/// so call it off the window's thread. `Ok(None)` is a cancel; an error means
/// there was no portal to ask.
pub fn pick(folder: bool, title: &str) -> Result<Option<PathBuf>, String> {
    let err = |e: zbus::Error| format!("file portal: {e}");
    let conn = Connection::session().map_err(err)?;

    // Listen for the answer before asking: a quick portal can answer before
    // the call returns the request's path. That path is predictable from the
    // bus name and a token of our choosing.
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let token = format!("omaread{}_{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
    let sender = conn.unique_name().ok_or("file portal: no bus name")?;
    let sender = sender.as_str().trim_start_matches(':').replace('.', "_");
    let request = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.freedesktop.portal.Request")
        .and_then(|r| r.member("Response"))
        .and_then(|r| r.path(request.as_str()))
        .map_err(err)?
        .build();
    let mut answers = MessageIterator::for_match_rule(rule, &conn, Some(1)).map_err(err)?;

    let mut options: HashMap<&str, Value> = HashMap::new();
    options.insert("handle_token", token.as_str().into());
    options.insert("directory", folder.into());
    if !folder {
        let books = vec![(0u32, "*.epub"), (0, "*.azw3"), (0, "*.mobi"), (0, "*.azw")];
        options.insert("filters", vec![("EPUB, AZW3, MOBI", books)].into());
    }
    conn.call_method(
        DEST,
        OBJECT,
        Some("org.freedesktop.portal.FileChooser"),
        "OpenFile",
        &("", title, options),
    )
    .map_err(err)?;

    let answer = answers.next().ok_or("file portal: closed without answering")?.map_err(err)?;
    let (code, results): (u32, HashMap<String, OwnedValue>) = answer.body().deserialize().map_err(err)?;
    // 1 is a cancel and 2 the portal giving up; either way, nothing was picked.
    if code != 0 {
        return Ok(None);
    }
    let uris = results
        .get("uris")
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| Vec::<String>::try_from(v).ok())
        .unwrap_or_default();
    Ok(uris.first().and_then(|u| file_path(u)))
}

/// Open the file manager on the folder holding `path`, with it selected.
pub fn reveal(path: &Path) -> Result<(), String> {
    let (conn, file) = desktop(path)?;
    let options: HashMap<&str, Value> = HashMap::new();
    conn.call_method(DEST, OBJECT, Some("org.freedesktop.portal.OpenURI"), "OpenDirectory", &("", Fd::from(file.as_fd()), options))
        .map(drop)
        .map_err(|e| format!("open-uri portal: {e}"))
}

/// Move `path` to the desktop's trash, from where it can be put back.
pub fn trash(path: &Path) -> Result<(), String> {
    let (conn, file) = desktop(path)?;
    let reply = conn
        .call_method(DEST, OBJECT, Some("org.freedesktop.portal.Trash"), "TrashFile", &(Fd::from(file.as_fd()),))
        .map_err(|e| format!("trash portal: {e}"))?;
    // The portal answers 1 for done, 0 for failed.
    match reply.body().deserialize::<u32>() {
        Ok(1) => Ok(()),
        _ => Err(format!("trash portal: could not trash {}", path.display())),
    }
}

const DEST: Option<&str> = Some("org.freedesktop.portal.Desktop");
const OBJECT: &str = "/org/freedesktop/portal/desktop";

/// The session bus, and `path` open: a portal takes a file as a descriptor,
/// not a path.
fn desktop(path: &Path) -> Result<(Connection, std::fs::File), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let conn = Connection::session().map_err(|e| format!("portal: {e}"))?;
    Ok((conn, file))
}

/// A `file://` URI as a path: percent-escapes decoded to the raw bytes they
/// stand for, which on Linux need not be UTF-8.
fn file_path(uri: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let mut bytes = uri.strip_prefix("file://")?.bytes();
    let mut out = Vec::new();
    while let Some(b) = bytes.next() {
        out.push(match b {
            b'%' => u8::from_str_radix(std::str::from_utf8(&[bytes.next()?, bytes.next()?]).ok()?, 16).ok()?,
            b => b,
        });
    }
    Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_become_paths() {
        assert_eq!(file_path("file:///home/a/My%20Books/caf%C3%A9.epub"), Some("/home/a/My Books/café.epub".into()));
        assert_eq!(file_path("file:///x/100%25.epub"), Some("/x/100%.epub".into()));
        assert_eq!(file_path("https://x/a.epub"), None);
        assert_eq!(file_path("file:///x/bad%2"), None);
    }
}
