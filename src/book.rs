//! Container access. Everything that reads from a book file goes through here.
//!
//! An EPUB is read from its zip; an AZW3 or MOBI is decoded whole at open by
//! `mobi.rs` into the same href space, so nothing above this module knows which
//! one it has.

use rbook::Epub;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// One navigable line in the table of contents.
#[derive(Clone, Debug, PartialEq)]
pub struct TocEntry {
    pub label: String,
    /// Nesting level, 0 for top-level entries.
    pub depth: usize,
    /// Spine item the entry lands in.
    pub spine: usize,
    /// `id` within that chapter, taken from the href fragment. Plenty of books
    /// hang a whole TOC off one spine file; without this every entry opens on
    /// page 1 of it.
    pub fragment: Option<String>,
    /// Text to land on inside the chapter. Set on search hits, never on
    /// navigation entries — a contents entry knows its element, a hit knows only
    /// its words.
    pub find: Option<String>,
    /// A stored CFI to resolve, for a bookmark or highlight in the marks list.
    pub cfi: Option<String>,
}

enum Source {
    Epub(Mutex<Epub>),
    /// Every file an AZW3/MOBI yields, by href.
    Mobi(HashMap<String, Vec<u8>>),
}

/// An open book. Cheap to clone; the archive is shared.
///
/// Note on the href space: rbook yields absolute, slash-prefixed archive paths
/// (`/OEBPS/ch04.html`) and `read_resource_*` accepts either those or paths
/// relative to the OPF directory. A bare `OEBPS/x.png` is read as OPF-relative
/// and resolves to `/OEBPS/OEBPS/x.png`, so the leading slash must be preserved
/// end to end.
#[derive(Clone)]
pub struct Book {
    inner: Arc<Source>,
    /// Spine hrefs, in reading order, relative to the archive root.
    spine: Arc<Vec<String>>,
    pub title: String,
    /// First `dc:creator`, empty when the book names none.
    pub author: String,
    /// `dc:language`, used to pick hyphenation patterns.
    pub language: String,
    /// Laid out by the publisher page by page — comics, picture books
    /// (`rendition:layout` pre-paginated, or KF8's fixed-layout flag). Read
    /// through the reflowing paginator all the same, so the reader is told.
    pub fixed_layout: bool,
    /// Flattened navigation, in document order. Never empty — see `read_toc`.
    pub toc: Arc<Vec<TocEntry>>,
    /// Cumulative byte length of the spine items, with the total at the end, so
    /// `offsets.len() == spine.len() + 1`. See `Book::progress`.
    offsets: Arc<Vec<usize>>,
}

impl Book {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        if crate::library::is_mobi(path) {
            return Self::open_mobi(path);
        }
        let epub = Epub::open(path).map_err(|e| format!("open epub: {e}"))?;

        let title = epub
            .metadata()
            .title()
            .map(|t| t.value().to_string())
            .unwrap_or_else(|| stem(path));

        let author = epub
            .metadata()
            .creators()
            .next()
            .map(|c| c.value().to_string())
            .unwrap_or_default();

        let language = epub
            .metadata()
            .language()
            .map(|l| l.value().to_string())
            .unwrap_or_default();

        let spine: Vec<String> = epub
            .spine()
            .iter()
            .filter_map(|entry| entry.manifest_entry())
            // A spine item need not be XHTML: it may be an image, or something
            // else with a `fallback` chain that leads to one (EPUB 3.3 §3.4.4).
            // Read the first link of the chain this can show.
            .map(|entry| {
                std::iter::once(entry)
                    .chain(entry.fallbacks())
                    .find(|e| {
                        matches!(e.media_type(), "application/xhtml+xml" | "text/html")
                            || is_image(e.href().as_str())
                    })
                    .unwrap_or(entry)
            })
            .map(|entry| entry.href().as_str().to_string())
            .collect();

        if spine.is_empty() {
            return Err("epub has an empty spine".into());
        }

        let toc = read_toc(&epub, &spine);
        let fixed = epub
            .metadata()
            .by_property("rendition:layout")
            .any(|e| e.value().trim() == "pre-paginated");
        let mut book = Self::assemble(Source::Epub(Mutex::new(epub)), spine, title, author, language, toc);
        book.fixed_layout = fixed;
        Ok(book)
    }

    fn open_mobi(path: &Path) -> Result<Self, String> {
        let m = crate::mobi::open(path)?;
        if m.spine.is_empty() {
            return Err("book has no text".into());
        }
        let toc = m
            .toc
            .iter()
            .filter_map(|(label, depth, href)| {
                let (file, fragment) = match href.split_once('#') {
                    Some((f, frag)) => (f, Some(frag.to_string())),
                    None => (href.as_str(), None),
                };
                Some(TocEntry {
                    label: label.split_whitespace().collect::<Vec<_>>().join(" "),
                    depth: *depth,
                    spine: m.spine.iter().position(|s| s == file)?,
                    fragment,
                    find: None,
                    cfi: None,
                })
            })
            .filter(|e| !e.label.is_empty())
            .collect();
        let title = Some(m.title).filter(|t| !t.trim().is_empty()).unwrap_or_else(|| stem(path));
        let toc = finish_toc(toc, m.spine.len());
        let mut book = Self::assemble(Source::Mobi(m.files), m.spine, title, m.author, m.language, toc);
        book.fixed_layout = m.fixed_layout;
        Ok(book)
    }

    fn assemble(
        source: Source,
        spine: Vec<String>,
        title: String,
        author: String,
        language: String,
        toc: Vec<TocEntry>,
    ) -> Self {
        let mut book = Self {
            inner: Arc::new(source),
            spine: Arc::new(spine),
            title,
            author,
            language,
            fixed_layout: false,
            toc: Arc::new(toc),
            offsets: Arc::new(Vec::new()),
        };

        // Reading progress is weighted by the size of each spine item. Counting
        // spine items instead would call a two-page foreword and a sixty-page
        // chapter the same fraction of a book. The bytes are the markup, not the
        // prose, but they track it closely enough for a percentage and they cost
        // one pass over the (already decompressed) text at open.
        let mut offsets = Vec::with_capacity(book.spine.len() + 1);
        let mut running = 0usize;
        for href in book.spine.iter() {
            offsets.push(running);
            running += book.read_bytes(href).map_or(0, |b| b.len());
        }
        offsets.push(running);
        book.offsets = Arc::new(offsets);
        book
    }

    pub fn chapter_count(&self) -> usize {
        self.spine.len()
    }

    /// How far through the whole book a position is, as 0.0..=1.0.
    ///
    /// `within` is the fraction through the spine item, so the figure moves
    /// smoothly page by page rather than jumping a whole chapter at a time.
    pub fn progress(&self, spine: usize, within: f32) -> f32 {
        let total = self.offsets.last().copied().unwrap_or(0) as f32;
        if total <= 0.0 {
            // Nothing readable to weigh by: fall back to counting spine items.
            let n = self.spine.len().max(1) as f32;
            return ((spine as f32 + within.clamp(0.0, 1.0)) / n).clamp(0.0, 1.0);
        }
        let start = self.offsets.get(spine).copied().unwrap_or(0) as f32;
        let end = self.offsets.get(spine + 1).copied().unwrap_or(0) as f32;
        ((start + (end - start) * within.clamp(0.0, 1.0)) / total).clamp(0.0, 1.0)
    }

    /// The inverse of `progress`: the spine item a fraction of the book falls
    /// in, and how far through that item it is.
    pub fn locate(&self, fraction: f32) -> (usize, f32) {
        let f = fraction.clamp(0.0, 1.0);
        let n = self.spine.len().max(1);
        let total = self.offsets.last().copied().unwrap_or(0) as f32;
        if total <= 0.0 {
            // As `progress` does: count spine items.
            let at = f * n as f32;
            let spine = (at as usize).min(n - 1);
            return (spine, (at - spine as f32).min(1.0));
        }
        let at = f * total;
        let spine = self.offsets[1..].iter().position(|&end| end as f32 > at).unwrap_or(n - 1);
        let (s, e) = (self.offsets[spine] as f32, self.offsets[spine + 1] as f32);
        (spine, if e > s { ((at - s) / (e - s)).clamp(0.0, 1.0) } else { 0.0 })
    }

    /// Href of the nth spine item, archive-root-relative.
    pub fn chapter_href(&self, index: usize) -> Option<&str> {
        self.spine.get(index).map(String::as_str)
    }

    /// Raw XHTML of the nth spine item.
    pub fn chapter_html(&self, index: usize) -> Result<String, String> {
        let href = self
            .chapter_href(index)
            .ok_or_else(|| format!("no spine item {index}"))?
            .to_owned();
        // An image in the spine is a page of its own: give it one to sit on.
        if is_image(&href) {
            let name = href.rsplit('/').next().unwrap_or_default();
            return Ok(format!(r#"<html><body><img src="{name}" alt=""/></body></html>"#));
        }
        self.read_str(&href)
    }

    pub fn read_str(&self, href: &str) -> Result<String, String> {
        match &*self.inner {
            Source::Epub(epub) => {
                let epub = epub.lock().map_err(|_| "book lock poisoned")?;
                epub.read_resource_str(href).map_err(|e| format!("read {href}: {e}"))
            }
            Source::Mobi(_) => String::from_utf8(self.read_bytes(href)?)
                .map_err(|e| format!("read {href}: {e}")),
        }
    }

    pub fn read_bytes(&self, href: &str) -> Result<Vec<u8>, String> {
        match &*self.inner {
            Source::Epub(epub) => {
                let epub = epub.lock().map_err(|_| "book lock poisoned")?;
                epub.read_resource_bytes(href).map_err(|e| format!("read {href}: {e}"))
            }
            Source::Mobi(files) => files.get(href).cloned().ok_or_else(|| format!("read {href}: not found")),
        }
    }
}

/// The raster formats `image` is built with (Cargo.toml).
fn is_image(href: &str) -> bool {
    crate::library::has_ext(Path::new(href), &["png", "jpg", "jpeg", "gif", "webp"])
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Untitled".into())
}

/// Flatten the EPUB 3 `nav` (or the EPUB 2 NCX — rbook falls back on its own)
/// into a list of spine targets.
///
/// Entries that do not resolve to a spine item are dropped: there is nowhere to
/// navigate to, and showing a dead line is worse than showing none. A book with
/// no usable navigation at all falls back to its spine, so the contents key
/// always does something.
fn read_toc(epub: &Epub, spine: &[String]) -> Vec<TocEntry> {
    let mut out = Vec::new();

    if let Some(root) = epub.toc().contents() {
        for entry in root.flatten() {
            let label = entry.label().split_whitespace().collect::<Vec<_>>().join(" ");
            if label.is_empty() {
                continue;
            }
            // `manifest_entry` looks the target up by path, so a fragment does
            // not stop it resolving; the fragment itself is read off the href.
            let Some(href) = entry.manifest_entry().map(|m| m.href().as_str().to_string())
            else {
                continue;
            };
            let Some(index) = spine.iter().position(|s| *s == href) else {
                continue;
            };
            out.push(TocEntry {
                label,
                depth: entry.depth(),
                spine: index,
                fragment: entry
                    .href()
                    .and_then(|h| h.fragment())
                    .map(str::to_string),
                find: None,
                cfi: None,
            });
        }
    }

    finish_toc(out, spine.len())
}

/// Rebase depths and fall back to the spine when there is no navigation.
fn finish_toc(mut out: Vec<TocEntry>, spine_len: usize) -> Vec<TocEntry> {
    // rbook counts depth from the nav root, so the top level is 1 in some books
    // and deeper in others. Indentation is relative; rebase it on the shallowest
    // entry present.
    if let Some(base) = out.iter().map(|e| e.depth).min() {
        for e in &mut out {
            e.depth -= base;
        }
    }

    if out.is_empty() {
        out = (0..spine_len)
            .map(|i| TocEntry {
                label: crate::i18n::tr!("chapter-n", n = i + 1),
                depth: 0,
                spine: i,
                fragment: None,
                find: None,
                cfi: None,
            })
            .collect();
    }
    out
}


#[cfg(test)]
mod tests {
    use super::*;

    /// `locate` is how the go-to bar turns a percentage into a place, so it
    /// has to land where `progress` would say that place is.
    #[test]
    fn locate_inverts_progress() {
        let book = Book {
            inner: Arc::new(Source::Mobi(HashMap::new())),
            spine: Arc::new(vec!["a".into(), "b".into(), "c".into()]),
            title: String::new(),
            author: String::new(),
            language: String::new(),
            fixed_layout: false,
            toc: Arc::new(Vec::new()),
            offsets: Arc::new(vec![0, 100, 400, 500]),
        };
        for (spine, within) in [(0, 0.0), (0, 0.5), (1, 0.0), (1, 0.25), (2, 0.9)] {
            let (s, w) = book.locate(book.progress(spine, within));
            assert_eq!(s, spine, "{spine} {within}");
            assert!((w - within).abs() < 1e-4, "{spine} {within} -> {w}");
        }
        assert_eq!(book.locate(1.0), (2, 1.0));
        assert_eq!(book.locate(-3.0), (0, 0.0));
    }
}
