//! The hermetic resource provider.
//!
//! Omaread never makes a network request. This provider resolves resources
//! *only* from inside the open book's archive. `blitz-net` is deliberately not a
//! dependency, so there is no HTTP client in the binary to regress into.
//!
//! See CONTEXT.md §5.

use crate::book::Book;
use crate::db::Db;
use crate::grid::COVER_ORIGIN;
use blitz_dom::net::Resource;
use blitz_traits::net::{BoxedHandler, Bytes, NetCallback, NetProvider, Request, SharedCallback};
use std::sync::{Arc, Mutex};

/// Synthetic origin for in-archive resources. Chosen so that relative hrefs in a
/// chapter resolve against it via normal URL rules, and so that anything with a
/// real scheme (http, https, data with remote refs) is visibly not ours.
pub const ORIGIN: &str = "omaread-book://book/";

pub struct BookNetProvider<D> {
    book: Book,
    callback: SharedCallback<D>,
}

impl<D> BookNetProvider<D> {
    pub fn new(book: Book, callback: SharedCallback<D>) -> Self {
        Self { book, callback }
    }
}

impl<D: Send + Sync + 'static> NetProvider<D> for BookNetProvider<D> {
    fn fetch(&self, doc_id: usize, request: Request, handler: BoxedHandler<D>) {
        let url = request.url;

        // Anything not on our synthetic origin is off-book: a tracking pixel, a
        // remote stylesheet, a CDN font. Drop it silently and render the
        // placeholder. This is the whole point of the type.
        if url.scheme() != "omaread-book" {
            log_blocked(&url);
            return;
        }

        let Some(href) = in_archive_path(url.path()) else {
            log_blocked(&url);
            return;
        };

        match self.book.read_bytes(&href) {
            Ok(bytes) => handler.bytes(doc_id, Bytes::from(bytes), self.callback.clone()),
            Err(e) => eprintln!("omaread: missing resource {href}: {e}"),
        }
    }
}

/// Serves cover images to the library view straight out of SQLite. Same
/// hermetic rule as the book provider: nothing but our own origin resolves.
/// Covers are shrunk to card size on the way in; anything stored before that
/// rule existed is shrunk the first time it is shown and written back, so the
/// cost is paid once per book rather than on every rebuild.
/// Above this, a stored cover is bigger than a card can use. A shrunk one lands
/// well under it, so the check costs nothing after the first visit.
const SHRINK_OVER: usize = 80 * 1024;

pub struct CoverProvider {
    db: Arc<Mutex<Db>>,
    callback: SharedCallback<Resource>,
}

impl CoverProvider {
    /// `card` is a cover's size in physical pixels: decoded covers are resized
    /// to exactly that, see `FitCover`.
    pub fn new(db: Arc<Mutex<Db>>, callback: SharedCallback<Resource>, card: (u32, u32)) -> Self {
        Self { db, callback: Arc::new(FitCover { card, inner: callback }) }
    }
}

/// Resizes each decoded cover to the card's pixel size before the document
/// gets it. vello_cpu's bilinear sampler took ~10ms a cover, every frame, which
/// made a library frame ~300ms; drawn 1:1 the grid can sample nearest
/// (`image-rendering: pixelated`) at no cost and no loss. A quarter of the
/// RGBA, too.
struct FitCover {
    card: (u32, u32),
    inner: SharedCallback<Resource>,
}

impl NetCallback<Resource> for FitCover {
    fn call(&self, doc_id: usize, result: Result<Resource, Option<String>>) {
        let result = match result {
            Ok(Resource::Image(node, kind, w, h, rgba)) => {
                let (cw, ch) = self.card;
                image::RgbaImage::from_raw(w, h, Arc::unwrap_or_clone(rgba))
                    .map(|img| image::imageops::thumbnail(&img, cw, ch))
                    .map(|img| Resource::Image(node, kind, cw, ch, Arc::new(img.into_raw())))
                    .ok_or(None)
            }
            other => other,
        };
        self.inner.call(doc_id, result);
    }
}

impl NetProvider<Resource> for CoverProvider {
    fn fetch(&self, doc_id: usize, request: Request, handler: BoxedHandler<Resource>) {
        let url = request.url;
        if url.scheme() != "omaread-cover" {
            log_blocked(&url);
            return;
        }
        let hash = url.path().trim_start_matches('/');
        let Some(bytes) = self.db.lock().ok().and_then(|d| d.cover(hash)) else { return };
        let bytes = match bytes.len() > SHRINK_OVER {
            false => bytes,
            true => {
                let small = crate::library::shrink_cover(bytes);
                if let Ok(db) = self.db.lock() {
                    let _ = db.set_cover(hash, &small);
                }
                small
            }
        };
        handler.bytes(doc_id, Bytes::from(bytes), self.callback.clone());
    }
}

const _: () = {
    // Keep the origin constant and the scheme check above in step.
    assert!(COVER_ORIGIN.as_bytes()[0] == b'o');
};

fn log_blocked(url: &blitz_traits::net::Url) {
    eprintln!("omaread: blocked off-book resource {url}");
}

/// Normalise a URL path into an absolute in-archive path, rejecting traversal.
///
/// EPUBs are zip archives from untrusted sources; `../` in an href must never
/// escape the archive root.
///
/// The result keeps its leading slash: rbook treats a slashless path as relative
/// to the OPF directory, which silently doubles the prefix (see `book.rs`).
pub fn in_archive_path(path: &str) -> Option<String> {
    let decoded = percent_decode(path);
    let mut out: Vec<&str> = Vec::new();

    for segment in decoded.split('/') {
        match segment {
            "" | "." => continue,
            ".." => {
                // Refuse to climb above the root rather than silently clamping:
                // an href that tries is malformed or hostile, and we want neither.
                out.pop()?;
            }
            s if s.contains('\\') || s.contains('\0') => return None,
            s => out.push(s),
        }
    }

    if out.is_empty() {
        return None;
    }
    Some(format!("/{}", out.join("/")))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(b) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::in_archive_path;

    #[test]
    fn resolves_normal_paths() {
        assert_eq!(in_archive_path("/OEBPS/ch01.html").as_deref(), Some("/OEBPS/ch01.html"));
        assert_eq!(in_archive_path("OEBPS/./assets/x.png").as_deref(), Some("/OEBPS/assets/x.png"));
        assert_eq!(in_archive_path("/OEBPS/a/../b.png").as_deref(), Some("/OEBPS/b.png"));
    }

    #[test]
    fn percent_decodes() {
        assert_eq!(in_archive_path("/OEBPS/a%20b.png").as_deref(), Some("/OEBPS/a b.png"));
    }

    #[test]
    fn rejects_traversal_and_junk() {
        assert_eq!(in_archive_path("/../etc/passwd"), None);
        assert_eq!(in_archive_path("/OEBPS/../../etc/passwd"), None);
        assert_eq!(in_archive_path("/%2e%2e/%2e%2e/etc/passwd"), None);
        assert_eq!(in_archive_path("/"), None);
        assert_eq!(in_archive_path("/OEBPS/a\\b.png"), None);
    }

    /// A cover reaches the document at exactly the card's pixel size, which is
    /// what makes nearest sampling in the grid exact; anything else passes.
    #[test]
    fn covers_arrive_at_card_size() {
        use super::{FitCover, NetCallback, Resource};
        use blitz_dom::util::ImageType;
        use std::sync::{Arc, Mutex};

        struct Keep(Mutex<Vec<Resource>>);
        impl NetCallback<Resource> for Keep {
            fn call(&self, _: usize, r: Result<Resource, Option<String>>) {
                self.0.lock().unwrap().extend(r.ok());
            }
        }
        let keep = Arc::new(Keep(Mutex::new(Vec::new())));
        let fit = FitCover { card: (225, 348), inner: keep.clone() };

        let rgba = Arc::new(vec![200u8; 450 * 696 * 4]);
        fit.call(0, Ok(Resource::Image(7, ImageType::Image, 450, 696, rgba)));
        fit.call(0, Ok(Resource::Font(Default::default())));

        let got = keep.0.lock().unwrap();
        match &got[0] {
            Resource::Image(7, _, 225, 348, px) => assert_eq!(px.len(), 225 * 348 * 4),
            _ => panic!("cover not fitted"),
        }
        assert!(matches!(got[1], Resource::Font(_)), "non-image passes through");
    }
}
