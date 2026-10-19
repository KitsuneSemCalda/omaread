//! Omaread — a proper EPUB reader for Omarchy.
//!
//! Phase 1: one chapter on screen. See CONTEXT.md for the design record.

mod a11y;
mod book;
mod chapter;
mod net;
mod cfi;
mod check;
mod db;
mod grid;
mod hud;
mod hyphen;
mod i18n;
mod library;
mod mobi;
mod paginate;
mod paint;
mod portal;
mod search;
mod shot;
mod style;
mod toc;

use anyrender::WindowRenderer;
use anyrender_vello_cpu::VelloCpuWindowRenderer as VelloWindowRenderer;
use peniko::Color;
use blitz_dom::net::Resource;
use blitz_traits::net::{NetCallback, SharedCallback};
use book::Book;
use db::{BookRow, Db, Sort};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use chapter::Chapter;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};
use style::{Chrome, GUTTER_EM, MEASURE_EM, PAGE_MARGIN_EM, ReadingStyle, Theme};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

/// A highlighter is opaque enough to see and transparent enough to read
/// through; the selection is the same idea in the chrome's accent
/// (`SELECTION_ALPHA`).
pub const HIGHLIGHT: Color = Color::from_rgba8(0xff, 0xd6, 0x2e, 0x66);
const SELECTION_ALPHA: f32 = 0.25;
/// The underline that says a highlight carries a note: the highlighter's own
/// amber, opaque, so it reads on every theme.
const NOTED: Color = Color::from_rgb8(0xe0, 0x9a, 0x00);
/// The card under the pointer, outlined in the text colour at this alpha —
/// Omarchy's image picker marks its unselected slices the same way, and the
/// selected card gets the accent, as the picker's selected slice does.
const CARD_HOVER_ALPHA: f32 = 0.28;
/// How far outside the card its ring is drawn, in CSS pixels. On the card's
/// edge the ring touched the title and author, which start there; outside it,
/// cover and text both get air. Well inside half of `grid::GAP`, so
/// neighbouring rings never meet.
const CARD_RING: f32 = 8.0;
/// A HUD control or menu row under the pointer: Omarchy's hover-cursor state,
/// a text-colour fill and a fainter hairline (`[controls]` in its shell.toml).
const HOVER_FILL_ALPHA: f32 = 0.08;
const HOVER_BORDER_ALPHA: f32 = 0.25;

#[derive(Clone, Copy)]
struct Slide {
    from: usize,
    forward: bool,
    at: Instant,
}

/// How long a page takes to slide out of the way. Long enough to see which way
/// the page went, short enough that holding the key still turns pages quickly.
const SLIDE: Duration = Duration::from_millis(160);
/// Wake-up interval while a page is sliding. 60fps: a 160ms slide is ten
/// frames, and asking for them twice that fast only queues work the compositor
/// has not been asked for yet.
const FRAME: Duration = Duration::from_millis(16);

/// How long to wait before re-trying a frame the compositor refused. Grows with
/// each consecutive failure.
const RETRY: Duration = Duration::from_millis(500);

/// How long the reading HUD lingers after the pointer stops moving.
const HUD_LINGER: Duration = Duration::from_millis(2200);

/// Below this window width, a second column would squeeze the measure into
/// something unreadable, so two-column mode silently falls back to one.
const TWO_COLUMN_MIN_EM: f32 = 2.0 * (MEASURE_EM + 2.0 * GUTTER_EM);

/// Whether two columns actually fit, and are wanted.
///
/// Below the minimum the measure would squeeze into something unreadable, so it
/// silently falls back to one (CONTEXT.md §3). The library and the contents are
/// single documents laid out at window width, so they are always one column.
fn columns_for(requested: usize, css_width: f32, font_px: f32, reading: bool) -> usize {
    let fits = css_width >= TWO_COLUMN_MIN_EM * font_px;
    match requested >= 2 && fits && reading {
        true => 2,
        false => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::step_row;

    #[test]
    fn menu_rows_step_and_wrap() {
        assert_eq!(step_row(None, 1, 8), 0, "down from nothing is the top");
        assert_eq!(step_row(None, -1, 8), 7, "up from nothing is the bottom");
        assert_eq!(step_row(Some(2), 1, 8), 3);
        assert_eq!(step_row(Some(0), -1, 8), 7, "up from the top wraps");
        assert_eq!(step_row(Some(7), 1, 8), 0, "down from the bottom wraps");
        assert_eq!(step_row(Some(5), isize::MIN, 8), 0);
        assert_eq!(step_row(None, isize::MAX, 8), 7);
    }

    use super::columns_for;
    use crate::style::{GUTTER_EM, MEASURE_EM};

    /// A taller window shows more cards, and the covers were chosen for the old
    /// height — so the newly revealed row came up bare. Resizing re-paginates the
    /// grid in place; it has to notice the visible set changed.
    /// The CSS and `grid::per_row` must agree about how many cards fit, or arrow
    /// keys step onto columns that are not there. Asserted against a real
    /// layout, not against the arithmetic on its own.
    #[test]
    fn the_laid_out_row_holds_what_per_row_says() {
        use crate::db::BookRow;

        let rows: Vec<BookRow> = (0..40)
            .map(|i| BookRow { hash: format!("h{i}"), title: format!("L{i}"), ..Default::default() })
            .collect();
        let ua = crate::grid::stylesheet(&crate::style::Theme::White.chrome());

        for width in [900u32, 1200, 1517, 2000, 3840] {
            let doc = crate::chapter::layout_document(
                crate::grid::html(&rows, "", crate::db::Sort::Recent, &[], None, 0..rows.len()),
                ua.clone(),
                None,
                crate::chapter::viewport(width, 2400, 1.0, false),
                2300.0,
            )
            .expect("the grid must lay out");

            let tops = crate::chapter::indexed_tops(doc.dom());
            let first = tops.first().map(|&(_, y)| y).unwrap_or(0.0);
            let in_row = tops.iter().filter(|&&(_, y)| (y - first).abs() < 1.0).count();

            assert_eq!(
                in_row,
                crate::grid::per_row(width as f32),
                "at {width}px the layout put {in_row} in a row"
            );
        }
    }

    /// PageDown left the selection behind, and the rebuild then snapped the
    /// page back to it: the library could not be paged at all.
    #[test]
    fn the_library_selection_rides_a_page_turn() {
        assert_eq!(super::ride_along(7, 0..15, 15..30), 22, "same place, next page");
        assert_eq!(super::ride_along(22, 15..30, 0..15), 7, "and back");
        assert_eq!(super::ride_along(12, 0..15, 15..18), 17, "a short last page");
        assert_eq!(super::ride_along(16, 0..15, 15..30), 16, "already there");
    }

    /// Calibre's notes, from Postman's *Divertirse hasta morir*: the id is on
    /// the `<small>` marker, not the note. A marker opening its paragraph stands
    /// for the note; the back-link's target, closing a body paragraph, is a
    /// place and must not pop the paragraph up as a note.
    #[test]
    fn a_marker_opening_a_paragraph_stands_for_the_note() {
        let body = r#"<p>al decir: «El daguerrotipo no es un mero instrumento».<sup><small id="ref">
            <a href="n.html#note"><span>[4]</span></a></small></sup></p>
            <p><sup><small id="note"><span>[4]</span></small></sup> Sontag, p. 165. <a href="c.html#ref">&lt;&lt;</a></p>"#;
        let doc = crate::chapter::layout_fragment(
            body,
            &crate::style::ReadingStyle::default(),
            crate::chapter::viewport(900, 700, 1.0, false),
        );
        let dom: &blitz_dom::BaseDocument = &doc;
        let note = super::find_by_attr(dom, 0, "id", "note").unwrap();
        let opened = super::note_of_marker(dom, note);
        assert_eq!(crate::chapter::tag_of(dom, opened).as_deref(), Some("p"));
        assert!(crate::chapter::text_of(dom, opened).contains("Sontag, p. 165"));

        let back = super::find_by_attr(dom, 0, "id", "ref").unwrap();
        assert_eq!(super::note_of_marker(dom, back), back, "a place, not a note");
    }

    /// A superscript note reference is a ten-pixel target inside a sentence.
    /// Landing beside it has to count, or footnotes are unopenable.
    #[test]
    fn a_near_miss_still_finds_the_note_reference() {
        let html = r##"<html><body><p>Una frase larga que ocupa una linea entera y
            termina con una nota<sup><a href="#n1">2</a></sup> y sigue despues
            con mas palabras para llenar la medida.</p>
            <p id="n1">2 La nota, que es corta.</p></body></html>"##;
        let doc = crate::chapter::layout_document(
            html.to_string(),
            crate::style::ReadingStyle::default().stylesheet(),
            None,
            crate::chapter::viewport(900, 700, 1.0, false),
            600.0,
        )
        .expect("the page must lay out");

        // Sweep the paragraph for the one point that is exactly on the glyph.
        let par = crate::chapter::node_containing_text(doc.dom(), "termina con una nota")
            .expect("no paragraph");
        let (px, py, pw, ph) = crate::chapter::node_rect(doc.dom(), par).expect("no box");
        let mut on: Option<(f32, f32)> = None;
        let mut y = py;
        while y < py + ph && on.is_none() {
            let mut x = px;
            while x < px + pw {
                if doc
                    .doc
                    .hit(x, y)
                    .and_then(|h| super::ancestor_attr(doc.dom(), h.node_id, "href"))
                    .is_some()
                {
                    on = Some((x, y));
                    break;
                }
                x += 1.0;
            }
            y += 1.0;
        }
        let (hx, hy) = on.expect("the link is not hittable anywhere");

        // Six pixels off is a miss for the engine and a hit for a reader.
        assert_eq!(super::href_at(&doc.doc, hx, hy).as_deref(), Some("#n1"));
        assert_eq!(super::href_at(&doc.doc, hx - 6.0, hy).as_deref(), Some("#n1"));
        assert_eq!(super::href_at(&doc.doc, hx, hy + 6.0).as_deref(), Some("#n1"));
        // Far away is still a miss, or every click would follow something.
        assert_eq!(super::href_at(&doc.doc, px, py + ph - 1.0), None);
    }

    /// A note is shown where you are; a section is somewhere to go.
    #[test]
    fn a_short_block_is_a_note_and_a_section_is_a_place() {
        assert!(super::is_note("p", "1. Ibíd., p. 233."), "a bibliography entry");
        assert!(super::is_note("aside", "A footnote, as EPUB 3 would write one."));
        assert!(super::is_note("li", "Postman, N., Amusing Ourselves to Death, 1985."));

        // A heading is where a section starts: that is a page to turn to.
        assert!(!super::is_note("h2", "Capítulo 4"));
        assert!(!super::is_note("section", "Capítulo 4"));
        // So is a whole chapter that happens to sit in a <div>.
        assert!(!super::is_note("div", &"palabra ".repeat(1000)));
        // A long footnote is still a note; whether it fits is the popup's call.
        assert!(super::is_note("aside", &"palabra ".repeat(200)));
        // And an empty target says nothing worth a popup.
        assert!(!super::is_note("p", "   "));
    }

    /// A short note sits a quarter down the window; a long one moves up to fit,
    /// and one taller than the window overflows it, which is what sends the
    /// reader to its page instead.
    #[test]
    fn a_long_note_moves_up_until_it_cannot_fit() {
        let note_box = |words: usize| {
            let doc = crate::chapter::layout_document(
                crate::hud::note_html(&"palabra ".repeat(words)),
                crate::hud::note_stylesheet(&crate::style::ReadingStyle::default().chrome()),
                None,
                crate::chapter::viewport(900, 800, 1.0, false),
                800.0,
            )
            .expect("the note must lay out");
            let n = super::find_by_attr(doc.dom(), 0, "class", "note").expect("no note");
            let (_, y, _, h) = crate::chapter::node_rect(doc.dom(), n).expect("no box");
            (y, y + h)
        };
        let (top, _) = note_box(10);
        assert!((top - 208.0).abs() < 1.0, "a short note sits at 26vh, not {top}");
        let (top, bottom) = note_box(250);
        assert!(top < 208.0 && bottom <= 800.0, "a long one moves up and fits: {top}..{bottom}");
        let (_, bottom) = note_box(3000);
        assert!(bottom > 800.0, "too long to fit runs off the window");
    }

    /// Highlights land under their chapter in reading order, notes under the
    /// words they are about, and a bookmark only when it says something.
    #[test]
    fn marks_export_as_markdown_by_chapter() {
        let entry = |label: &str, spine| crate::book::TocEntry {
            label: label.into(),
            depth: 0,
            spine,
            fragment: None,
            find: None,
            cfi: None,
        };
        let mark = |cfi: &str, length, text: &str, note: &str| crate::db::Mark {
            cfi: cfi.into(),
            length,
            text: text.into(),
            note: note.into(),
            ..Default::default()
        };
        let toc = [entry("Uno", 1), entry("Dos", 3)];
        let marks = [
            mark("epubcfi(/6/4!/4/2:0)", 5, "Hola\nmundo", ""),
            mark("epubcfi(/6/4!/4/6)", 0, "", ""),
            mark("epubcfi(/6/6!/4/2:3)", 3, "otra", "Mi nota"),
            mark("epubcfi(/6/8!/4/2)", 0, "", "Volver aquí"),
        ];
        assert_eq!(
            super::marks_markdown("Libro", "Autora", &toc, &marks),
            "# Libro\n\n*Autora*\n\n## Uno\n\n> Hola\n> mundo\n\n> otra\n\nMi nota\n\n## Dos\n\nVolver aquí\n"
        );
    }

    /// The page you left goes the way you turned, and the one arriving comes
    /// from the other side. Backwards is the mirror, or the gesture lies.
    #[test]
    fn a_turn_slides_the_way_it_was_turned() {
        let w = 900.0;
        assert_eq!(super::slide_offsets(true, 0.0, w), (0.0, w), "the new page waits offstage right");
        assert_eq!(super::slide_offsets(true, 1.0, w), (-w, 0.0), "and lands where the old one was");
        assert_eq!(super::slide_offsets(false, 0.0, w), (0.0, -w), "turning back, it waits at the left");
        assert_eq!(super::slide_offsets(false, 1.0, w), (w, 0.0));
        // Halfway, the two are adjacent: no gap, no overlap.
        let (out, into) = super::slide_offsets(true, 0.5, w);
        assert!((into - out - w).abs() < 0.01, "{out} and {into} must abut");
    }

    #[test]
    fn a_taller_window_reveals_cards_the_covers_did_not_cover() {
        use crate::db::BookRow;

        let rows: Vec<BookRow> = (0..40)
            .map(|i| BookRow {
                hash: format!("h{i}"),
                title: format!("Libro {i}"),
                author: "Autor".into(),
                has_cover: true,
                ..Default::default()
            })
            .collect();
        let ua = crate::grid::stylesheet(&crate::style::Theme::White.chrome());
        let markup = crate::grid::html(
            &rows,
            "",
            crate::db::Sort::Recent,
            &[],
            None,
            0..rows.len(),
        );

        // A window with room for one row, then one with room for more.
        let short = 620.0_f32;
        let tall = 1400.0_f32;
        let mut doc = crate::chapter::layout_document(
            markup,
            ua,
            None,
            crate::chapter::viewport(1500, short as u32, 1.0, false),
            short - 100.0,
        )
        .expect("the grid must lay out");

        let before = super::visible_cards(&doc, 0);
        crate::chapter::relayout(
            &mut doc,
            crate::chapter::viewport(1500, tall as u32, 1.0, false),
            tall - 100.0,
        );
        let after = super::visible_cards(&doc, 0);

        assert!(
            after.end > before.end,
            "a taller window should show more cards: {before:?} -> {after:?}"
        );
    }

    /// Rendering a control is not the same as being able to click it: the HUD is
    /// hit-tested at window coordinates, so every button has to actually occupy
    /// some. Scans across both bars rather than guessing exact text widths.
    #[test]
    fn every_hud_control_is_hittable() {
        use std::collections::HashSet;

        let (w, h) = (1200u32, 900u32);
        let doc = crate::chapter::layout_document(
            crate::hud::html("Un libro", "page 8 of 19", 2, false, h as f32, Some("Back to page 7"), false, None),
            crate::hud::stylesheet(&crate::style::Theme::White.chrome()),
            None,
            crate::chapter::viewport(w, h, 1.0, false),
            h as f32,
        )
        .expect("the HUD must lay out");

        // Middle of the top bar, and of the bottom one.
        let mut found: HashSet<String> = HashSet::new();
        for y in [33.0_f32, h as f32 - 33.0] {
            for x in (0..w).step_by(4) {
                if let Some(hit) = doc.doc.hit(x as f32, y) {
                    if let Some(what) =
                        crate::ancestor_attr(doc.dom(), hit.node_id, "data-hud")
                    {
                        found.insert(what);
                    }
                }
            }
        }

        for want in ["bookmark", "contents", "highlight", "smaller", "bigger", "columns", "readout"]
        {
            assert!(found.contains(want), "{want} is drawn but not clickable: {found:?}");
        }
    }

    /// The go-to bar is a click target across most of the foot, and the window
    /// reads a fraction off the track's box, so the box has to be wide.
    #[test]
    fn the_go_to_track_is_hittable_across_the_foot() {
        let (w, h) = (1200u32, 900u32);
        let doc = crate::chapter::layout_document(
            crate::hud::html("Un libro", "5%", 1, false, h as f32, None, false, Some((0.5, "page 3 of 9 · Uno"))),
            crate::hud::stylesheet(&crate::style::Theme::White.chrome()),
            None,
            crate::chapter::viewport(w, h, 1.0, false),
            h as f32,
        )
        .expect("the HUD must lay out");
        let track = crate::find_by_attr(doc.dom(), 0, "data-hud", "scrub").expect("a track");
        let (x, y, tw, th) = crate::chapter::node_rect(doc.dom(), track).expect("a box");
        assert!(tw > w as f32 / 3.0, "the track is {tw}px of {w}");
        let hit = doc.doc.hit(x + tw / 2.0, y + th / 2.0).expect("a hit");
        assert_eq!(crate::ancestor_attr(doc.dom(), hit.node_id, "data-hud").as_deref(), Some("scrub"));
        assert!(crate::find_by_attr(doc.dom(), 0, "data-hud", "library").is_none(), "it takes the bar's place");
    }

    /// The context menu is placed by the window, so its rows have to be where
    /// the window put them — and fit the height `open_context_menu` estimates.
    #[test]
    fn the_context_menu_rows_are_hittable_where_it_was_opened() {
        let (left, top) = (300.0_f32, 200.0_f32);
        let doc = crate::chapter::layout_document(
            crate::hud::context_html(true, true, false),
            crate::grid::menu_stylesheet(&crate::style::Theme::White.chrome(), left, top, crate::hud::CONTEXT_W),
            None,
            crate::chapter::viewport(1200, 900, 1.0, false),
            900.0,
        )
        .expect("the menu must lay out");

        let x = left + crate::hud::CONTEXT_W / 2.0;
        let mut found = Vec::new();
        for y in (top as u32..top as u32 + 5 * 34 + 32).step_by(2) {
            let what = doc
                .doc
                .hit(x, y as f32)
                .and_then(|hit| crate::ancestor_attr(doc.dom(), hit.node_id, "data-set"));
            if let Some(what) = what.filter(|w| found.last() != Some(w)) {
                found.push(what);
            }
        }
        assert_eq!(found, ["copy", "highlight", "unhighlight", "note", "bookmark"]);
    }

    /// Two columns are only worth having when each still gets a real measure;
    /// below that the fallback has to be silent, not a squeezed 38ch column.
    /// Typed into the go-to bar: a page when the book is measured, a
    /// percentage with `%` or when it is not.
    #[test]
    fn the_go_to_bar_reads_pages_and_percentages() {
        use super::goto_typed;
        assert_eq!(goto_typed("", Some(101)), None);
        assert_eq!(goto_typed("%", Some(101)), None);
        assert_eq!(goto_typed("1", Some(101)), Some(0.0));
        assert_eq!(goto_typed("51", Some(101)), Some(0.5));
        assert_eq!(goto_typed("101", Some(101)), Some(1.0));
        assert_eq!(goto_typed("9999", Some(101)), Some(1.0), "past the end is the end");
        assert_eq!(goto_typed("25%", Some(101)), Some(0.25));
        assert_eq!(goto_typed("25", None), Some(0.25), "no pages yet: a percentage");
        assert_eq!(goto_typed("0", Some(1)), Some(0.0), "a one-page book");
    }

    #[test]
    fn two_columns_need_the_width_and_the_reading_view() {
        let em = 20.0;
        let min = 2.0 * (MEASURE_EM + 2.0 * GUTTER_EM) * em;

        assert_eq!(columns_for(2, min, em, true), 2);
        assert_eq!(columns_for(2, min - 1.0, em, true), 1, "too narrow falls back");
        assert_eq!(columns_for(1, min * 2.0, em, true), 1, "not asked for");
        // The library and the contents are one document at window width.
        assert_eq!(columns_for(2, min, em, false), 1);
        // A bigger font needs a wider window for the same two columns.
        assert_eq!(columns_for(2, min, em * 1.6, true), 1);
    }
}

/// The row `by` steps from `at` among `n`, wrapping; with nothing reached yet,
/// down starts at the top and up at the bottom. `isize::MIN`/`MAX` are Home/End.
fn step_row(at: Option<usize>, by: isize, n: usize) -> usize {
    let n = n.max(1) as isize;
    (match (by, at) {
        (isize::MIN, _) => 0,
        (isize::MAX, _) => n - 1,
        (1.., None) => 0,
        (..=0, None) => n - 1,
        (_, Some(i)) => (i as isize + by).rem_euclid(n),
    }) as usize
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// `#rrggbb` to a colour. The chrome palette is authored as CSS strings so the
/// stylesheet and the window ground cannot drift apart.
pub fn parse_hex(s: &str) -> Color {
    let h = s.trim_start_matches('#');
    let v = u32::from_str_radix(h, 16).unwrap_or(0);
    Color::from_rgb8((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--check") {
        std::process::exit(check::run(&args[1..]));
    }

    if args.first().map(String::as_str) == Some("--index") {
        let code = match db::Db::open() {
            Ok(db) => {
                let (done, failed) = library::index_all(&db);
                println!("indexed {done} books, {failed} failed");
                (failed > 0) as i32
            }
            Err(e) => {
                eprintln!("omaread: {e}");
                1
            }
        };
        std::process::exit(code);
    }

    if args.first().map(String::as_str) == Some("--shot") {
        std::process::exit(shot::run(&args[1..]));
    }

    if matches!(args.first().map(String::as_str), Some("-h" | "--help")) {
        println!(
            "omaread [book.epub] [chapter]   open the library, or a book\n\
             omaread --check <book.epub>...  headless conformance run\n\
             omaread --shot <book.epub> <chapter> <page> <out.ppm> [hud]\n\
             omaread --index                 index every library book for search\n\n\
             Library:  type to search · #tag to filter · F2 tag a book\n\
                       Enter open · F3 open a file · Tab sort · F5 rescan · Esc clear/quit\n\
             Reading:  ←/→ page · ↑/↓ chapter · g go to page or % · Tab contents · / search · t theme · +/- size · c columns · l library · q quit\n\
                       move the mouse for the menus: marks, contents, size, columns\n\
             Contents: ↑/↓ select · Enter go · Tab or Esc back\n\
             Marks:    m list · b bookmark · drag to select · right-click menu · h highlight · y copy\n\
                       in the list: n note · x delete · e export to Markdown"
        );
        return;
    }

    // Dev convenience: jump straight to a spine item.
    let start = args
        .get(1)
        .and_then(|s| s.parse::<usize>().ok())
        .map(|n| n.saturating_sub(1))
        .unwrap_or(0);

    // AccessKit's requests arrive as user events (see `a11y.rs`).
    let event_loop = EventLoop::<accesskit_winit::Event>::with_user_event()
        .build()
        .expect("create event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new(args.first().cloned(), start);
    app.proxy = Some(event_loop.create_proxy());
    event_loop.run_app(&mut app).expect("run event loop");

    // Leave without running destructors: quitting should not wait on freeing
    // every document and glyph cache. Nothing here needs a destructor to be
    // correct: reading position and marks are committed to SQLite before this
    // point, and WAL means a process that simply stops loses nothing.
    app.save_position();
    std::process::exit(0);
}

/// Resources arrive from the provider on whatever thread fetched them; funnel
/// them back to the event loop rather than touching the document directly.
/// Hands a fetched resource to the window's thread, tagged with the document
/// that asked for it, and wakes the window when one was given: covers are
/// decoded off the window's thread (`net::CoverProvider`) and arrive while the
/// loop sleeps.
struct ChannelCallback(Sender<(usize, Resource)>, Option<Arc<Window>>);

impl NetCallback<Resource> for ChannelCallback {
    fn call(&self, doc_id: usize, result: Result<Resource, Option<String>>) {
        if let Ok(resource) = result {
            let _ = self.0.send((doc_id, resource));
            if let Some(w) = &self.1 {
                w.request_redraw();
            }
        }
    }
}

/// Which surface is on screen. Every arm is a laid-out document — the library
/// and the contents are HTML/CSS through the same pipeline as a book
/// (CONTEXT.md §2); only where the markup comes from differs.
/// What the contents view is listing. All three are "places in this book", so
/// they share one document, one set of keys and one hit-test path.
#[derive(Clone, Copy, PartialEq)]
enum TocMode {
    Contents,
    Search,
    Marks,
}

#[derive(Clone, Copy, PartialEq)]
enum View {
    Library,
    Reading,
    Toc,
}

struct App {
    view: View,
    rows: Vec<BookRow>,
    query: String,
    sort: Sort,
    selected: usize,
    /// Rebuilt whenever the query, sort or selection changes.
    lib_doc: Option<Chapter>,
    /// Completions for the library search box, refreshed with the rows.
    suggestions: Vec<(String, String)>,
    /// Cards the current grid document carries covers for. When the page moves
    /// off this range the grid has to be built again.
    lib_covers: std::ops::Range<usize>,
    /// The page those covers were chosen for; turning past it needs a rebuild.
    lib_page: usize,
    /// The library card under the pointer, for hover feedback.
    hover_card: Option<usize>,
    /// The `data-hud` control under the pointer, same idea.
    hover_hud: Option<String>,
    /// And the `data-set` row of the settings panel under it.
    hover_menu: Option<String>,
    /// Where the reader was before following a link or a contents entry, so the
    /// bar can offer them the way back: the chapter, its page, and how to say so.
    ///
    /// The wording is captured on the way out, while that position is still the
    /// current one — it is the only moment the whole-book page number or the
    /// percentage can be worked out exactly.
    back: Option<(usize, usize, String)>,
    /// A footnote or reference on show over the page.
    popup: Option<Chapter>,
    /// The settings panel over the library, and the folder path being typed
    /// into it, if any.
    menu: Option<Chapter>,
    /// Where the panel was anchored when it opened, in page CSS pixels.
    menu_at: Option<(f32, f32)>,
    adding_folder: Option<String>,
    /// A page turn in flight: the page being left, and which way it went.
    slide: Option<Slide>,
    /// The chapter that page is in, when the turn crossed into another one.
    /// Kept only while it slides off; `None` means the page is in `chapter`.
    slide_from: Option<Chapter>,
    /// The engine has panicked on the open book since it was opened. Counted
    /// once per opening, towards quarantine.
    panicked: bool,
    /// A quarantined book the library has warned about: opening it again
    /// opens it anyway.
    warned: Option<String>,
    /// A tag being typed for the selected book. Tagging borrows the search box.
    tagging: Option<String>,
    /// The contents list. Rebuilt on every open and on every selection move.
    toc_doc: Option<Chapter>,
    /// Title and progress, painted over the page while the pointer is active.
    hud_doc: Option<Chapter>,
    /// What `hud_doc` was built from; it is rebuilt only when this changes,
    /// because pointer motion arrives far faster than the text does.
    hud_key: String,
    /// The page under the HUD carries a bookmark; its icon takes the accent.
    hud_bookmarked: bool,
    hud_shown: bool,
    /// The foot of the page shows a percentage until you click it, then a page
    /// number. Clicking again swaps back.
    show_page: bool,
    /// The go-to bar (`g`): where it points and what has been typed into it.
    /// The fraction is of pages when the book is measured, of bytes when not —
    /// the same two readings the readout gives.
    goto: Option<(f32, String)>,
    /// The pointer went down on its track and has not come up yet.
    scrubbing: bool,
    /// The stored highlight the last press landed inside, if any. What makes the
    /// menu offer "Remove" instead of "Highlight".
    sel_mark: Option<i64>,
    /// Omarchy has put a new theme in place (`style::watch_omarchy`).
    theme_rx: Option<Receiver<()>>,
    /// Measured pages per chapter, and the layout they were measured at. This is
    /// what lets the readout say "page 27 of 336" and mean it.
    chapter_pages: Option<Vec<usize>>,
    pages_layout: String,
    /// Page counts being measured off the window's thread: for which book, at
    /// which layout, and where the answer will arrive.
    measuring: Option<(String, String, Receiver<Vec<usize>>)>,
    /// The portal's file dialog, open off the window's thread: whether it is
    /// picking a folder, and where the answer will arrive.
    picking: Option<(bool, Receiver<Result<Option<PathBuf>, String>>)>,
    /// In-book search: the query, and whether the contents view is showing hits
    /// instead of navigation. Both lists are "places in this book", so they are
    /// the same view.
    find: String,
    toc_mode: TocMode,
    /// Marks for the open book; index-aligned with the list while in Marks mode.
    marks: Vec<db::Mark>,
    /// Where each bookmark falls, by mark id, worked out as the list opens.
    bookmark_places: std::collections::HashMap<i64, String>,
    /// Live selection, anchor then focus, each as (text element, byte). The two
    /// may sit in different paragraphs.
    sel: Option<((usize, usize), (usize, usize))>,
    dragging: bool,
    /// Where the last press on the page landed, in physical pixels. A release
    /// near it is a click: open the figure or the note under it.
    press_at: Option<(f32, f32)>,
    /// An off-book link the popup is asking about: Enter opens it.
    leaving: Option<String>,
    /// Mark being annotated, and the note as typed so far.
    noting: Option<i64>,
    note: String,
    /// When to put the HUD away. Drives `ControlFlow::WaitUntil`.
    hud_until: Option<Instant>,
    /// `OMAREAD_EXIT_AFTER=<ms>` quits by itself. Shutdown is a real code path
    /// with real bugs in it (CONTEXT.md §6), and it cannot be tested by hand or
    /// in CI without a way to reach it that needs no window manager.
    exit_at: Option<Instant>,
    /// Consecutive frames the compositor refused, and when to try the next one.
    /// A surface that times out once stays wedged until it is reconfigured, so
    /// a dropped frame is not something to retry at frame rate.
    drops: u32,
    retry_at: Option<Instant>,
    toc_sel: usize,
    /// The reading page to come back to when the contents close. All three
    /// views share `page`, so it has to be put somewhere.
    resume_page: usize,
    book: Option<Book>,
    db: Option<Arc<Mutex<Db>>>,
    hash: String,
    path: String,
    /// Restored on startup, consumed once the chapter it points at is open.
    pending: Option<cfi::Cfi>,
    /// Patterns for the book's `dc:language`; None means no hyphenation.
    hyphenator: Option<hyphen::Hyphenator_>,
    style: ReadingStyle,
    chapter: Option<Chapter>,
    index: usize,
    page: usize,
    /// 1 or 2. Two-column falls back to one below `TWO_COLUMN_MIN_EM`.
    columns: usize,
    window: Option<Arc<Window>>,
    /// The screen-reader bridge. Idle, and cheap, until an assistive
    /// technology asks for the tree.
    a11y: Option<accesskit_winit::Adapter>,
    proxy: Option<EventLoopProxy<accesskit_winit::Event>>,
    renderer: VelloWindowRenderer,
    net_tx: Sender<(usize, Resource)>,
    net_rx: Receiver<(usize, Resource)>,
    size: (u32, u32),
    scale: f32,
    /// Omarchy's text size as a factor (`style::omarchy_ui_scale`). The chrome
    /// — HUD, menus, popups — is laid out at `ui_scale()`, the page is not:
    /// the page has its own font size.
    ui: f32,
    cursor: (f32, f32),
}

impl App {
    fn new(open: Option<String>, start: usize) -> Self {
        let (net_tx, net_rx) = channel();

        // Progress is best-effort: a broken database must never stop you reading.
        let db = Db::open()
            .map_err(|e| eprintln!("omaread: no progress database ({e})"))
            .ok()
            .map(|d| Arc::new(Mutex::new(d)));
        let omarchy = style::omarchy_chrome();

        let mut app = Self {
            view: View::Library,
            rows: Vec::new(),
            query: String::new(),
            sort: Sort::Recent,
            selected: 0,
            lib_doc: None,
            suggestions: Vec::new(),
            lib_covers: 0..0,
            lib_page: 0,
            hover_card: None,
            hover_hud: None,
            hover_menu: None,
            a11y: None,
            proxy: None,
            slide: None,
            slide_from: None,
            panicked: false,
            warned: None,
            back: None,
            popup: None,
            menu: None,
            menu_at: None,
            adding_folder: None,
            tagging: None,
            toc_doc: None,
            hud_doc: None,
            hud_key: String::new(),
            hud_bookmarked: false,
            hud_shown: false,
            show_page: false,
            goto: None,
            theme_rx: None,
            scrubbing: false,
            sel_mark: None,
            chapter_pages: None,
            pages_layout: String::new(),
            measuring: None,
            picking: None,
            find: String::new(),
            toc_mode: TocMode::Contents,
            marks: Vec::new(),
            bookmark_places: Default::default(),
            sel: None,
            dragging: false,
            noting: None,
            note: String::new(),
            hud_until: None,
            exit_at: std::env::var("OMAREAD_EXIT_AFTER")
                .ok()
                .and_then(|v| v.parse().ok())
                .map(|ms| Instant::now() + Duration::from_millis(ms)),
            drops: 0,
            retry_at: None,
            toc_sel: 0,
            resume_page: 0,
            book: None,
            db,
            hash: String::new(),
            path: String::new(),
            pending: None,
            hyphenator: None,
            style: ReadingStyle {
                scale: style::setting("font-scale")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1.0),
                // On Omarchy the reader matches the desktop until told otherwise.
                theme: style::setting("theme")
                    .as_deref()
                    .and_then(Theme::named)
                    .unwrap_or(match omarchy {
                        Some(_) => Theme::Omarchy,
                        None => ReadingStyle::default().theme,
                    }),
                omarchy,
            },
            chapter: None,
            index: start,
            page: 0,
            columns: 1,
            window: None,
            renderer: VelloWindowRenderer::new(),
            net_tx,
            net_rx,
            size: (900, 1000),
            scale: 1.0,
            ui: style::omarchy_ui_scale(),
            cursor: (0.0, 0.0),
            press_at: None,
            leaving: None,
        };

        println!(
            "omaread: chrome — {}",
            match &app.style.omarchy {
                Some(c) => format!("Omarchy palette, ground {}", c.bg),
                None => "built-in palette (no omaread.css from Omarchy)".into(),
            }
        );

        if let Some(db) = app.db.clone() {
            if let Ok(d) = db.lock() {
                let (seen, added) = library::scan(&d);
                println!("omaread: library — {seen} files, {added} newly indexed");
            }
        }
        app.reload_rows();

        if let Some(p) = open {
            app.open_path(&PathBuf::from(p), start);
        }
        app
    }

    fn reload_rows(&mut self) {
        self.rows = self
            .db
            .as_ref()
            .and_then(|d| d.lock().ok()?.books(&self.query, self.sort).ok())
            .unwrap_or_default();
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
        self.suggestions = self.completions();
        self.lib_doc = None;
    }

    /// What to offer under the search box: tags while tagging or filtering by
    /// one, words and authors otherwise.
    fn completions(&self) -> Vec<(String, String)> {
        let Some(db) = self.db.as_ref().and_then(|d| d.lock().ok()) else { return Vec::new() };

        let tag_prefix = match (&self.tagging, self.query.trim().strip_prefix('#')) {
            (Some(t), _) => Some(t.as_str()),
            (None, Some(q)) => Some(q),
            (None, None) => None,
        };
        if let Some(prefix) = tag_prefix {
            return db
                .all_tags(prefix)
                .into_iter()
                .map(|(tag, n)| (format!("#{tag}"), i18n::tr!("books-count", n = n)))
                .collect();
        }
        match self.query.trim().is_empty() {
            true => Vec::new(),
            false => db.suggestions(&self.query, 6),
        }
    }

    /// Toggle the typed tag on the selected book. One key, both directions.
    fn apply_tag(&mut self) {
        let Some(tag) = self.tagging.take() else { return };
        let Some(row) = self.rows.get(self.selected).cloned() else { return };
        if let Some(db) = self.db.as_ref().and_then(|d| d.lock().ok()) {
            match db.toggle_tag(&row.hash, &tag) {
                Ok(true) => println!("omaread: tagged {} #{tag}", row.title),
                Ok(false) => println!("omaread: untagged {} #{tag}", row.title),
                Err(e) => eprintln!("omaread: could not tag: {e}"),
            }
        }
        self.reload_rows();
    }

    /// Import if needed, then open for reading.
    fn open_path(&mut self, path: &Path, start: usize) {
        let path = match self.db.as_ref().and_then(|d| d.lock().ok()) {
            Some(db) => library::import(&db, path),
            None => path.to_path_buf(),
        };
        let book = match Book::open(&path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("omaread: {e}");
                return;
            }
        };

        self.hash = db::file_hash(&path).unwrap_or_default();
        self.panicked = false;
        self.pending = self
            .db
            .as_ref()
            .and_then(|d| d.lock().ok()?.last_cfi(&self.hash).ok().flatten())
            .and_then(|s| cfi::Cfi::parse(&s));
        self.path = path.to_string_lossy().into_owned();

        let start = match &self.pending {
            Some(c) if c.spine < book.chapter_count() => c.spine,
            _ => start,
        };
        self.index_book(&book);
        self.hyphenator = hyphen::Hyphenator_::for_language(&book.language);
        println!(
            "omaread: {} ({} chapters, {})",
            book.title,
            book.chapter_count(),
            match (&book.language, self.hyphenator.is_some()) {
                (l, true) if !l.is_empty() => format!("hyphenating {l}"),
                (l, false) if !l.is_empty() => format!("no patterns for {l}"),
                _ => "no language declared".into(),
            }
        );

        if let Some(w) = &self.window {
            w.set_title(&format!("{} — Omaread", book.title));
        }
        self.book = Some(book);
        self.index = start;
        self.view = View::Reading;
        // Before the window exists the size is a guess, and `resumed` reloads
        // at the real one — laying the chapter out twice at startup, which for
        // an illustrated book is a wasted 120ms.
        if self.window.is_some() {
            self.load_chapter(start);
        }
        // Reflowing a book laid out page by page cannot look as designed; say
        // so, once per opening, rather than let it look broken.
        if self.book.as_ref().is_some_and(|b| b.fixed_layout) {
            self.open_note(&i18n::tr!("fixed-layout-notice"));
        }
        self.request_redraw();
    }

    fn open_selected(&mut self) {
        let Some(row) = self.rows.get(self.selected).cloned() else { return };
        if row.missing {
            eprintln!("omaread: {} is missing from disk", row.title);
            return;
        }
        // A book that took the engine down on two openings is not reopened on
        // a single click: say so, and let a second one through.
        if row.quarantined && self.warned.as_deref() != Some(row.hash.as_str()) {
            self.warned = Some(row.hash.clone());
            self.open_note(&i18n::tr!("open-quarantined", title = row.title.clone()));
            return;
        }
        self.warned = None;
        self.open_path(&PathBuf::from(&row.path), 0);
    }

    /// The engine panicked on the open book. One count per opening, so a bad
    /// chapter turned past twice is not two strikes; the second opening that
    /// panics quarantines it (CONTEXT.md §6).
    fn engine_panicked(&mut self) {
        if self.book.is_none() || std::mem::replace(&mut self.panicked, true) {
            return;
        }
        let Some(db) = self.db.as_ref().and_then(|d| d.lock().ok()) else { return };
        match db.note_panic(&self.hash) {
            Ok(n) if n >= db::QUARANTINE_AFTER => {
                eprintln!("omaread: the engine panicked on this book on {n} openings; quarantined")
            }
            Ok(_) => {}
            Err(e) => eprintln!("omaread: could not count the panic: {e}"),
        }
    }

    /// Lay out the library grid. Rebuilt on any change to query, sort or
    /// selection — the whole document is cheap next to a chapter, and an
    /// incremental DOM diff is complexity nobody has asked for.
    /// Lay out the library grid.
    ///
    /// Twice, deliberately. Loading a cover costs far more than laying out a
    /// card — 1817ms against 91ms for 361 books, measured — and only about
    /// fourteen covers are ever on screen. The first pass carries no covers and
    /// exists solely to find out which cards this page shows; the second gives
    /// those cards their covers. Before this, returning to the library blocked
    /// for nearly two seconds and swallowed the clicks that arrived meanwhile.
    fn build_library(&mut self) {
        let started = Instant::now();
        let ua = grid::stylesheet(&self.chrome());
        let markup = |covers: std::ops::Range<usize>| {
            grid::html(
                &self.rows,
                &self.query,
                self.sort,
                &self.suggestions,
                self.tagging.as_deref(),
                covers,
            )
        };

        // The probe carries the *same* markup as the real grid — every card with
        // its `<img>` — and simply has no provider to fetch through. Anything
        // less paginates differently: a blank jacket is text, which a page break
        // may fall inside, while a cover is an image block that cannot be split,
        // so a probe made of jackets fitted two rows where covers fit one.
        let probe = chapter::layout_document(
            markup(0..self.rows.len()),
            ua.clone(),
            None,
            self.viewport(),
            self.page_height(),
        );
        let visible = probe
            .as_ref()
            .map(|d| visible_cards(d, self.page))
            .unwrap_or(0..self.rows.len());

        let provider = self.db.clone().map(|db| {
            let px = |css: u32| (css as f32 * self.scale).round() as u32;
            Arc::new(net::CoverProvider::new(db, self.callback(), (px(grid::COVER_W), px(grid::COVER_H))))
                as Arc<dyn blitz_traits::net::NetProvider<Resource>>
        });
        self.lib_doc = chapter::layout_document(
            markup(visible.clone()),
            ua,
            provider,
            self.viewport(),
            self.page_height(),
        );
        self.lib_covers = visible;
        self.page = self
            .page
            .min(self.lib_doc.as_ref().map_or(0, |c| c.pages.count().saturating_sub(1)));
        self.lib_page = self.page;

        if std::env::var_os("OMAREAD_DEBUG_TIME").is_some() {
            eprintln!(
                "TIME grid rebuild: {} rows, covers {:?} in {:.0}ms",
                self.rows.len(),
                self.lib_covers,
                started.elapsed().as_secs_f32() * 1000.0
            );
        }
    }


    /// Lay out the HUD, unless the one in hand already says the right thing.
    fn build_hud(&mut self) {
        let Some(book) = self.book.clone() else { return };
        let readout = self.readout();
        let cols = self.effective_columns();
        let height = self.size.1 as f32 / self.ui_scale();
        let on_mark = self.sel_mark.is_some();
        let back = self.back.as_ref().map(|(_, _, label)| label.clone());
        self.hud_bookmarked = self.page_bookmarked();
        let scrub = self.goto.as_ref().map(|(at, _)| (*at, self.goto_label(*at)));
        let key = format!(
            "{readout}|{cols}|{on_mark}|{height}|{:?}|{}|{:?}|{}|{scrub:?}",
            self.style.theme,
            book.title,
            back,
            self.hud_bookmarked,
        );
        if key == self.hud_key && self.hud_doc.is_some() {
            return;
        }

        let html = hud::html(
            &book.title,
            &readout,
            cols,
            on_mark,
            height,
            back.as_deref(),
            self.hud_bookmarked,
            scrub.as_ref().map(|(at, label)| (*at, label.as_str())),
        );
        let ua = hud::stylesheet(&self.chrome());
        // The HUD spans the window; only the page is laid out at column width.
        let vp = chapter::viewport(self.size.0, self.size.1, self.ui_scale(), self.style.dark());
        self.hud_doc = chapter::layout_document(html, ua, None, vp, self.page_height());
        self.hud_key = key;
    }

    /// Index the book's text the first time it is opened. Search over a book
    /// you have never opened is what `--index` is for.
    fn index_book(&self, book: &Book) {
        let Some(db) = self.db.as_ref().and_then(|d| d.lock().ok()) else { return };
        if db.is_indexed(&self.hash) {
            return;
        }
        match library::index_book(&db, &self.hash, book) {
            Ok(n) => println!("omaread: indexed {n} chapters for search"),
            // Search is a convenience; a failure here must not stop you reading.
            Err(e) => eprintln!("omaread: could not index: {e}"),
        }
    }

    /// Marks, selection, notes and copy — everything Phase 7 adds.
    fn open_marks(&mut self) {
        self.marks = self.load_marks();
        self.bookmark_places = self.bookmark_places();
        self.toc_mode = TocMode::Marks;
        self.open_toc();
    }

    /// Where each bookmark falls, in the readout's own terms: the book's page
    /// once pages are measured, a percentage until then. Worked out while still
    /// in the reading view, because that is the layout the numbers belong to.
    ///
    /// ponytail: lays out every other chapter holding a bookmark on the window's
    /// thread, ~25 ms each; store the place with the mark if a book ever
    /// collects dozens.
    fn bookmark_places(&self) -> std::collections::HashMap<i64, String> {
        let mut places = std::collections::HashMap::new();
        let Some(book) = &self.book else { return places };
        let mut others = std::collections::HashMap::new();
        for m in self.marks.iter().filter(|m| m.is_bookmark()) {
            let Some(c) = cfi::Cfi::parse(&m.cfi) else { continue };
            let ch = match &self.chapter {
                Some(ch) if ch.index == c.spine => ch,
                _ => {
                    let laid = others.entry(c.spine).or_insert_with(|| {
                        let (w, h, ph) = (self.column_width(), self.size.1, self.page_height());
                        let hy = self.hyphenator.as_ref();
                        chapter::load_offscreen(book, c.spine, &self.style, hy, w, h, self.scale, ph)
                    });
                    let Some(ch) = laid.as_ref() else { continue };
                    ch
                }
            };
            let Some(y) = chapter::top_of_cfi(ch, &c) else { continue };
            let page = ch.pages.page_containing(y);
            let place = match self.measured_pages() {
                Some(per) => {
                    let before: usize = per.iter().take(c.spine).sum();
                    i18n::tr!("mark-page", n = before + page + 1)
                }
                None => {
                    let within = ch.pages.top_of(page) / ch.pages.content_height.max(1.0);
                    format!("{}%", (book.progress(c.spine, within) * 100.0).round() as u8)
                }
            };
            places.insert(m.id, place);
        }
        places
    }

    fn load_marks(&self) -> Vec<db::Mark> {
        self.db
            .as_ref()
            .and_then(|d| d.lock().ok())
            .map(|d| d.marks(&self.hash))
            .unwrap_or_default()
    }

    /// CFI of the character this page begins at — the anchor for a bookmark,
    /// and the same one reading progress uses. Without the character a page
    /// that starts mid-paragraph is the paragraph's first page.
    fn cfi_of_page(&self) -> Option<cfi::Cfi> {
        let ch = self.chapter.as_ref()?;
        let top = ch.pages.top_of(self.page);
        let node = chapter::node_at(ch.dom(), top)?;
        let mut c = cfi::of_node(ch.dom(), node, self.index)?;
        c.offset = chapter::char_at(ch.dom(), node, top);
        Some(c)
    }

    /// Whether the page on screen carries a bookmark. One indexed lookup, so
    /// the HUD asks on every build rather than keeping a copy that could drift.
    fn page_bookmarked(&self) -> bool {
        self.cfi_of_page().is_some_and(|c| {
            let db = self.db.as_ref().and_then(|d| d.lock().ok());
            db.and_then(|db| db.bookmark_at(&self.hash, &c.to_string())).is_some()
        })
    }

    fn toggle_bookmark(&mut self) {
        let Some(cfi) = self.cfi_of_page() else { return };
        let cfi = cfi.to_string();
        let Some(db) = self.db.as_ref().and_then(|d| d.lock().ok()) else { return };

        match db.bookmark_at(&self.hash, &cfi) {
            Some(id) => {
                let _ = db.remove_mark(id);
                println!("omaread: bookmark removed");
            }
            None => {
                let mark = db::Mark { cfi, ..Default::default() };
                match db.add_mark(&self.hash, &mark) {
                    Ok(()) => println!("omaread: bookmarked page {}", self.page + 1),
                    Err(e) => eprintln!("omaread: could not bookmark: {e}"),
                }
            }
        }
    }

    /// The selection as (element, byte range), one per paragraph it covers.
    fn selected(&self) -> Option<Vec<(usize, std::ops::Range<usize>)>> {
        let (a, f) = self.sel?;
        let ranges = chapter::ranges_between(self.chapter.as_ref()?.dom(), a, f);
        (!ranges.is_empty()).then_some(ranges)
    }

    /// The selection's first element, its character offset there, its length
    /// in characters across every paragraph, and its text, a paragraph a line.
    fn selected_span(&self) -> Option<(usize, usize, usize, String)> {
        let ranges = self.selected()?;
        let dom = self.chapter.as_ref()?.dom();
        let spans: Vec<_> = ranges.iter().filter_map(|(id, r)| chapter::char_span(dom, *id, r.clone())).collect();
        let first = spans.first()?;
        let text: Vec<&str> = spans.iter().map(|s| s.2.as_str()).collect();
        Some((ranges[0].0, first.0, spans.iter().map(|s| s.1).sum(), text.join("\n")))
    }

    fn selected_text(&self) -> Option<String> {
        self.selected_span().map(|s| s.3)
    }

    /// Returns the new mark's `(cfi, length)`, which is what identifies it.
    fn highlight_selection(&mut self) -> Option<(String, usize)> {
        let Some((node, start, len, text)) = self.selected_span() else {
            println!("omaread: nothing selected — drag across the text first");
            return None;
        };
        let ch = self.chapter.as_ref()?;
        let mut cfi = cfi::of_node(ch.dom(), node, self.index)?;
        cfi.offset = Some(start);

        let mark = db::Mark {
            cfi: cfi.to_string(),
            length: len,
            text,
            ..Default::default()
        };
        if let Some(db) = self.db.as_ref().and_then(|d| d.lock().ok()) {
            match db.add_mark(&self.hash, &mark) {
                Ok(()) => println!("omaread: highlighted “{}”", mark.text),
                Err(e) => eprintln!("omaread: could not highlight: {e}"),
            }
        }
        self.sel = None;
        self.request_redraw();
        Some((mark.cfi, mark.length))
    }

    /// Note the highlight the press landed in, or highlight the selection and
    /// note that. The typing happens in the marks list, where notes always have.
    fn note_here(&mut self) {
        let id = match self.sel_mark.take() {
            Some(id) => Some(id),
            None => self.highlight_selection().and_then(|(cfi, len)| {
                self.load_marks().into_iter().find(|m| m.cfi == cfi && m.length == len)
            }).map(|m| m.id),
        };
        let Some(id) = id else { return };
        self.open_marks();
        let Some(i) = self.marks.iter().position(|m| m.id == id) else { return };
        self.toc_sel = i;
        self.begin_note();
        self.build_toc();
        self.page = self.toc_doc.as_ref().and_then(|d| page_of_index(d, i)).unwrap_or(0);
        self.request_redraw();
    }

    fn copy_selection(&mut self) {
        let Some(text) = self.selected_text() else {
            println!("omaread: nothing selected");
            return;
        };
        // ponytail: wl-copy rather than a clipboard crate. Wayland is the
        // target (§1) and this is one process instead of one dependency; swap in
        // a crate if a non-Wayland target ever matters.
        use std::io::Write;
        use std::process::{Command, Stdio};
        let spawned = Command::new("wl-copy").stdin(Stdio::piped()).spawn();
        match spawned {
            Ok(mut child) => {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(text.as_bytes());
                }
                let _ = child.wait();
                println!("omaread: copied {} characters", text.chars().count());
            }
            Err(e) => eprintln!("omaread: could not run wl-copy ({e}); install wl-clipboard"),
        }
    }

    /// Write the book's highlights and notes to a Markdown file of its own,
    /// beside the library, and say where.
    fn export_marks(&mut self) {
        let Some(book) = &self.book else { return };
        let md = marks_markdown(&book.title, &book.author, &book.toc, &self.marks);
        let dir = db::highlights_dir();
        let path = dir.join(library::canonical_name(&book.author, &book.title, "md"));
        let said = match std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, md)) {
            Ok(()) => i18n::tr!("marks-exported", path = path.display().to_string()),
            Err(e) => i18n::tr!("marks-export-failed", error = e.to_string()),
        };
        self.open_note(&said);
    }

    fn begin_note(&mut self) {
        let Some(mark) = self.marks.get(self.toc_sel) else { return };
        self.note = mark.note.clone();
        self.noting = Some(mark.id);
    }

    fn save_note(&mut self) {
        let Some(id) = self.noting.take() else { return };
        if let Some(db) = self.db.as_ref().and_then(|d| d.lock().ok()) {
            if let Err(e) = db.set_note(id, &self.note) {
                eprintln!("omaread: could not save note: {e}");
            }
        }
        self.note.clear();
        self.marks = self.load_marks();
    }

    fn delete_mark(&mut self) {
        let Some(mark) = self.marks.get(self.toc_sel) else { return };
        let id = mark.id;
        if let Some(db) = self.db.as_ref().and_then(|d| d.lock().ok()) {
            let _ = db.remove_mark(id);
        }
        self.marks = self.load_marks();
        self.toc_sel = self.toc_sel.min(self.marks.len().saturating_sub(1));
    }

    /// Flow-coordinate rectangles to paint over this page: the live selection,
    /// and every stored highlight in this chapter.
    fn selection_rects(&self) -> Vec<(f32, f32, f32, f32)> {
        let Some(ch) = self.chapter.as_ref() else { return Vec::new() };
        let ranges = self.selected().unwrap_or_default();
        ranges.into_iter().flat_map(|(id, r)| chapter::range_rects(ch.dom(), id, r)).collect()
    }

    /// ponytail: re-queried every frame. It is one indexed read on a WAL
    /// database and only matters while dragging; cache per chapter if a profile
    /// ever shows it.
    ///
    /// Returns the highlights, and an underline along the foot of each one that
    /// carries a note — the mark that says a click will show it.
    #[allow(clippy::type_complexity)]
    fn highlight_rects(&self) -> (Vec<(f32, f32, f32, f32)>, Vec<(f32, f32, f32, f32)>) {
        let (Some(ch), Some(db)) = (
            self.chapter.as_ref(),
            self.db.as_ref().and_then(|d| d.lock().ok()),
        ) else {
            return (Vec::new(), Vec::new());
        };
        let (mut all, mut noted) = (Vec::new(), Vec::new());
        for m in db.marks_in(&self.hash, self.index) {
            let Some(c) = cfi::Cfi::parse(&m.cfi) else { continue };
            let Some(node) = cfi::resolve(ch.dom(), &c) else { continue };
            let rects = chapter::highlight_rects(ch.dom(), node, c.offset.unwrap_or(0), m.length);
            if !m.note.is_empty() {
                noted.extend(rects.iter().map(|&(x0, _, x1, y1)| (x0, y1 - 2.0, x1, y1)));
            }
            all.extend(rects);
        }
        (all, noted)
    }

    /// The note on a stored highlight, if it has one.
    fn mark_note(&self, id: i64) -> Option<String> {
        let db = self.db.as_ref()?.lock().ok()?;
        db.marks_in(&self.hash, self.index)
            .into_iter()
            .find(|m| m.id == id && !m.note.is_empty())
            .map(|m| m.note)
    }

    /// The figure under the pointer, as an in-archive path.
    fn figure_at(&self) -> Option<String> {
        let ch = self.chapter.as_ref()?;
        let (fx, fy) = self.pointer_in_flow();
        let hit = ch.doc.hit(fx, fy)?;
        if chapter::tag_of(ch.dom(), hit.node_id)? != "img" {
            return None;
        }
        let src = ancestor_attr(ch.dom(), hit.node_id, "src")?;
        let here = self.book.as_ref()?.chapter_href(self.index)?.to_string();
        let dir = here.rsplit_once('/').map_or("", |(d, _)| d);
        net::in_archive_path(&format!("{dir}/{src}"))
    }

    /// A figure on its own, as large as the window allows and never larger than
    /// it is: the page caps images at 85vh, and a technical figure needs more.
    /// It is the popup, so the next click or key puts it away.
    fn open_figure(&mut self, href: &str) {
        let Some(book) = self.book.clone() else { return };
        // The book's provider answers synchronously, so the image is in hand
        // once the document is built; no trip through the event loop.
        let (tx, rx) = channel();
        let provider: Arc<dyn blitz_traits::net::NetProvider<Resource>> =
            Arc::new(net::BookNetProvider::new(book, Arc::new(ChannelCallback(tx, None))));
        let bg = self.chrome().bg;
        let vp = chapter::viewport(self.size.0, self.size.1, self.ui_scale(), self.style.dark());
        let src = format!("{}{}", net::ORIGIN, href.trim_start_matches('/'));
        let Some(mut fig) = chapter::layout_document(
            hud::figure_html(&src),
            hud::figure_stylesheet(&bg),
            Some(provider),
            vp.clone(),
            self.page_height(),
        ) else {
            return;
        };
        for (_, r) in rx.try_iter() {
            fig.doc.load_resource(r);
        }
        chapter::relayout(&mut fig, vp, self.page_height());
        self.popup = Some(fig);
        self.request_redraw();
    }

    /// Begin a selection at the pointer. Coordinates arrive in physical pixels.
    fn begin_selection(&mut self) {
        // Remember a highlight under the press, so the menu can offer to remove
        // it — "select a highlight, then remove it from the menu".
        self.sel_mark = self.mark_at(self.pointer_in_flow());
        let Some(local) = self.pointer_in_text() else {
            self.sel = None;
            self.request_redraw();
            return;
        };
        let Some(at) = self.text_point(local) else { return };
        self.sel = Some((at, at));
        self.dragging = true;
    }

    /// Move the selection's focus to the text under the pointer, in whatever
    /// paragraph that is. Between paragraphs, or in the margin, it stays where
    /// it was.
    fn extend_selection(&mut self) {
        let Some((anchor, _)) = self.sel else { return };
        let Some(focus) = self.pointer_in_text().and_then(|l| self.text_point(l)) else { return };
        self.sel = Some((anchor, focus));
        self.request_redraw();
    }

    /// The byte a point local to a text element falls on.
    fn text_point(&self, (node, x, y): (usize, f32, f32)) -> Option<(usize, usize)> {
        let tl = chapter::text_layout(self.chapter.as_ref()?.dom(), node)?;
        let s = tl.layout.scale();
        Some((node, parley::Cursor::from_point(&tl.layout, x * s, y * s).index()))
    }

    /// Pointer position in flow coordinates (CSS pixels into the chapter).
    ///
    /// In two-column mode the right half of the window is a different page of
    /// the same flow, so the column under the pointer decides both the vertical
    /// offset and how much to take off the x.
    fn pointer_in_flow(&self) -> (f32, f32) {
        let cols = self.effective_columns();
        let col_w = (self.size.0 as f32 / self.scale) / cols as f32;
        let x = self.cursor.0 / self.scale;
        let col = ((x / col_w).floor() as usize).min(cols - 1);
        let page = (self.page + col).min(self.page_count().saturating_sub(1));
        let y = self.cursor.1 / self.scale - self.page_margin();
        (
            x - col as f32 * col_w,
            self.chapter.as_ref().map_or(y, |c| c.pages.flow_y(page, y)),
        )
    }

    /// The text element under the pointer, with pointer coordinates local to it.
    fn pointer_in_text(&self) -> Option<(usize, f32, f32)> {
        let ch = self.chapter.as_ref()?;
        let (fx, fy) = self.pointer_in_flow();
        let hit = ch.doc.hit(fx, fy)?;
        let node = chapter::text_element(ch.dom(), hit.node_id)?;
        let (ox, oy) = chapter::node_origin(ch.dom(), node)?;
        Some((node, fx - ox, fy - oy))
    }

    /// Open the settings panel under the cog, and remember where that was.
    ///
    /// The anchor is taken once, here. Changing the theme drops the shelf
    /// document to rebuild it with the new palette, and a panel that asked the
    /// shelf where the cog is *while it was gone* jumped to the left margin.
    fn open_menu(&mut self) {
        // The cog is in the shelf's flow, and the shelf is a page, so its window
        // position is the flow position plus the page margin.
        let margin = self.page_margin();
        self.menu_at = self
            .lib_doc
            .as_ref()
            .and_then(|d| find_by_attr(d.dom(), 0, "data-icon", "gear"))
            .and_then(|n| chapter::node_rect(self.lib_doc.as_ref()?.dom(), n))
            .map(|(x, y, _, h)| (x - 10.0, y + h + margin));
        self.build_menu();
    }

    /// Lay out the settings panel where `open_menu` put it.
    fn build_menu(&mut self) {
        // The anchor is in page pixels and the panel is chrome, laid out at
        // `ui_scale`; `ui` is the step between the two. Taken here, not when
        // the panel opened, so F5 picking up a new text size moves it with the
        // cog. Clamped so a cog near the right edge does not hang the panel off
        // the screen.
        let ui = self.ui;
        let win_w = self.size.0 as f32 / self.ui_scale();
        let (left, top) = self.menu_at.map_or((grid::SIDE_PAD as f32, 74.0), |(x, y)| {
            (
                (x / ui).min(win_w - grid::MENU_W - grid::SIDE_PAD as f32).max(grid::SIDE_PAD as f32),
                y / ui + 10.0,
            )
        });
        let folders: Vec<String> =
            library::folders().iter().map(|p| p.display().to_string()).collect();
        let html = grid::menu_html(
            &format!("{:?}", self.style.theme),
            self.style.omarchy.is_some(),
            &folders,
            self.adding_folder.as_deref(),
        );
        let vp = chapter::viewport(self.size.0, self.size.1, self.ui_scale(), self.style.dark());
        self.menu = chapter::layout_document(
            html,
            grid::menu_stylesheet(&self.chrome(), left, top, grid::MENU_W),
            None,
            vp,
            self.page_height(),
        );
        self.request_redraw();
    }

    /// The panel row under the pointer, by its `data-set` name.
    fn menu_under_pointer(&self) -> Option<String> {
        let menu = self.menu.as_ref()?;
        let (x, y) = self.pointer_in_ui();
        let hit = menu.doc.hit(x, y)?;
        ancestor_attr(menu.dom(), hit.node_id, "data-set")
    }

    /// The open menu's rows, in order, as `(data-set, data-label)`.
    fn menu_rows(&self) -> Vec<(String, String)> {
        fn walk(dom: &blitz_dom::BaseDocument, id: usize, out: &mut Vec<(String, String)>) {
            let Some(node) = dom.get_node(id) else { return };
            if let blitz_dom::NodeData::Element(el) = &node.data {
                let attr = |n: &str| el.attrs.iter().find(|a| &*a.name.local == n).map(|a| a.value.to_string());
                if let (Some(set), Some(label)) = (attr("data-set"), attr("data-label")) {
                    out.push((set, label));
                }
            }
            for c in &node.children {
                walk(dom, *c, out);
            }
        }
        let mut out = Vec::new();
        if let Some(m) = &self.menu {
            walk(m.dom(), 0, &mut out);
        }
        out
    }

    /// Move the menu's highlighted row: the hover wash, driven by the arrows.
    /// `by` wraps; `isize::MIN` and `isize::MAX` go to the first and last row.
    fn step_menu(&mut self, by: isize) {
        let rows = self.menu_rows();
        if rows.is_empty() {
            return;
        }
        let at = self.hover_menu.as_ref().and_then(|h| rows.iter().position(|(s, _)| s == h));
        self.hover_menu = Some(rows[step_row(at, by, rows.len())].0.clone());
        self.request_redraw();
    }

    /// The menu's own keys: arrows walk the rows, Enter or Space acts on the
    /// one reached. Returns false for any other key.
    fn menu_key(&mut self, key: &Key) -> bool {
        match key {
            Key::Named(NamedKey::ArrowDown) => self.step_menu(1),
            Key::Named(NamedKey::ArrowUp) => self.step_menu(-1),
            Key::Named(NamedKey::Home) => self.step_menu(isize::MIN),
            Key::Named(NamedKey::End) => self.step_menu(isize::MAX),
            Key::Named(NamedKey::Enter | NamedKey::Space) => {
                if let Some(what) = self.hover_menu.clone() {
                    self.menu_do(what);
                }
            }
            _ => return false,
        }
        true
    }

    /// A press inside the settings panel. Returns false when the panel is not
    /// open, or the press missed every control — which closes it.
    fn menu_action(&mut self) -> bool {
        if self.menu.is_none() {
            return false;
        }
        let Some(what) = self.menu_under_pointer() else {
            self.menu = None;
            self.adding_folder = None;
            self.request_redraw();
            // On the page a press elsewhere is a press like any other: it
            // starts the next selection.
            return self.view != View::Reading;
        };
        self.menu_do(what)
    }

    /// Act on a menu row by its `data-set` name: a click, Enter on the row
    /// the arrows reached, or a screen reader's request.
    fn menu_do(&mut self, what: String) -> bool {
        // The reading view's context menu: act once, then get out of the way.
        if self.view == View::Reading {
            self.menu = None;
            match what.as_str() {
                "copy" => self.copy_selection(),
                "highlight" => drop(self.highlight_selection()),
                "unhighlight" => self.remove_selected_mark(),
                "note" => self.note_here(),
                "bookmark" => self.toggle_bookmark(),
                _ => {}
            }
            self.request_redraw();
            return true;
        }

        match what.split_once(':') {
            Some(("theme", name)) => {
                self.style.theme = Theme::named(name).unwrap_or(Theme::Sepia);
                style::set_setting("theme", name);
                self.lib_doc = None;
                self.hud_doc = None;
            }
            Some(("drop", path)) => {
                let keep: Vec<std::path::PathBuf> = library::folders()
                    .into_iter()
                    .filter(|p| p.display().to_string() != path)
                    .collect();
                if let Err(e) = library::set_folders(&keep) {
                    eprintln!("omaread: {e}");
                }
            }
            _ if what == "add" => self.pick(true),
            _ if what == "open" => {
                self.menu = None;
                self.pick(false);
                self.request_redraw();
                return true;
            }
            _ => {}
        }
        self.build_menu();
        true
    }

    /// The context menu, at the pointer: a right click, or the end of a drag.
    /// It lives in `menu`, like the settings panel, so hover, hit-testing and
    /// painting are the panel's.
    fn open_context_menu(&mut self) {
        let html = hud::context_html(
            self.selected().is_some(),
            self.sel_mark.is_some(),
            self.page_bookmarked(),
        );

        let (win_w, win_h) = (self.size.0 as f32 / self.ui_scale(), self.size.1 as f32 / self.ui_scale());
        let (x, y) = self.pointer_in_ui();
        let left = x.min(win_w - hud::CONTEXT_W - 8.0).max(0.0);

        // Laid out once at the top to measure it, then again where it fits: a
        // translated row that wraps makes it taller than its row count says.
        let vp = chapter::viewport(self.size.0, self.size.1, self.ui_scale(), self.style.dark());
        let chrome = self.chrome();
        let lay = |top: f32| {
            chapter::layout_document(
                html.clone(),
                grid::menu_stylesheet(&chrome, left, top, hud::CONTEXT_W),
                None,
                vp.clone(),
                self.page_height(),
            )
        };
        let height = lay(0.0)
            .and_then(|m| chapter::node_rect(m.dom(), find_by_attr(m.dom(), 0, "class", "menu")?))
            .map_or(0.0, |(_, _, _, h)| h);
        self.menu = lay(y.min(win_h - height - 8.0).max(0.0));
        self.hover_menu = None;
        self.request_redraw();
    }

    /// Add whatever folder was typed.
    fn add_folder(&mut self) {
        let Some(typed) = self.adding_folder.take() else { return };
        self.add_dir(library::expand_home(typed.trim()));
    }

    /// Scan another folder, and pick up what is in it.
    fn add_dir(&mut self, dir: PathBuf) {
        if !dir.is_dir() {
            eprintln!("omaread: {} is not a folder", dir.display());
        } else {
            let mut dirs = library::folders();
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
            if let Err(e) = library::set_folders(&dirs) {
                eprintln!("omaread: {e}");
            }
            self.rescan();
        }
        if self.menu.is_some() {
            self.build_menu();
        }
    }

    /// Open a book from anywhere on disk: picked, or dropped on the window.
    fn open_file(&mut self, path: &Path) {
        self.save_position();
        self.menu = None;
        self.open_path(path, 0);
    }

    /// Ask the desktop for a book to open, or a folder to scan. Off the
    /// window's thread: the dialog stays up for as long as the reader likes.
    fn pick(&mut self, folder: bool) {
        if self.picking.is_some() {
            return;
        }
        let title = i18n::tr!(if folder { "picker-folder" } else { "picker-book" });
        let (tx, rx) = channel();
        let window = self.window.clone();
        std::thread::spawn(move || {
            let _ = tx.send(portal::pick(folder, &title));
            if let Some(w) = window {
                w.request_redraw();
            }
        });
        self.picking = Some((folder, rx));
    }

    /// Act on what `pick` brought back. With no portal to ask, a folder can
    /// still be typed into the panel.
    fn take_picked(&mut self) {
        let Some((folder, rx)) = &self.picking else { return };
        let folder = *folder;
        let got = match rx.try_recv() {
            Ok(got) => got,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err("file portal: no answer".into()),
        };
        self.picking = None;
        match got {
            Ok(Some(dir)) if folder => self.add_dir(dir),
            Ok(Some(book)) => self.open_file(&book),
            Ok(None) => {}
            Err(e) => {
                eprintln!("omaread: {e}");
                if folder && self.menu.is_some() {
                    self.adding_folder = Some(String::new());
                    self.build_menu();
                }
            }
        }
    }

    /// Follow whatever `<a>` is under the pointer. Returns true when it dealt
    /// with the press, so the page does not also start a selection.
    ///
    /// A footnote or a bibliography entry opens where you are; anything longer
    /// is a place, so go there — and remember where you were.
    fn link_action(&mut self) -> bool {
        if self.view != View::Reading {
            return false;
        }
        // A popup takes the next click, whatever it lands on.
        if self.popup.take().is_some() {
            self.leaving = None;
            self.request_redraw();
            return true;
        }
        let href = {
            let Some(ch) = self.chapter.as_ref() else { return false };
            let (fx, fy) = self.pointer_in_flow();
            match href_at(&ch.doc, fx, fy) {
                Some(h) => h,
                None => return false,
            }
        };
        // Off-book links (§5): show where it really goes, and leave only on
        // Enter. Web and mail only; a file:// or a custom scheme hands the
        // desktop more than a book should.
        if ["http://", "https://", "mailto:"].iter().any(|p| href.starts_with(p)) {
            self.open_note(&i18n::tr!("link-leave", url = href.as_str()));
            self.leaving = Some(href);
            return true;
        }
        if href.contains("://") {
            eprintln!("omaread: external link {href} (not opened)");
            return true;
        }
        let Some((spine, frag)) = self.resolve_href(&href) else { return false };

        if let Some(text) = self.note_at(spine, frag.as_deref()) {
            self.open_note(&text);
            if self.note_fits() {
                return true;
            }
            // Taller than the window: it is read where it lives.
            self.popup = None;
        }

        self.back = Some((self.index, self.page, self.here_label()));
        self.hud_doc = None;
        self.go_to(spine, frag.as_deref());
        true
    }

    /// Split a book-relative href into the spine item it names and its fragment.
    fn resolve_href(&self, href: &str) -> Option<(usize, Option<String>)> {
        let (path, frag) = match href.split_once('#') {
            Some((p, f)) => (p, Some(f.to_string())),
            None => (href, None),
        };
        let book = self.book.as_ref()?;
        if path.is_empty() {
            return Some((self.index, frag));
        }
        // Relative to the chapter it was found in, and normalised the same way
        // the resource provider normalises: same input, same answer.
        let here = book.chapter_href(self.index).unwrap_or_default();
        let dir = here.rsplit_once('/').map_or("", |(d, _)| d);
        let target = net::in_archive_path(&format!("{dir}/{path}"))?;
        (0..book.chapter_count())
            .find(|&i| {
                book.chapter_href(i).and_then(net::in_archive_path).as_deref() == Some(&target)
            })
            .map(|i| (i, frag))
    }

    /// The text of a link target, when the target is a note rather than a place.
    ///
    /// A footnote, an endnote and a bibliography entry are all a short block of
    /// text; a section is a heading, a wrapper, or pages of prose. That is the
    /// whole test — the `epub:type="noteref"` a spec would use is carried by 4
    /// books of 60 in the real library, so it cannot be the rule.
    fn note_at(&self, spine: usize, frag: Option<&str>) -> Option<String> {
        let frag = frag?;
        let owned;
        let ch = match spine == self.index {
            true => self.chapter.as_ref()?,
            // Notes usually live in their own spine item, so lay it out — a
            // chapter is 20–40ms and this is a click, not a frame.
            false => {
                let book = self.book.clone()?;
                let cb = self.callback();
                owned = chapter::load(
                    &book,
                    spine,
                    &self.style,
                    self.viewport(),
                    self.page_height(),
                    self.hyphenator.as_ref(),
                    cb,
                )
                .ok()?;
                &owned
            }
        };
        let node = find_by_attr(ch.dom(), 0, "id", frag)?;
        let node = note_of_marker(ch.dom(), node);
        let tag = chapter::tag_of(ch.dom(), node)?;
        let text = chapter::text_of(ch.dom(), node);
        (is_note(&tag, &text) && !chapter::has_heading(ch.dom(), node)).then_some(text)
    }

    /// Does the note popup show the whole note? A long one is still a note, as
    /// long as it can be read without scrolling.
    fn note_fits(&self) -> bool {
        let Some(p) = self.popup.as_ref() else { return false };
        let win_h = self.size.1 as f32 / self.ui_scale();
        find_by_attr(p.dom(), 0, "class", "note")
            .and_then(|n| chapter::node_rect(p.dom(), n))
            .is_some_and(|(_, y, _, h)| y + h <= win_h)
    }

    /// F1 anywhere; `?` wherever it is not text being typed — the library's
    /// search, a tag, a folder path, the book search, a note.
    fn wants_help(&self, key: &Key) -> bool {
        match key {
            Key::Named(NamedKey::F1) => true,
            Key::Character(c) if c.as_str() == "?" => match self.view {
                View::Library => {
                    self.query.is_empty() && self.tagging.is_none() && self.adding_folder.is_none()
                }
                View::Reading => true,
                View::Toc => self.toc_mode != TocMode::Search && self.noting.is_none(),
            },
            _ => false,
        }
    }

    /// The keys overlay. Keys are not translated; what they do is.
    fn open_help(&mut self) {
        use i18n::tr;
        let mut rows: Vec<(&str, String)> = match self.view {
            View::Library => vec![
                ("← → ↑ ↓", tr!("help-move")),
                ("Enter", tr!("help-open")),
                ("a–z", tr!("help-search")),
                ("Esc", tr!("help-clear-quit")),
                ("PgUp PgDn", tr!("help-page")),
                ("Tab", tr!("help-sort")),
                ("F2", tr!("help-tag")),
                ("F3", tr!("help-open-file")),
                ("F5", tr!("help-rescan")),
                ("F10", tr!("help-settings")),
            ],
            View::Reading => vec![
                ("← → Space", tr!("help-turn")),
                ("↑ ↓", tr!("help-chapter")),
                ("g", tr!("help-goto")),
                ("Tab", tr!("help-contents")),
                ("/", tr!("help-find")),
                ("m", tr!("help-marks")),
                ("b", tr!("help-bookmark")),
                ("h", tr!("help-highlight")),
                ("y", tr!("help-copy")),
                ("t", tr!("help-theme")),
                ("+ -", tr!("help-size")),
                ("c", tr!("help-columns")),
                ("F10", tr!("help-menu")),
                ("l Esc", tr!("help-library")),
                ("q", tr!("help-quit")),
            ],
            View::Toc => {
                let mut rows = vec![
                    ("↑ ↓", tr!("help-move")),
                    ("Enter", tr!("help-go")),
                    ("Tab Esc", tr!("help-back")),
                ];
                if self.toc_mode == TocMode::Marks {
                    rows.push(("n", tr!("help-note")));
                    rows.push(("x", tr!("help-delete")));
                    rows.push(("e", tr!("help-export")));
                }
                if self.toc_mode != TocMode::Search {
                    rows.push(("l", tr!("help-library")));
                    rows.push(("q", tr!("help-quit")));
                }
                rows
            }
        };
        rows.push(("? F1", tr!("help-help")));
        let vp = chapter::viewport(self.size.0, self.size.1, self.ui_scale(), self.style.dark());
        self.popup = chapter::layout_document(
            hud::help_html(&rows),
            hud::help_stylesheet(&self.chrome()),
            None,
            vp,
            self.page_height(),
        );
        self.request_redraw();
    }

    fn open_note(&mut self, text: &str) {
        let vp = chapter::viewport(self.size.0, self.size.1, self.ui_scale(), self.style.dark());
        self.popup = chapter::layout_document(
            hud::note_html(text),
            hud::note_stylesheet(&self.chrome()),
            None,
            vp,
            self.page_height(),
        );
        self.request_redraw();
    }

    /// Go to a spine item, landing on a fragment's page when there is one.
    fn go_to(&mut self, spine: usize, frag: Option<&str>) {
        if spine != self.index {
            self.load_chapter(spine);
        }
        self.page = match frag
            .and_then(|f| {
                let ch = self.chapter.as_ref()?;
                let node = find_by_attr(ch.dom(), 0, "id", f)?;
                Some(ch.pages.page_containing(chapter::node_top(ch.dom(), node)?))
            }) {
            Some(page) => page,
            None if spine != self.index => 0,
            None => self.page,
        };
        self.resume_page = self.page;
        self.save_position();
        self.request_redraw();
    }

    /// Back to where the last link was followed from.
    fn go_back(&mut self) {
        let Some((spine, page, _)) = self.back.take() else { return };
        if spine != self.index {
            self.load_chapter(spine);
        }
        self.page = page.min(self.page_count().saturating_sub(1));
        self.resume_page = self.page;
        self.hud_doc = None;
        self.save_position();
        self.request_redraw();
    }

    /// Open the go-to bar where the reader is. Asking where page 200 is is
    /// asking for page numbers, so the book is measured as the readout's click
    /// would; until that lands the bar works in percentages.
    fn open_goto(&mut self) {
        self.measure_book();
        let here = match self.measured_pages() {
            Some(pages) => {
                let total = pages.iter().sum::<usize>();
                (self.book_page().0 - 1) as f32 / total.saturating_sub(1).max(1) as f32
            }
            None => self.progress(),
        };
        self.goto = Some((here, String::new()));
        self.menu = None;
        self.hud_shown = true;
        self.request_redraw();
    }

    /// Where a fraction of the book is: the chapter, and either the page in it
    /// (measured) or how far through it the bytes say (not).
    fn goto_place(&self, at: f32) -> Option<(usize, Result<usize, f32>)> {
        if let Some(pages) = self.measured_pages() {
            let total = pages.iter().sum::<usize>();
            let mut n = (at * total.saturating_sub(1) as f32).round() as usize;
            for (spine, &count) in pages.iter().enumerate() {
                if n < count {
                    return Some((spine, Ok(n)));
                }
                n -= count;
            }
            return Some((pages.len().checked_sub(1)?, Ok(usize::MAX)));
        }
        let (spine, within) = self.book.as_ref()?.locate(at);
        Some((spine, Err(within)))
    }

    /// What the go-to bar says it points at: the page or the percentage, and
    /// the chapter it is in.
    fn goto_label(&self, at: f32) -> String {
        let (Some(book), Some((spine, _))) = (&self.book, self.goto_place(at)) else {
            return String::new();
        };
        let place = match self.measured_pages() {
            Some(pages) => {
                let total = pages.iter().sum::<usize>().max(1);
                let page = (at * (total - 1) as f32).round() as usize + 1;
                i18n::tr!("hud-page-of", page = page, total = total)
            }
            None => format!("{}%", (at * 100.0).round() as u8),
        };
        // The track needs the rest of the bar, so a long chapter name gives way.
        let mut name = chapter_name(&book.toc, spine);
        if let Some((cut, _)) = name.char_indices().nth(40) {
            name = format!("{}…", name[..cut].trim_end());
        }
        format!("{place} · {name}")
    }

    /// Go where the bar points, leaving the way back on the bar.
    fn goto_jump(&mut self) {
        let Some((at, _)) = self.goto.take() else { return };
        let Some((spine, place)) = self.goto_place(at) else { return };
        if self.chapter.is_some() {
            self.back = Some((self.index, self.page, self.here_label()));
        }
        if spine != self.index || self.chapter.is_none() {
            self.load_chapter(spine);
        }
        self.page = match (place, &self.chapter) {
            (Ok(page), _) => page,
            (Err(within), Some(ch)) => ch.pages.page_containing(within * ch.pages.content_height),
            (Err(_), None) => 0,
        }
        .min(self.page_count().saturating_sub(1));
        self.resume_page = self.page;
        self.hud_doc = None;
        self.poke_hud();
        self.save_position();
        self.request_redraw();
    }

    /// The go-to bar takes its own keys; any other closes it, and Escape or
    /// `g` do nothing else.
    fn goto_key(&mut self, key: &Key) -> bool {
        let total = self.measured_pages().map(|p| p.iter().sum::<usize>());
        let Some((mut at, mut typed)) = self.goto.take() else { return false };
        match key {
            Key::Named(NamedKey::Enter) => {
                self.goto = Some((at, typed));
                self.goto_jump();
                return true;
            }
            Key::Named(NamedKey::Escape) => {}
            Key::Character(c) if c.as_str() == "g" => {}
            Key::Named(named) => {
                at = match named {
                    NamedKey::ArrowLeft => at - 0.01,
                    NamedKey::ArrowRight => at + 0.01,
                    NamedKey::PageUp => at - 0.1,
                    NamedKey::PageDown => at + 0.1,
                    NamedKey::Home => 0.0,
                    NamedKey::End => 1.0,
                    NamedKey::Backspace => {
                        typed.pop();
                        goto_typed(&typed, total).unwrap_or(at)
                    }
                    _ => {
                        self.poke_hud();
                        self.request_redraw();
                        return false;
                    }
                };
                if *named != NamedKey::Backspace {
                    typed.clear();
                }
                self.goto = Some((at.clamp(0.0, 1.0), typed));
            }
            Key::Character(c) if c.chars().all(|c| c.is_ascii_digit() || c == '%') => {
                typed.push_str(c);
                at = goto_typed(&typed, total).unwrap_or(at);
                self.goto = Some((at, typed));
            }
            _ => {
                self.poke_hud();
                self.request_redraw();
                return false;
            }
        }
        // Closed: the HUD goes back to leaving on its own.
        if self.goto.is_none() {
            self.poke_hud();
        }
        self.request_redraw();
        true
    }

    /// Move the go-to bar to the pointer, over its track.
    fn scrub_to_pointer(&mut self) {
        let Some(hud) = &self.hud_doc else { return };
        let Some((x, _, w, _)) = find_by_attr(hud.dom(), 0, "data-hud", "scrub")
            .and_then(|n| chapter::node_rect(hud.dom(), n))
        else {
            return;
        };
        let at = ((self.pointer_in_ui().0 - x) / w.max(1.0)).clamp(0.0, 1.0);
        if let Some(goto) = self.goto.as_mut() {
            *goto = (at, String::new());
            self.request_redraw();
        }
    }

    /// Where the reader is, worded for the way-back button. Always about the
    /// whole book: a chapter page number beside a whole-book percentage reads as
    /// a contradiction — "back to page 5" from 32% of the way in.
    fn here_label(&self) -> String {
        match self.measured_pages() {
            Some(_) => i18n::tr!("hud-back-to-page", page = self.book_page().0),
            // Not measured, so there is no honest page number for the book —
            // the readout says a percentage here too.
            None => i18n::tr!("hud-back-to-percent", percent = (self.progress() * 100.0).round() as u8),
        }
    }

    /// What the foot of the page says.
    ///
    /// Both readings are about the whole book. An unmeasured layout used to fall
    /// back to the chapter's own numbering — true, but "page 3 of 8" beside a
    /// book you are 37% through reads as a bug, and two columns hit it every
    /// time because halving the column width is a different layout.
    fn readout(&self) -> String {
        match (self.show_page, self.measured_pages().is_some()) {
            (true, true) => {
                let (page, total) = self.book_page();
                i18n::tr!("hud-page-of", page = page, total = total)
            }
            _ => format!("{}%", (self.progress() * 100.0).round() as u8),
        }
    }

    /// Page number across the whole book, not the chapter.
    ///
    /// The current chapter is measured — it is laid out — so its pages-per-byte
    /// converts the book's byte length into a page count. Chapters are only ever
    /// paginated on demand, and doing every one of a 131-chapter book to get an
    /// exact total would cost seconds on open.
    ///
    /// ponytail: an estimate, so the total drifts a few percent as you move
    /// between chapters of different density. Cache measured page counts per
    /// (chapter, font size, column width) if it ever needs to be exact.
    fn book_page(&self) -> (usize, usize) {
        match self.measured_pages() {
            Some(per_chapter) => {
                let before: usize = per_chapter.iter().take(self.index).sum();
                let total: usize = per_chapter.iter().sum();
                ((before + self.page + 1).min(total.max(1)), total.max(1))
            }
            // Not measured at this layout: the chapter's own numbering, which is
            // true, rather than an estimate that is not.
            None => (self.page + 1, self.page_count()),
        }
    }

    /// The cached counts, but only if they were measured at the layout now on
    /// screen — pagination depends on font size and column width.
    fn measured_pages(&self) -> Option<&Vec<usize>> {
        (self.pages_layout == self.layout_key()).then_some(self.chapter_pages.as_ref()?)
    }

    fn layout_key(&self) -> String {
        format!("{:.0}x{}", self.style.font_px(), self.column_width())
    }

    /// Measure every chapter so the readout can give a real page number.
    ///
    /// Cached in the database per book and layout, so this runs once: three
    /// seconds for a 131-chapter book, and instant every time after. Called from
    /// the click that asks for page numbers, never from a redraw.
    ///
    /// It runs on its own thread: every chapter is laid out, images and all, and
    /// on the window's thread that held the page still for as long. Until the
    /// counts arrive (`take_measured`) the readout gives the chapter's own
    /// numbering, as it does for any layout not measured yet.
    fn measure_book(&mut self) {
        let key = self.layout_key();
        if self.pages_layout == key && self.chapter_pages.is_some() {
            return;
        }
        let Some(book) = self.book.clone() else { return };
        let count = book.chapter_count();

        if let Some(cached) = self
            .db
            .as_ref()
            .and_then(|d| d.lock().ok())
            .and_then(|d| d.pagination(&self.hash, &key, count))
        {
            self.chapter_pages = Some(cached);
            self.pages_layout = key;
            return;
        }

        if self.measuring.as_ref().is_some_and(|(h, k, _)| *h == self.hash && *k == key) {
            return;
        }

        println!("omaread: measuring {count} chapters for page numbers…");
        let (tx, rx) = channel();
        let (style, width, height, scale, page_h) =
            (self.style.clone(), self.column_width(), self.size.1, self.scale, self.page_height());
        let (window, at) = (self.window.clone(), key.clone());
        std::thread::spawn(move || {
            let started = Instant::now();
            let hyphenator = hyphen::Hyphenator_::for_language(&book.language);
            let pages =
                chapter::page_counts(&book, &style, hyphenator.as_ref(), width, height, scale, page_h);
            println!(
                "omaread: {} pages at {at} ({:.1}s)",
                pages.iter().sum::<usize>(),
                started.elapsed().as_secs_f32()
            );
            let _ = tx.send(pages);
            if let Some(w) = window {
                w.request_redraw();
            }
        });
        self.measuring = Some((self.hash.clone(), key, rx));
    }

    /// Pick up page counts `measure_book` finished, store them, and use them if
    /// the book and layout they were measured for are still the ones on screen.
    fn take_measured(&mut self) {
        let Some((hash, key, rx)) = &self.measuring else { return };
        let pages = match rx.try_recv() {
            Ok(pages) => pages,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            // The thread died: nothing is coming. Asking again starts over.
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.measuring = None;
                return;
            }
        };
        if let Some(db) = self.db.as_ref().and_then(|d| d.lock().ok()) {
            if let Err(e) = db.save_pagination(hash, key, &pages) {
                eprintln!("omaread: could not save page counts: {e}");
            }
        }
        if *hash == self.hash && *key == self.layout_key() {
            self.chapter_pages = Some(pages);
            self.pages_layout = key.clone();
            self.request_redraw();
        }
        self.measuring = None;
    }

    /// The stored highlight containing a flow position, if any.
    fn mark_at(&self, flow: (f32, f32)) -> Option<i64> {
        let ch = self.chapter.as_ref()?;
        let db = self.db.as_ref()?.lock().ok()?;
        db.marks_in(&self.hash, self.index).into_iter().find_map(|m| {
            let c = cfi::Cfi::parse(&m.cfi)?;
            let node = cfi::resolve(ch.dom(), &c)?;
            let rects = chapter::highlight_rects(ch.dom(), node, c.offset?, m.length);
            let inside = rects.iter().any(|&(x0, y0, x1, y1)| {
                flow.0 >= x0 && flow.0 <= x1 && flow.1 >= y0 && flow.1 <= y1
            });
            inside.then_some(m.id)
        })
    }

    fn remove_selected_mark(&mut self) {
        let Some(id) = self.sel_mark.take() else { return };
        if let Some(db) = self.db.as_ref().and_then(|d| d.lock().ok()) {
            match db.remove_mark(id) {
                Ok(()) => println!("omaread: highlight removed"),
                Err(e) => eprintln!("omaread: could not remove: {e}"),
            }
        }
        self.sel = None;
        self.request_redraw();
    }

    /// Where each HUD icon goes, as `(icon, rect)` in CSS pixels. The HUD
    /// reserves an empty box per control and the window fills it in.
    fn hud_icons(&self) -> Vec<(paint::Icon, (f32, f32, f32, f32))> {
        let Some(hud) = &self.hud_doc else { return Vec::new() };
        [
            ("bookmark", paint::Icon::Bookmark),
            ("contents", paint::Icon::Contents),
            ("highlight", paint::Icon::Highlight),
            ("back", paint::Icon::Back),
        ]
        .into_iter()
        .filter_map(|(name, icon)| {
            let node = find_by_attr(hud.dom(), 0, "data-icon", name)?;
            Some((icon, chapter::node_rect(hud.dom(), node)?))
        })
        .collect()
    }

    /// A control in the HUD under the pointer, if any. Returns true when it
    /// handled the click, so the page does not also start a selection.
    /// The control under the pointer, by its `data-hud` name.
    ///
    /// The HUD is laid out at window size and painted unscrolled, so screen CSS
    /// pixels are its own coordinates.
    fn hud_under_pointer(&self) -> Option<String> {
        if !self.hud_shown || self.view != View::Reading {
            return None;
        }
        let hud = self.hud_doc.as_ref()?;
        let (x, y) = self.pointer_in_ui();
        let hit = hud.doc.hit(x, y)?;
        ancestor_attr(hud.dom(), hit.node_id, "data-hud")
    }

    fn hud_action(&mut self) -> bool {
        let Some(what) = self.hud_under_pointer() else { return false };
        self.hud_do(&what)
    }

    /// Act on a HUD control by name: a click, or a screen reader's request.
    fn hud_do(&mut self, what: &str) -> bool {
        // Anything else on the bars is done with the go-to bar.
        if what != "scrub" {
            self.goto = None;
        }
        match what {
            "scrub" => {
                self.scrubbing = true;
                self.scrub_to_pointer();
            }
            "bookmark" => self.toggle_bookmark(),
            "contents" => {
                self.toc_mode = TocMode::Contents;
                self.open_toc();
            }
            "back" => self.go_back(),
            "highlight" => drop(self.highlight_selection()),
            "unhighlight" => self.remove_selected_mark(),
            "library" => {
                self.to_library();
                return true;
            }
            "smaller" => self.set_font_scale(self.style.scale - 0.1),
            "bigger" => self.set_font_scale(self.style.scale + 0.1),
            "columns" => self.toggle_columns(),
            "readout" => {
                self.show_page = !self.show_page;
                if self.show_page {
                    self.measure_book();
                }
            }
            _ => return false,
        }
        // Acting on the HUD is using the HUD, so keep it up.
        self.hud_doc = None;
        self.poke_hud();
        self.request_redraw();
        true
    }

    /// Font size is a property of the reader, not of the book, so it is stored
    /// once and applies to every book from then on.
    fn set_font_scale(&mut self, scale: f32) {
        let scale = scale.clamp(0.8, 1.6);
        if (scale - self.style.scale).abs() < f32::EPSILON {
            return;
        }
        self.style.scale = scale;
        style::set_setting("font-scale", &format!("{scale:.2}"));
        println!("omaread: {:.0}px", self.style.font_px());
        self.restyle();
        if self.show_page {
            self.measure_book();
        }
    }

    fn toggle_columns(&mut self) {
        self.columns = if self.columns == 2 { 1 } else { 2 };
        let got = self.effective_columns();
        println!(
            "omaread: {got} column{}{}",
            plural(got),
            match self.columns == 2 && got == 1 {
                true => " — window too narrow for two",
                false => "",
            }
        );
        // The document is laid out at column width, so the flow has to be
        // re-measured — and page numbers are counted per layout, so they are
        // too. Only on a deliberate change: a window drag is a layout per pixel.
        self.relayout();
        if self.show_page {
            self.measure_book();
        }
        // Asked for two and got one: say why, or the key looks broken. Only
        // here, on the key; a window dragged narrower falls back silently.
        if self.columns == 2 && got == 1 {
            self.open_note(&i18n::tr!("columns-too-narrow"));
        }
        self.request_redraw();
    }

    /// How far through the book the current page is, 0.0..=1.0.
    fn progress(&self) -> f32 {
        let Some(book) = &self.book else { return 0.0 };
        let within = match &self.chapter {
            Some(ch) if ch.pages.content_height > 0.0 => {
                ch.pages.top_of(self.page) / ch.pages.content_height
            }
            _ => 0.0,
        };
        book.progress(self.index, within)
    }

    /// Wake the HUD and start its clock. Pointer motion is the gesture.
    fn poke_hud(&mut self) {
        if self.view != View::Reading {
            return;
        }
        self.hud_until = Some(Instant::now() + HUD_LINGER);
        if !self.hud_shown {
            self.hud_shown = true;
            self.request_redraw();
        }
    }

    fn to_library(&mut self) {
        self.save_position();
        if let Some(w) = &self.window {
            w.set_title("Omaread");
        }
        self.reload_rows();
        // `page` is one field for all three views, so coming back from page 47
        // of a book landed on page 47 of the shelf. The shelf starts at the top,
        // on the book just closed — which has usually moved to the front of a
        // Recent sort, so the old index pointed at somebody else.
        self.selected = self
            .rows
            .iter()
            .position(|r| r.hash == self.hash)
            .unwrap_or(0);
        self.page = 0;
        self.view = View::Library;
        self.request_redraw();
    }

    fn chrome(&self) -> Chrome {
        self.style.chrome()
    }

    /// The document the current view paints. Every view is one.
    fn doc(&self) -> Option<&Chapter> {
        match self.view {
            View::Library => self.lib_doc.as_ref(),
            View::Toc => self.toc_doc.as_ref(),
            View::Reading => self.chapter.as_ref(),
        }
    }

    fn doc_mut(&mut self) -> Option<&mut Chapter> {
        match self.view {
            View::Library => self.lib_doc.as_mut(),
            View::Toc => self.toc_doc.as_mut(),
            View::Reading => self.chapter.as_mut(),
        }
    }

    /// Lay out the contents. Cheap next to a chapter, so it is rebuilt on every
    /// open and every selection move rather than diffed — the same bargain the
    /// library grid makes.
    fn build_toc(&mut self) {
        if self.book.is_none() {
            return;
        }
        let entries = self.toc_entries();
        let (heading, subtitle) = self.toc_heading(entries.len());
        let html = toc::html(&entries, &heading, &subtitle, self.index, self.toc_sel);
        let ua = toc::stylesheet(&self.chrome());
        self.toc_doc =
            chapter::layout_document(html, ua, None, self.viewport(), self.page_height());
    }

    /// What the contents view is called, and the line under it, for `n` entries.
    fn toc_heading(&self, n: usize) -> (String, String) {
        let title = self.book.as_ref().map(|b| b.title.clone()).unwrap_or_default();
        match self.toc_mode {
            TocMode::Search => (
                i18n::tr!("toc-search"),
                i18n::tr!("toc-search-hits", query = self.find.as_str(), n = n),
            ),
            TocMode::Marks => (
                i18n::tr!("toc-marks"),
                match &self.noting {
                    Some(_) => i18n::tr!("toc-note-typing", note = self.note.as_str()),
                    None => i18n::tr!("toc-marks-count", n = n),
                },
            ),
            TocMode::Contents => (i18n::tr!("toc-contents"), title),
        }
    }

    /// Hand the screen reader the current tree, if one is listening.
    fn update_a11y(&mut self) {
        if let Some(mut adapter) = self.a11y.take() {
            adapter.update_if_active(|| self.a11y_tree());
            self.a11y = Some(adapter);
        }
    }

    /// The window as a screen reader sees it (`a11y.rs`).
    fn a11y_tree(&self) -> accesskit::TreeUpdate {
        use a11y::Target as T;
        use accesskit::{Role, Toggled};
        let title = match (&self.book, self.view) {
            (Some(b), View::Reading | View::Toc) => b.title.clone(),
            _ => "Omaread".to_string(),
        };
        let mut t = a11y::Tree::new(&title);

        match self.view {
            View::Library => {
                let value = match &self.tagging {
                    Some(tag) => format!("#{tag}"),
                    None => self.query.clone(),
                };
                t.add(T::Root, T::Search, Role::SearchInput, &i18n::tr!("search-hint")).set_value(value);
                t.add(T::Root, T::Gear, Role::Button, &i18n::tr!("a11y-settings"));
                let count = i18n::tr!("books-count", n = self.rows.len());
                t.add(T::Root, T::List, Role::List, &i18n::tr!("library-title", heading = count));
                for (i, b) in self.rows.iter().enumerate() {
                    let by = match b.missing {
                        true => i18n::tr!("library-file-missing"),
                        false => b.author.clone(),
                    };
                    let label = match by.is_empty() {
                        true => b.title.clone(),
                        false => format!("{}, {by}", b.title),
                    };
                    t.add(T::List, T::Item(i), Role::ListItem, &label).set_selected(i == self.selected);
                }
                t.focus(match self.rows.is_empty() {
                    true => T::Search,
                    false => T::Item(self.selected),
                });
            }
            View::Reading => {
                t.add(T::Root, T::Toolbar, Role::Toolbar, "");
                for (i, name) in a11y::HUD.iter().enumerate() {
                    if let Some(label) = self.hud_label(name) {
                        t.add(T::Toolbar, T::Hud(i), Role::Button, &label);
                    }
                }
                // The page on screen, both columns of it. Its text is its name,
                // so moving focus to it on a turn is what reads the page out.
                if let Some(ch) = &self.chapter {
                    let last = (self.page + self.effective_columns() - 1)
                        .min(ch.pages.count().saturating_sub(1));
                    let from = ch.pages.top_of(self.page);
                    let to = ch.pages.top_of(last) + ch.pages.extent_of(last);
                    let blocks = chapter::page_text(ch.dom(), from, to);
                    let key = (self.index as u64) << 24 | self.page as u64;
                    let text: Vec<&str> = blocks.iter().map(|(_, s)| s.as_str()).collect();
                    t.add(T::Root, T::Page(key), Role::Document, &text.join("\n"))
                        .set_description(self.readout());
                    for (k, (level, s)) in blocks.iter().enumerate().take(u16::MAX as usize) {
                        let role = if *level > 0 { Role::Heading } else { Role::Paragraph };
                        let node = t.add(T::Page(key), T::Text(key, k as u16), role, s);
                        if *level > 0 {
                            node.set_level(*level as usize);
                        }
                    }
                    t.focus(T::Page(key));
                }
                if let Some(note) = &self.popup {
                    let text = chapter::text_of(note.dom(), 0);
                    let label = if text.is_empty() { i18n::tr!("a11y-figure") } else { text };
                    t.add(T::Root, T::Note, Role::Dialog, &label);
                    t.focus(T::Note);
                }
            }
            View::Toc => {
                let entries = self.toc_entries();
                let (heading, subtitle) = self.toc_heading(entries.len());
                t.add(T::Root, T::List, Role::List, &format!("{heading}, {subtitle}"));
                for (i, e) in entries.iter().enumerate() {
                    t.add(T::List, T::Item(i), Role::ListItem, &e.label).set_selected(i == self.toc_sel);
                }
                t.focus(T::Item(self.toc_sel));
                if self.noting.is_some() {
                    t.add(T::Root, T::Typing, Role::TextInput, &i18n::tr!("menu-add-note"))
                        .set_value(self.note.clone());
                    t.focus(T::Typing);
                }
            }
        }

        if self.menu.is_some() {
            let label = match self.view {
                View::Reading => i18n::tr!("a11y-actions"),
                _ => i18n::tr!("a11y-settings"),
            };
            t.add(T::Root, T::Menu, Role::Menu, &label);
            t.focus(T::Menu);
            let theme = format!("theme:{}", format!("{:?}", self.style.theme).to_lowercase());
            for (i, (set, label)) in self.menu_rows().iter().enumerate() {
                match set.starts_with("theme:") {
                    true => {
                        let on = if *set == theme { Toggled::True } else { Toggled::False };
                        t.add(T::Menu, T::MenuRow(i), Role::MenuItemRadio, label).set_toggled(on);
                    }
                    false => drop(t.add(T::Menu, T::MenuRow(i), Role::MenuItem, label)),
                }
                if self.hover_menu.as_ref() == Some(set) {
                    t.focus(T::MenuRow(i));
                }
            }
            if let Some(typed) = &self.adding_folder {
                t.add(T::Menu, T::Typing, Role::TextInput, &i18n::tr!("settings-add-folder"))
                    .set_value(typed.clone());
                t.focus(T::Typing);
            }
        }
        t.finish()
    }

    /// A HUD control's spoken name, or `None` when it is not on offer now.
    fn hud_label(&self, name: &str) -> Option<String> {
        Some(match name {
            "bookmark" => i18n::tr!("hud-bookmark"),
            "contents" => i18n::tr!("hud-contents"),
            "highlight" if self.sel_mark.is_none() => i18n::tr!("hud-highlight"),
            "unhighlight" if self.sel_mark.is_some() => i18n::tr!("menu-remove-highlight"),
            "smaller" => i18n::tr!("a11y-smaller"),
            "bigger" => i18n::tr!("a11y-bigger"),
            "columns" => i18n::tr!("a11y-columns", n = self.columns),
            "library" => i18n::tr!("hud-library"),
            "back" => self.back.as_ref()?.2.clone(),
            "readout" => self.readout(),
            _ => return None,
        })
    }

    /// A screen reader asked for something: a click, or focus on a row.
    fn a11y_action(&mut self, req: accesskit::ActionRequest) {
        use a11y::Target as T;
        let click = req.action == accesskit::Action::Click;
        match a11y::target(req.target_node) {
            Some(T::Hud(i)) if click => drop(a11y::HUD.get(i).map(|name| self.hud_do(name))),
            Some(T::MenuRow(i)) => {
                if let Some((set, _)) = self.menu_rows().into_iter().nth(i) {
                    match click {
                        true => drop(self.menu_do(set)),
                        false => self.hover_menu = Some(set),
                    }
                }
            }
            Some(T::Item(i)) => match self.view {
                View::Library => {
                    self.move_selection(i as isize - self.selected as isize);
                    if click {
                        self.open_selected();
                    }
                }
                View::Toc if click => self.open_toc_entry(i),
                View::Toc => self.move_toc_selection(i as isize - self.toc_sel as isize),
                View::Reading => {}
            },
            Some(T::Gear) if click && self.view == View::Library => {
                self.open_menu();
                self.hover_menu = None;
                self.step_menu(1);
            }
            _ => {}
        }
        self.request_redraw();
    }

    /// What the contents view lists: the book's navigation, or search hits.
    fn toc_entries(&self) -> Vec<book::TocEntry> {
        let Some(book) = &self.book else { return Vec::new() };
        match self.toc_mode {
            TocMode::Contents => return (*book.toc).clone(),
            TocMode::Marks => {
                return self
                    .marks
                    .iter()
                    .map(|m| {
                        let spine = cfi::Cfi::parse(&m.cfi).map_or(0, |c| c.spine);
                        let kind = match m.is_bookmark() {
                            true => i18n::tr!("mark-bookmark"),
                            false => i18n::tr!("mark-highlight"),
                        };
                        // A bookmark has no words of its own: it reads as where it is.
                        let text = match m.is_bookmark() {
                            true => self
                                .bookmark_places
                                .get(&m.id)
                                .cloned()
                                .unwrap_or_else(|| chapter_name(&book.toc, spine)),
                            false => m.text.clone(),
                        };
                        let body = match m.note.is_empty() {
                            true => text,
                            false => format!("{text} — {}", m.note),
                        };
                        book::TocEntry {
                            label: format!("{kind} · {body}"),
                            depth: 0,
                            spine,
                            fragment: None,
                            find: None,
                            cfi: Some(m.cfi.clone()),
                        }
                    })
                    .collect();
            }
            TocMode::Search => {}
        }
        let Some(fts) = search::fts_query(&self.find) else { return Vec::new() };
        let Some(db) = self.db.as_ref().and_then(|d| d.lock().ok()) else { return Vec::new() };

        db.search_in_book(&self.hash, &fts, 200)
            .into_iter()
            .map(|hit| {
                // Name the chapter from the book's own navigation when it has
                // one: "Chapter 4 · …the words…" beats a bare snippet.
                let where_ = chapter_name(&book.toc, hit.spine);
                book::TocEntry {
                    label: format!("{where_} · {}", hit.snippet),
                    depth: 0,
                    spine: hit.spine,
                    fragment: None,
                    find: Some(self.find.clone()),
                    cfi: None,
                }
            })
            .collect()
    }

    fn open_toc(&mut self) {
        let Some(toc) = self.book.as_ref().map(|b| b.toc.clone()) else { return };
        self.resume_page = self.page;
        // Open on the entry covering where you are, not at the top: in a long
        // book the useful part of the contents is the part you are in. A search
        // starts at the best hit, which is the first one.
        self.toc_sel = match self.toc_mode {
            TocMode::Contents => toc.iter().rposition(|e| e.spine <= self.index).unwrap_or(0),
            // A search starts at the best hit; marks are in reading order.
            _ => 0,
        };
        self.view = View::Toc;
        self.page = 0;
        self.build_toc();
        self.page = self.toc_doc.as_ref().and_then(|d| page_of_index(d, self.toc_sel)).unwrap_or(0);
        self.request_redraw();
    }

    fn close_toc(&mut self) {
        self.toc_mode = TocMode::Contents;
        self.noting = None;
        self.view = View::Reading;
        self.page = self.resume_page.min(self.page_count().saturating_sub(1));
        self.request_redraw();
    }

    /// Navigate to a contents entry, landing on its fragment's page when it has
    /// one — books that keep several chapters in one spine file would otherwise
    /// send every entry to page 1.
    fn open_toc_entry(&mut self, i: usize) {
        // The entry the list shows: in marks or search that is not the book's
        // contents, and indexing those sent a mark to whatever chapter shared
        // its position in the list.
        let Some(entry) = self.toc_entries().get(i).cloned() else { return };
        if self.chapter.is_some() {
            self.back = Some((self.index, self.resume_page, self.here_label()));
            self.hud_doc = None;
        }
        self.view = View::Reading;
        if entry.spine == self.index && self.chapter.is_some() {
            self.page = 0;
        } else {
            self.load_chapter(entry.spine);
        }
        // A stored mark knows its element, and a highlight its character.
        if let Some(raw) = &entry.cfi {
            match cfi::Cfi::parse(raw).and_then(|c| {
                let ch = self.chapter.as_ref()?;
                Some(ch.pages.page_containing(chapter::top_of_cfi(ch, &c)?))
            }) {
                Some(page) => self.page = page,
                None => eprintln!("omaread: {raw} no longer resolves"),
            }
        }
        // A hit knows its words, not its element: locate the text in the
        // chapter that is now laid out.
        if let Some(needle) = &entry.find {
            match self
                .chapter
                .as_ref()
                .and_then(|ch| chapter::node_containing_text(ch.dom(), needle))
                .and_then(|node| {
                    let ch = self.chapter.as_ref()?;
                    Some(ch.pages.page_containing(chapter::node_top(ch.dom(), node)?))
                }) {
                Some(page) => self.page = page,
                // Folding is close but not identical to FTS5's, so a miss is
                // possible; the chapter is still the right place to be.
                None => eprintln!("omaread: “{needle}” did not resolve to a page in this chapter"),
            }
        }
        if let Some(frag) = &entry.fragment {
            match self
                .chapter
                .as_ref()
                .and_then(|ch| find_by_attr(ch.dom(), 0, "id", frag))
                .and_then(|node| {
                    let ch = self.chapter.as_ref()?;
                    Some(ch.pages.page_containing(chapter::node_top(ch.dom(), node)?))
                }) {
                Some(page) => self.page = page,
                // A dangling fragment is the book's problem, not the reader's:
                // the chapter is still the right place to be.
                None => eprintln!("omaread: contents point at #{frag}, which is not in the chapter"),
            }
        }
        self.resume_page = self.page;
        self.save_position();
        self.request_redraw();
    }

    fn move_toc_selection(&mut self, by: isize) {
        let last = match self.toc_entries().len() {
            0 => return,
            n => n as isize - 1,
        };
        self.toc_sel = (self.toc_sel as isize + by).clamp(0, last) as usize;
        // Follow the selection onto its page.
        if let Some(page) = self.toc_doc.as_ref().and_then(|d| page_of_index(d, self.toc_sel)) {
            self.page = page;
        }
    }

    fn chapter_count(&self) -> usize {
        self.book.as_ref().map_or(0, |b| b.chapter_count())
    }

    fn callback(&self) -> SharedCallback<Resource> {
        Arc::new(ChannelCallback(self.net_tx.clone(), self.window.clone()))
    }

    /// How many columns actually paint.
    fn effective_columns(&self) -> usize {
        columns_for(
            self.columns,
            self.size.0 as f32 / self.scale,
            self.style.font_px(),
            self.view == View::Reading,
        )
    }

    /// Width of one column in physical pixels.
    fn column_width(&self) -> u32 {
        (self.size.0 / self.effective_columns() as u32).max(1)
    }

    /// The document is laid out at *column* width, not window width: in
    /// two-column mode both columns are views onto the same flow.
    fn viewport(&self) -> blitz_traits::shell::Viewport {
        chapter::viewport(
            self.column_width(),
            self.size.1,
            self.scale,
            self.style.dark(),
        )
    }

    /// The scale the chrome is laid out and painted at: the monitor's, times
    /// Omarchy's text size. Its CSS pixels are window pixels over this.
    fn ui_scale(&self) -> f32 {
        self.scale * self.ui
    }

    /// Forget the chrome laid out at the old scale. The HUD comes back on the
    /// next frame; a menu or a popup was placed for a window that is gone.
    /// Pick up the palette Omarchy rendered, if it changed.
    fn reread_palette(&mut self) {
        let omarchy = style::omarchy_chrome();
        if omarchy == self.style.omarchy {
            return;
        }
        self.style.omarchy = omarchy;
        // Every surface is styled from the palette when it is built.
        (self.lib_doc, self.toc_doc) = (None, None);
        self.drop_chrome();
        if self.style.theme == Theme::Omarchy {
            self.restyle();
        }
        self.request_redraw();
    }

    fn drop_chrome(&mut self) {
        self.hud_doc = None;
        self.menu = None;
        self.relay_popup();
    }

    /// The pointer in chrome CSS pixels.
    fn pointer_in_ui(&self) -> (f32, f32) {
        (self.cursor.0 / self.ui_scale(), self.cursor.1 / self.ui_scale())
    }

    /// Vertical margin above and below the page, in CSS pixels.
    fn page_margin(&self) -> f32 {
        match self.view {
            View::Reading => PAGE_MARGIN_EM * self.style.font_px(),
            View::Library | View::Toc => style::CHROME_MARGIN,
        }
    }

    /// Usable height of one page, in CSS pixels — the window less its margins.
    fn page_height(&self) -> f32 {
        (self.size.1 as f32 / self.scale - 2.0 * self.page_margin()).max(1.0)
    }

    /// Jump to the saved position, if it points into the chapter just loaded.
    fn restore_position(&mut self) {
        let Some(c) = self.pending.take() else { return };
        if c.spine != self.index {
            self.pending = Some(c);
            return;
        }
        let Some(ch) = &self.chapter else { return };
        // The character the page began on, when the saved position names one;
        // the paragraph's own top otherwise.
        match chapter::top_of_cfi(ch, &c) {
            Some(y) => {
                self.page = ch.pages.page_containing(y);
                println!("omaread: resumed at page {} of {}", self.page + 1, ch.pages.count());
            }
            None => eprintln!("omaread: saved position no longer resolves; starting at the top"),
        }
    }

    /// Anchor the current page to a CFI and store it.
    fn save_position(&self) {
        let (Some(db), Some(ch)) = (&self.db, &self.chapter) else { return };
        let Ok(db) = db.lock() else { return };
        let top = ch.pages.top_of(self.page);
        let Some(node) = chapter::node_at(ch.dom(), top) else { return };
        let Some(mut c) = cfi::of_node(ch.dom(), node, self.index) else { return };
        // Which character of it the page starts on. Without this the address is
        // the paragraph, and a paragraph three pages long resumes at its first.
        c.offset = chapter::char_at(ch.dom(), node, top);
        let title = self.book.as_ref().map(|b| b.title.as_str()).unwrap_or("");
        let progress = self.progress();
        if let Err(e) =
            db.save_progress(&self.hash, &self.path, title, &c.to_string(), progress)
        {
            eprintln!("omaread: could not save position: {e}");
        }
    }

    fn last_page_of_loaded(&self) -> usize {
        self.chapter.as_ref().map_or(0, |c| c.pages.count().saturating_sub(1))
    }

    fn page_count(&self) -> usize {
        self.doc().map_or(1, |c| c.pages.count())
    }

    /// Page a chrome document — library or contents. `turn` is the reading
    /// version: it crosses chapter boundaries and saves a position, neither of
    /// which means anything here.
    fn turn_view(&mut self, forward: bool) {
        let last = self.page_count().saturating_sub(1);
        let from = self.page;
        self.page = if forward {
            (self.page + 1).min(last)
        } else {
            self.page.saturating_sub(1)
        };
        // The library's selection rides along, to the same place on the new
        // page. Left behind, the rebuild snapped straight back to the page it
        // was on — PageDown and the wheel did nothing but rebuild twice.
        if self.view == View::Library
            && let Some(doc) = &self.lib_doc
        {
            self.selected =
                ride_along(self.selected, visible_cards(doc, from), visible_cards(doc, self.page));
        }
        self.request_redraw();
    }

    /// Load a chapter, skipping forward past any the engine chokes on.
    fn load_chapter(&mut self, index: usize) {
        self.load_chapter_at(index, false);
    }

    /// `backwards` opens the chapter on its last page, for turning back across a
    /// chapter boundary.
    fn load_chapter_at(&mut self, mut index: usize, backwards: bool) {
        let started = Instant::now();
        let Some(book) = self.book.clone() else { return };
        let count = book.chapter_count();
        while index < count {
            let vp = self.viewport();
            let ph = self.page_height();
            let cb = self.callback();
            match chapter::load(&book, index, &self.style, vp, ph, self.hyphenator.as_ref(), cb) {
                Ok(ch) => {
                    if std::env::var_os("OMAREAD_DEBUG_PAGES").is_some() {
                        let atoms = chapter::collect_atoms(ch.dom());
                        let mut bad = 0;
                        for (n, &t) in ch.pages.tops.iter().enumerate() {
                            if let Some(a) =
                                atoms.iter().find(|a| t > a.top && t < a.bottom)
                            {
                                bad += 1;
                                eprintln!(
                                    "  BAD page {n} top {t:.1} cuts {:?} {:.1}..{:.1}",
                                    a.kind, a.top, a.bottom
                                );
                            }
                        }
                        eprintln!(
                            "  PAGES {} tops={:?} atoms={} cutting={bad}",
                            ch.pages.count(),
                            ch.pages.tops.iter().map(|t| t.round()).collect::<Vec<_>>(),
                            atoms.len(),
                        );
                    }
                    if std::env::var_os("OMAREAD_DEBUG_LAYOUT").is_some() {
                        let vw = self.size.0 as f32 / self.scale;
                        for tag in ["html", "body", "div", "p"] {
                            if let Some((x, y, w, h)) = chapter::element_rect(ch.dom(), tag) {
                                eprintln!(
                                    "  LAYOUT <{tag}> x={x:.0} y={y:.0} w={w:.0} h={h:.0}                                      (window {vw:.0}px, left gap {x:.0}, right gap {:.0})",
                                    vw - (x + w)
                                );
                            }
                        }
                    }
                    println!(
                        "omaread: chapter {}/{} — {} pages, {} chars, {} lines",
                        index + 1,
                        count,
                        ch.pages.count(),
                        ch.text_len(),
                        ch.line_count(),
                    );
                    if std::env::var_os("OMAREAD_DEBUG_TIME").is_some() {
                        eprintln!(
                            "TIME chapter {index} load+paginate: {:.0}ms",
                            started.elapsed().as_secs_f32() * 1000.0
                        );
                    }
                    self.chapter = Some(ch);
                    self.index = index;
                    self.page = if backwards { self.last_page_of_loaded() } else { 0 };
                    self.restore_position();
                    return;
                }
                Err(e) => {
                    eprintln!("omaread: {e}");
                    if e.starts_with(chapter::PANICKED) {
                        self.engine_panicked();
                    }
                    index += 1;
                }
            }
        }
        eprintln!("omaread: no renderable chapter at or after {index}");
    }

    /// Turn one page, crossing chapter boundaries in either direction.
    fn turn(&mut self, forward: bool) {
        let step = self.effective_columns();
        let was = self.page;
        let into = if forward {
            let last = self.page_count().saturating_sub(1);
            if self.page + step <= last {
                self.page += step;
                None
            } else if self.index + 1 < self.chapter_count() {
                Some(self.index + 1)
            } else {
                return;
            }
        } else if self.page >= step {
            self.page -= step;
            None
        } else if self.index > 0 {
            Some(self.index - 1)
        } else {
            return;
        };
        self.slide_from = None;
        // Hold on to the chapter being left, so its page can slide off while
        // the next chapter's slides in.
        if let Some(index) = into {
            let from = self.chapter.take();
            self.load_chapter_at(index, !forward);
            match self.chapter.is_some() {
                true => self.slide_from = from,
                false => self.chapter = from,
            }
        }
        if into.is_none() || self.slide_from.is_some() {
            self.slide = Some(Slide { from: was, forward, at: Instant::now() });
        }
        // ponytail: a write per page turn. WAL makes it cheap; batch only if it
        // ever shows up in a profile.
        self.save_position();
        self.request_redraw();
    }

    fn turn_chapter(&mut self, forward: bool) {
        let next = if forward {
            self.index + 1
        } else {
            match self.index.checked_sub(1) {
                Some(i) => i,
                None => return,
            }
        };
        if next < self.chapter_count() {
            self.load_chapter(next);
            self.request_redraw();
        }
    }

    /// The stylesheet is baked in at construction, so a theme or size change
    /// means rebuilding the document.
    fn restyle(&mut self) {
        let Some(index) = self.chapter.as_ref().map(|c| c.index) else {
            return;
        };
        // Keep the reader's place across a re-flow by carrying the flow offset,
        // not the page number: a smaller font means more text per page.
        let anchor = self.chapter.as_ref().map_or(0.0, |c| c.pages.top_of(self.page));
        self.load_chapter(index);
        if let Some(ch) = &self.chapter {
            self.page = ch.pages.page_containing(anchor);
        }
        self.request_redraw();
    }

    /// A popup laid out for a window that is gone. A note is text, so it is
    /// laid out again at the new size — the fixed-layout notice arrives just as
    /// Hyprland rescales a new window, and was lost to that. A figure goes; it
    /// comes back on the next click.
    fn relay_popup(&mut self) {
        if self.popup.as_ref().is_some_and(|p| find_by_attr(p.dom(), 0, "data-help", "").is_some()) {
            return self.open_help();
        }
        let note = self.popup.take().map(|p| chapter::text_of(p.dom(), 0));
        if let Some(text) = note.filter(|t| !t.is_empty()) {
            self.open_note(&text);
        }
    }

    fn relayout(&mut self) {
        self.relay_popup();
        if self.view != View::Reading {
            let vp = self.viewport();
            let ph = self.page_height();
            if let Some(doc) = self.doc_mut() {
                chapter::relayout(doc, vp, ph);
                let last = doc.pages.count().saturating_sub(1);
                self.page = self.page.min(last);
            }
            // Re-paginating in place moves the page boundaries, so a resize can
            // put cards on screen that this document was never given covers for
            // — the newly revealed row came up bare. Rebuild when the set of
            // visible cards actually changes, which during a drag is a handful
            // of times rather than every pixel.
            if self.view == View::Library {
                let now = self.lib_doc.as_ref().map(|d| visible_cards(d, self.page));
                if now.is_some_and(|now| now != self.lib_covers) {
                    self.lib_doc = None;
                }
            }
            return;
        }
        let vp = self.viewport();
        let ph = self.page_height();
        let anchor = self.chapter.as_ref().map_or(0.0, |c| c.pages.top_of(self.page));
        let panicked = match &mut self.chapter {
            Some(ch) => !chapter::relayout(ch, vp, ph),
            None => return,
        };
        if panicked {
            eprintln!("omaread: engine panicked during relayout; reloading chapter");
            self.engine_panicked();
            let i = self.index;
            self.load_chapter(i);
            return;
        }
        if let Some(ch) = &self.chapter {
            self.page = ch.pages.page_containing(anchor);
        }
    }

    fn drain_resources(&mut self) {
        let pending: Vec<(usize, Resource)> = self.net_rx.try_iter().collect();
        if pending.is_empty() {
            return;
        }
        // Resources belong to whichever document asked for them: cover images
        // for the library, figures for a chapter. A cover decoded off-thread can
        // land after its grid was rebuilt; that document is gone, and its node
        // ids mean nothing in this one.
        let loaded = match self.doc_mut() {
            Some(doc) => {
                let id = doc.doc.id();
                let mine: Vec<Resource> =
                    pending.into_iter().filter(|(d, _)| *d == id).map(|(_, r)| r).collect();
                if mine.is_empty() {
                    return;
                }
                catch_unwind(AssertUnwindSafe(|| {
                    for resource in mine {
                        doc.doc.load_resource(resource);
                    }
                }))
                .is_ok()
            }
            None => return,
        };

        if loaded {
            self.relayout();
            self.request_redraw();
        } else {
            eprintln!("omaread: engine panicked loading a resource; ignoring it");
            if self.view == View::Reading {
                self.engine_panicked();
            }
        }
    }

    fn request_redraw(&self) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Put the surface back after the compositor refused a frame. Acquiring a
    /// surface texture that times out leaves the swapchain wedged: every later
    /// frame times out too, which is what a frozen window is. Reconfiguring it
    /// clears that; if a reconfigure was not enough, the surface gets built
    /// again from scratch. Retries slow down as they fail, so a screen that is
    /// off for an hour is not an hour of the GPU being asked every 16ms.
    fn recover(&mut self) {
        self.drops += 1;
        let (w, h) = self.size;
        if self.drops == 1 {
            self.renderer.set_size(w, h);
        } else if let Some(window) = self.window.clone() {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                self.renderer.suspend();
                self.renderer.resume(window, w, h);
            }));
        }
        self.retry_at = Some(Instant::now() + RETRY * self.drops.min(10));
    }

    fn redraw(&mut self) {
        let started = Instant::now();
        let (w, h) = self.size;
        let scale = self.scale as f64;
        let ui_scale = self.ui_scale() as f64;
        let page = self.page;
        let page_h = self.page_height();
        let margin = self.page_margin();
        let ground = match self.view {
            View::Reading => {
                let [r, g, b] = self.style.background_rgb();
                Color::from_rgb8(r, g, b)
            }
            _ => parse_hex(&self.chrome().bg),
        };

        match self.view {
            // A different page shows different cards, and only the cards on the
            // page carry covers.
            View::Library if self.lib_doc.is_none() || self.page != self.lib_page => {
                self.build_library();
                // A selection on another page has nothing to outline here. Only
                // possible under a sort that does not put the last-read book
                // first, so the second build is rare.
                match self.lib_doc.as_ref().and_then(|d| page_of_index(d, self.selected)) {
                    Some(p) if p != self.page => {
                        self.page = p;
                        self.build_library();
                    }
                    _ => {}
                }
            }
            View::Toc if self.toc_doc.is_none() => self.build_toc(),
            View::Reading if self.hud_shown => self.build_hud(),
            _ => {}
        }
        let showing_hud = self.hud_shown && self.view == View::Reading;
        let cols = self.effective_columns();
        // Both borrow self immutably, so they cannot wait until after the
        // disjoint-field destructure below.
        let ((highlights, noted), selection) = match self.view {
            View::Reading => (self.highlight_rects(), self.selection_rects()),
            _ => ((Vec::new(), Vec::new()), Vec::new()),
        };
        // The library's selection is painted, so arrow keys cost a frame instead
        // of a rebuild — a rebuild re-requests every cover.
        let card_rect = |index: usize| -> Option<(f32, f32, f32, f32)> {
            let doc = self.lib_doc.as_ref()?;
            let node = find_by_attr(doc.dom(), 0, "data-index", &index.to_string())?;
            let (x, y, w, h) = chapter::node_rect(doc.dom(), node)?;
            Some((x - CARD_RING, y - CARD_RING, w + 2.0 * CARD_RING, h + 2.0 * CARD_RING))
        };
        let (selected_card, hovered_card) = match self.view {
            View::Library => (
                card_rect(self.selected),
                self.hover_card.filter(|i| Some(*i) != Some(self.selected)).and_then(card_rect),
            ),
            _ => (None, None),
        };
        let icons = match (showing_hud, self.view) {
            (true, _) => self.hud_icons(),
            // The library's own control: the cog left of the search box.
            (false, View::Library) => self
                .lib_doc
                .as_ref()
                .and_then(|d| find_by_attr(d.dom(), 0, "data-icon", "gear"))
                .and_then(|n| {
                    let d = self.lib_doc.as_ref()?;
                    Some(vec![(paint::Icon::Gear, chapter::node_rect(d.dom(), n)?)])
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        let chrome = self.chrome();
        let ink = parse_hex(&chrome.fg);
        let hud_bookmarked = self.hud_bookmarked;
        let accent = parse_hex(&chrome.accent);

        // Disjoint field borrows: `render` takes the renderer mutably, the
        // closure needs the document mutably to set the page offset. That rules
        // out `doc_mut`, which borrows all of `self`.
        let hover_hud = self.hover_hud.clone();
        let hover_menu = self.hover_menu.clone();
        // How far through a page turn this frame is, eased out, and which page
        // is on its way off. Only in the reading view: the library rebuilds its
        // grid on a page turn to fetch that page's covers, so the outgoing page
        // would lose its covers halfway across.
        let slide = self.slide.filter(|_| self.view == View::Reading).and_then(|s| {
            let t = s.at.elapsed().as_secs_f32() / SLIDE.as_secs_f32();
            (t < 1.0).then(|| (s, 1.0 - (1.0 - t) * (1.0 - t)))
        });
        let App { renderer, chapter, slide_from, lib_doc, toc_doc, hud_doc, popup, menu, view, .. } = self;
        let hud = if showing_hud { hud_doc.as_mut() } else { None };
        let Some(ch) = (match view {
            View::Library => lib_doc.as_mut(),
            View::Toc => toc_doc.as_mut(),
            View::Reading => chapter.as_mut(),
        }) else {
            return;
        };
        if std::env::var_os("OMAREAD_DEBUG_PAINT").is_some() {
            eprintln!(
                "PAINT page {}/{} cols={cols} top={:.0} page_h={page_h:.0} win={w}x{h}",
                page + 1,
                ch.pages.count(),
                ch.pages.top_of(page),
            );
        }
        let count = ch.pages.count();
        // Read the flow slices before borrowing the document mutably: each
        // column is a different page of the same flow.
        let slices: Vec<(f32, f32)> = (0..cols)
            .map(|c| page + c)
            .map(|p| match p < count {
                true => ch.pages.slice(p),
                false => (0.0, 0.0),
            })
            .collect();
        // A table header repeated over each column that continues a table.
        let heads: Vec<Option<(f32, f32)>> = (0..cols).map(|c| ch.pages.head_of(page + c)).collect();
        // The page being left, in the same flow: a turn within one chapter is
        // two slices of one document, which is exactly what two columns are.
        // Across a boundary it is a slice of the chapter before.
        let (outgoing, outgoing_heads): (Vec<(f32, f32)>, Vec<Option<(f32, f32)>>) = match slide {
            Some((s, _)) => {
                let from = slide_from.as_ref().unwrap_or(&*ch);
                (0..cols)
                    .map(|c| s.from + c)
                    .map(|p| match p < from.pages.count() {
                        true => (from.pages.slice(p), from.pages.head_of(p)),
                        false => ((0.0, 0.0), None),
                    })
                    .unzip()
            }
            None => Default::default(),
        };
        let doc = &mut ch.doc;
        let frame = paint::Frame {
            width: w,
            height: h,
            scale,
            margin,
            page_height: page_h,
        };
        // The chrome is the same window at a bigger scale.
        let ui_frame = paint::Frame { scale: ui_scale, ..frame };
        let col_w = (w as f32 / scale as f32) / cols as f32;

        let mut painted = true;
        renderer.render(|scene| {
            // An engine panic while painting must not take the window with it.
            painted = catch_unwind(AssertUnwindSafe(|| {
                // Two columns are two pages of one flow side by side, not CSS
                // multicol: the same document painted twice at two scroll
                // offsets (CONTEXT.md §3).
                if let Some((s, t)) = slide {
                    // The old page walks off the way you turned, the new one
                    // follows it in. Both are the same document at two offsets.
                    let (out_x, in_x) = slide_offsets(s.forward, t, w as f32 / scale as f32);
                    paint::clear(scene, ground, &frame);
                    for (col, &(top, extent)) in outgoing.iter().enumerate() {
                        let x = out_x + col as f32 * col_w;
                        let doc = match slide_from.as_mut() {
                            Some(from) => &mut from.doc,
                            None => &mut *doc,
                        };
                        paint::column(scene, doc, top, extent, x, col_w, &frame, ground, false);
                        if let Some(head) = outgoing_heads[col] {
                            paint::head(scene, doc, head, x, col_w, &frame, ground);
                        }
                    }
                    for (col, &(top, extent)) in slices.iter().enumerate() {
                        let x = in_x + col as f32 * col_w;
                        paint::column(scene, doc, top, extent, x, col_w, &frame, ground, false);
                        if let Some(head) = heads[col] {
                            paint::head(scene, doc, head, x, col_w, &frame, ground);
                        }
                    }
                } else {
                for (col, &(top, extent)) in slices.iter().enumerate() {
                    let x = col as f32 * col_w;
                    if page + col >= count {
                        // Past the end of the chapter: clean paper, not whatever
                        // the previous frame left there.
                        paint::column(scene, doc, 0.0, 0.0, x, col_w, &frame, ground, col == 0);
                        continue;
                    }
                    paint::column(scene, doc, top, extent, x, col_w, &frame, ground, col == 0);
                    paint::bands(scene, &highlights, HIGHLIGHT, top, extent, x, &frame);
                    paint::bands(scene, &noted, NOTED, top, extent, x, &frame);
                    paint::bands(scene, &selection, accent.with_alpha(SELECTION_ALPHA), top, extent, x, &frame);
                    if let Some(head) = heads[col] {
                        paint::head(scene, doc, head, x, col_w, &frame, ground);
                    }
                }
                }
                // Before the selection's outline, so a card that is both
                // hovered and selected still reads as selected.
                if let Some((cx, cy, cw, chh)) = hovered_card {
                    let (top, _) = slices.first().copied().unwrap_or((0.0, 0.0));
                    paint::outline(
                        scene,
                        (cx, cy - top + margin, cw, chh),
                        ink.with_alpha(CARD_HOVER_ALPHA),
                        2.0,
                        scale,
                    );
                }
                if let Some((cx, cy, cw, chh)) = selected_card {
                    // Flow -> screen: the grid is a page like any other.
                    let (top, _) = slices.first().copied().unwrap_or((0.0, 0.0));
                    paint::outline(
                        scene,
                        (cx, cy - top + margin, cw, chh),
                        accent,
                        2.0,
                        scale,
                    );
                }
                if let Some(note) = popup.as_mut() {
                    paint::overlay(scene, &mut note.doc, &ui_frame);
                }
                // The library's cog, in flow coordinates like the selection
                // outline: the shelf is a page, so it carries the page margin.
                if matches!(view, View::Library) {
                    let (top, _) = slices.first().copied().unwrap_or((0.0, 0.0));
                    for &(icon, (cx, cy, cw, chh)) in &icons {
                        paint::icon(scene, icon, (cx, cy - top + margin, cw, chh), ink, scale);
                    }
                }
                if let Some(hud) = hud {
                    paint::overlay(scene, &mut hud.doc, &ui_frame);
                    for &(icon, rect) in &icons {
                        let on = icon == paint::Icon::Bookmark && hud_bookmarked;
                        paint::icon(scene, icon, rect, if on { accent } else { ink }, ui_scale);
                    }
                    // Over the bar, not under it: the bar's ground is opaque.
                    if let Some(rect) = hover_hud
                        .as_deref()
                        .and_then(|w| find_by_attr(hud.dom(), 0, "data-hud", w))
                        .and_then(|n| chapter::node_rect(hud.dom(), n))
                    {
                        paint::wash(scene, rect, ink.with_alpha(HOVER_FILL_ALPHA), ui_scale);
                        paint::outline(scene, rect, ink.with_alpha(HOVER_BORDER_ALPHA), 1.0, ui_scale);
                    }
                }
                // Last, so the context menu sits over the bars too.
                if let Some(m) = menu.as_mut() {
                    paint::overlay(scene, &mut m.doc, &ui_frame);
                    if let Some(rect) = hover_menu
                        .as_deref()
                        .and_then(|r| find_by_attr(m.dom(), 0, "data-set", r))
                        .and_then(|n| chapter::node_rect(m.dom(), n))
                    {
                        paint::wash(scene, rect, ink.with_alpha(HOVER_FILL_ALPHA), ui_scale);
                        paint::outline(scene, rect, ink.with_alpha(HOVER_BORDER_ALPHA), 1.0, ui_scale);
                    }
                }
            }))
            .is_ok();
        });
        if !painted && self.view == View::Reading {
            self.engine_panicked();
        }

        if std::env::var_os("OMAREAD_DEBUG_TIME").is_some() {
            eprintln!("TIME frame: {:.1}ms", started.elapsed().as_secs_f32() * 1000.0);
        }
    }

    fn on_key(&mut self, event_loop: &ActiveEventLoop, key: Key) {
        // A note is read and dismissed; nothing else happens while it is up.
        // Unless it asked about a link, and the key is Enter.
        if self.popup.take().is_some() {
            if let Some(url) = self.leaving.take()
                && key == Key::Named(NamedKey::Enter)
            {
                open_url(&url);
            }
            self.request_redraw();
            return;
        }
        if self.wants_help(&key) {
            self.menu = None;
            return self.open_help();
        }
        match self.view {
            View::Library => self.library_key(event_loop, key),
            View::Reading => self.reading_key(event_loop, key),
            View::Toc => self.toc_key(event_loop, key),
        }
    }

    fn library_key(&mut self, event_loop: &ActiveEventLoop, key: Key) {
        let cols = self.cards_per_row();

        // The panel takes every key while it is up, the same way the tag box
        // below takes every letter while it is being typed.
        if self.menu.is_some() {
            if self.adding_folder.is_none() && self.menu_key(&key) {
                return;
            }
            match (key, self.adding_folder.is_some()) {
                (Key::Named(NamedKey::Escape), true) => self.adding_folder = None,
                (Key::Named(NamedKey::Escape | NamedKey::F10 | NamedKey::ContextMenu), false) => {
                    self.menu = None
                }
                (Key::Named(NamedKey::Enter), true) => return self.add_folder(),
                (Key::Named(NamedKey::Backspace), true) => {
                    if let Some(t) = self.adding_folder.as_mut() {
                        t.pop();
                    }
                }
                (Key::Character(c), true) => {
                    if let Some(t) = self.adding_folder.as_mut() {
                        t.push_str(c.as_str());
                    }
                }
                (Key::Named(NamedKey::Space), true) => {
                    if let Some(t) = self.adding_folder.as_mut() {
                        t.push(' ');
                    }
                }
                (Key::Named(NamedKey::F5), _) => self.rescan(),
                _ => return,
            }
            match self.menu.is_some() {
                true => self.build_menu(),
                false => self.request_redraw(),
            }
            return;
        }

        // While a tag is being typed every letter is tag text, so this runs
        // before the search-and-navigate keys below.
        if self.tagging.is_some() {
            match key {
                Key::Named(NamedKey::Escape) => self.tagging = None,
                Key::Named(NamedKey::Enter) => self.apply_tag(),
                Key::Named(NamedKey::Backspace) => {
                    if let Some(t) = self.tagging.as_mut() {
                        t.pop();
                    }
                }
                Key::Character(c) => {
                    if let Some(t) = self.tagging.as_mut() {
                        t.push_str(c.as_str());
                    }
                }
                Key::Named(NamedKey::Space) => {
                    if let Some(t) = self.tagging.as_mut() {
                        t.push('-');
                    }
                }
                _ => return,
            }
            self.suggestions = self.completions();
            self.lib_doc = None;
            self.request_redraw();
            return;
        }

        match key {
            Key::Named(NamedKey::Escape) => {
                if self.query.is_empty() {
                    event_loop.exit();
                } else {
                    self.query.clear();
                    self.reload_rows();
                }
            }
            Key::Named(NamedKey::Enter) => {
                self.open_selected();
                return;
            }
            Key::Named(NamedKey::Backspace) => {
                self.query.pop();
                self.reload_rows();
            }
            // Moving the selection paints; it does not rebuild.
            Key::Named(NamedKey::ArrowRight) => return self.move_selection(1),
            Key::Named(NamedKey::ArrowLeft) => return self.move_selection(-1),
            Key::Named(NamedKey::ArrowDown) => return self.move_selection(cols),
            Key::Named(NamedKey::ArrowUp) => return self.move_selection(-cols),
            Key::Named(NamedKey::PageDown) => self.turn_view(true),
            Key::Named(NamedKey::PageUp) => self.turn_view(false),
            Key::Named(NamedKey::Space) => {
                // Space types a space while searching, otherwise pages down.
                if self.query.is_empty() {
                    self.turn_view(true);
                } else {
                    self.query.push(' ');
                    self.reload_rows();
                }
            }
            // Typing searches. Sort and rescan need a modifier-free key that is
            // not a letter, so they live on F5 and Tab.
            Key::Named(NamedKey::Tab) => {
                self.sort = self.sort.next();
                println!("omaread: sorted by {}", self.sort.label());
                self.reload_rows();
            }
            Key::Named(NamedKey::F5) => self.rescan(),
            Key::Named(NamedKey::F3) => return self.pick(false),
            // The menu key, or F10 where there is none: the panel the cog
            // opens, so it is reachable without a mouse.
            Key::Named(NamedKey::F10 | NamedKey::ContextMenu) => {
                self.open_menu();
                self.hover_menu = None;
                self.step_menu(1);
                return;
            }
            // A modifier-free key that is not a letter, for the same reason
            // sort and rescan are: letters go to the search box.
            Key::Named(NamedKey::F2) if !self.rows.is_empty() => {
                self.tagging = Some(String::new());
                self.suggestions = self.completions();
            }
            Key::Character(c) => {
                self.query.push_str(c.as_str());
                self.reload_rows();
            }
            _ => return,
        }
        self.lib_doc = None;
        self.request_redraw();
    }

    /// The library card under the pointer, if any.
    fn card_under_pointer(&self) -> Option<usize> {
        if self.view != View::Library {
            return None;
        }
        let doc = self.lib_doc.as_ref()?;
        let (x, y) = self.pointer_on_page(doc)?;
        let hit = doc.doc.hit(x, y)?;
        ancestor_attr(doc.dom(), hit.node_id, "data-index")?.parse().ok()
    }

    /// The pointer in a chrome page's flow, or `None` over the margins. What
    /// lies there is masked off and belongs to the page before or after: the
    /// next row's card took the hover ring, painted over the mask, and a click
    /// in the margin opened a book nobody could see.
    fn pointer_on_page(&self, doc: &Chapter) -> Option<(f32, f32)> {
        let y = self.cursor.1 / self.scale - self.page_margin();
        (0.0..doc.pages.extent_of(self.page))
            .contains(&y)
            .then(|| (self.cursor.0 / self.scale, doc.pages.flow_y(self.page, y)))
    }

    /// Open whatever is under the pointer. Coordinates are in CSS px relative
    /// to the page slice, so the page offset has to come back off the Y.
    fn on_click(&mut self) {
        if self.view == View::Reading {
            return;
        }
        let Some(doc) = self.doc() else { return };
        let Some((x, y)) = self.pointer_on_page(doc) else { return };
        let Some(hit) = doc.doc.hit(x, y) else { return };
        match self.view {
            View::Library => {
                // The gear opens the panel; a press anywhere else in it is
                // handled by `menu_action`, which ran before this.
                if ancestor_attr(doc.dom(), hit.node_id, "data-menu").is_some() {
                    self.open_menu();
                    return;
                }
                // A suggestion is a click target too; check it first, since a
                // card and a suggestion are both just boxes in this document.
                if let Some(text) = ancestor_attr(doc.dom(), hit.node_id, "data-suggest") {
                    self.query = text;
                    self.reload_rows();
                    self.request_redraw();
                    return;
                }
                let Some(hash) = ancestor_attr(doc.dom(), hit.node_id, "data-hash") else {
                    return;
                };
                let Some(i) = self.rows.iter().position(|r| r.hash == hash) else { return };
                self.selected = i;
                self.open_selected();
            }
            View::Toc => {
                let Some(i) = ancestor_attr(doc.dom(), hit.node_id, "data-index")
                    .and_then(|v| v.parse().ok())
                else {
                    return;
                };
                self.open_toc_entry(i);
            }
            View::Reading => {}
        }
    }

    fn cards_per_row(&self) -> isize {
        // The grid owns its own geometry; a second copy of these numbers here is
        // how arrow keys start stepping onto columns that do not exist.
        grid::per_row(self.size.0 as f32 / self.scale) as isize
    }

    fn move_selection(&mut self, by: isize) {
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() as isize - 1;
        self.selected = (self.selected as isize + by).clamp(0, last) as usize;
        // Follow the selection onto its page.
        if let Some(page) = self.lib_doc.as_ref().and_then(|d| page_of_index(d, self.selected)) {
            self.page = page;
        }
        self.request_redraw();
    }

    fn rescan(&mut self) {
        // A theme switch rewrites the rendered palette, and F5 already means
        // "pick up what changed on disk".
        self.style.omarchy = style::omarchy_chrome();
        // Not `drop_chrome`: F5 is also pressed from inside the settings
        // panel, which its caller rebuilds.
        self.ui = style::omarchy_ui_scale();
        self.hud_doc = None;
        let Some(db) = self.db.clone() else { return };
        if let Ok(d) = db.lock() {
            let (seen, added) = library::scan(&d);
            println!("omaread: rescan — {seen} files, {added} new");
        }
        self.reload_rows();
    }

    fn reading_key(&mut self, event_loop: &ActiveEventLoop, key: Key) {
        // The menu's own keys act on it; any other key puts it away, and
        // Escape or the menu key do nothing else.
        if self.menu.is_some() {
            if self.menu_key(&key) {
                return;
            }
            self.menu = None;
            self.request_redraw();
            if matches!(key, Key::Named(NamedKey::Escape | NamedKey::F10 | NamedKey::ContextMenu)) {
                return;
            }
        }
        if self.goto_key(&key) {
            return;
        }
        match key {
            Key::Named(NamedKey::Escape) => self.to_library(),
            Key::Named(NamedKey::Tab) => self.open_toc(),
            Key::Named(NamedKey::ArrowRight) | Key::Named(NamedKey::PageDown) => self.turn(true),
            Key::Named(NamedKey::ArrowLeft) | Key::Named(NamedKey::PageUp) => self.turn(false),
            Key::Named(NamedKey::Space) => self.turn(true),
            Key::Named(NamedKey::ArrowDown) => self.turn_chapter(true),
            Key::Named(NamedKey::ArrowUp) => self.turn_chapter(false),
            // The right click's menu, from the keyboard.
            Key::Named(NamedKey::F10 | NamedKey::ContextMenu) => {
                self.open_context_menu();
                self.hover_menu = None;
                self.step_menu(1);
            }
            Key::Character(c) => match c.as_str() {
                "q" => {
                    self.save_position();
                    event_loop.exit();
                }
                "l" => self.to_library(),
                "/" => {
                    self.find.clear();
                    self.toc_mode = TocMode::Search;
                    self.open_toc();
                }
                "m" => self.open_marks(),
                "b" => self.toggle_bookmark(),
                "h" => drop(self.highlight_selection()),
                "y" => self.copy_selection(),
                "c" => self.toggle_columns(),
                "g" => self.open_goto(),
                "t" => {
                    self.style.theme = match self.style.theme {
                        Theme::Omarchy => Theme::White,
                        Theme::White => Theme::Sepia,
                        Theme::Sepia => Theme::Grey,
                        Theme::Grey => Theme::Night,
                        Theme::Night if self.style.omarchy.is_some() => Theme::Omarchy,
                        Theme::Night => Theme::White,
                    };
                    println!("omaread: theme {:?}", self.style.theme);
                    style::set_setting("theme", &format!("{:?}", self.style.theme).to_lowercase());
                    self.restyle();
                }
                "+" | "=" => self.set_font_scale(self.style.scale + 0.1),
                "-" => self.set_font_scale(self.style.scale - 0.1),
                _ => {}
            },
            _ => {}
        }
    }

    /// Contents keys are deliberately few: this is a list you pass through, not
    /// a place to configure anything.
    fn toc_key(&mut self, event_loop: &ActiveEventLoop, key: Key) {
        match key {
            Key::Named(NamedKey::Escape) => {
                // Esc backs out one step: query first, then the view.
                if self.noting.is_some() {
                    self.noting = None;
                    self.note.clear();
                } else if self.toc_mode == TocMode::Search && !self.find.is_empty() {
                    self.find.clear();
                } else {
                    self.close_toc();
                    return;
                }
            }
            Key::Named(NamedKey::Tab) => {
                self.close_toc();
                return;
            }
            Key::Named(NamedKey::Backspace) if self.noting.is_some() => {
                self.note.pop();
            }
            Key::Named(NamedKey::Space) if self.noting.is_some() => self.note.push(' '),
            Key::Character(c) if self.noting.is_some() => self.note.push_str(c.as_str()),
            Key::Named(NamedKey::Backspace) if self.toc_mode == TocMode::Search => {
                self.find.pop();
                self.toc_sel = 0;
            }
            Key::Named(NamedKey::Space) if self.toc_mode == TocMode::Search => {
                self.find.push(' ');
                self.toc_sel = 0;
            }
            // While searching every letter is query text, so the single-key
            // commands below are unreachable — which is what you want when you
            // are typing a word that happens to contain "q".
            Key::Character(c) if self.toc_mode == TocMode::Search => {
                self.find.push_str(c.as_str());
                self.toc_sel = 0;
            }
            // Marks mode has room for commands, because nothing is being typed.
            Key::Character(c) if self.toc_mode == TocMode::Marks => match c.as_str() {
                "n" => self.begin_note(),
                "x" => self.delete_mark(),
                "e" => self.export_marks(),
                "q" => {
                    self.save_position();
                    event_loop.exit();
                    return;
                }
                "l" => {
                    self.to_library();
                    return;
                }
                _ => return,
            },
            Key::Named(NamedKey::Enter) => {
                if self.noting.is_some() {
                    self.save_note();
                } else {
                    self.open_toc_entry(self.toc_sel);
                    return;
                }
            }
            Key::Named(NamedKey::ArrowDown) => self.move_toc_selection(1),
            Key::Named(NamedKey::ArrowUp) => self.move_toc_selection(-1),
            Key::Named(NamedKey::Home) => self.move_toc_selection(isize::MIN / 2),
            Key::Named(NamedKey::End) => self.move_toc_selection(isize::MAX / 2),
            Key::Named(NamedKey::PageDown) | Key::Named(NamedKey::Space) => {
                self.turn_view(true);
                return;
            }
            Key::Named(NamedKey::PageUp) => {
                self.turn_view(false);
                return;
            }
            Key::Character(c) => match c.as_str() {
                "q" => {
                    self.save_position();
                    event_loop.exit();
                    return;
                }
                "l" => {
                    self.to_library();
                    return;
                }
                _ => return,
            },
            _ => return,
        }
        // The selection is baked into the markup, so moving it rebuilds.
        self.toc_doc = None;
        self.request_redraw();
    }
}

/// Hand a link to the desktop's browser or mail client. Reaped on a thread so
/// it leaves no zombie behind.
fn open_url(url: &str) {
    match std::process::Command::new("xdg-open").arg(url).spawn() {
        Ok(mut child) => drop(std::thread::spawn(move || child.wait())),
        Err(e) => eprintln!("omaread: could not run xdg-open ({e})"),
    }
}

/// The `href` under a point in the flow, or within a few pixels of it.
///
/// A note reference is a superscript digit — a 10px target in a 660px measure,
/// and it sits inside a sentence, so the pointer lands beside it as often as on
/// it. The exact point wins; the ring is only consulted when it hits nothing.
///
/// ponytail: eight probes at a fixed radius. Measuring the glyph's own box
/// would be exact, but an inline element has no Taffy box to measure
/// (CONTEXT.md §9) — the run lives in the paragraph's parley layout.
fn href_at(doc: &blitz_html::HtmlDocument, x: f32, y: f32) -> Option<String> {
    const R: f32 = 7.0;
    [
        (0.0, 0.0),
        (-R, 0.0),
        (R, 0.0),
        (0.0, -R),
        (0.0, R),
        (-R, -R),
        (R, -R),
        (-R, R),
        (R, R),
    ]
    .into_iter()
    .find_map(|(dx, dy)| {
        let hit = doc.hit(x + dx, y + dy)?;
        ancestor_attr(doc, hit.node_id, "href")
    })
}

/// Is this link target a note to read here, or a place to go to?
///
/// A footnote, an endnote and a bibliography entry are all a short block of
/// text; a section is a heading, a wrapper, or pages of prose. That is the whole
/// test — `epub:type="noteref"`, which a spec would key on, is carried by 4
/// books of 60 sampled from the real library, so it cannot be the rule.
///
/// The cut-off is only a bound on what is worth laying out as a popup: a note
/// is shown when it fits the window (`note_fits`), and a target holding a
/// heading is a section whatever its length (`chapter::has_heading`).
///
/// ponytail: length and headings. Key on `epub:type`/`role` as well if a real
/// book gets it wrong.
/// The paragraph a target stands for, when the target is the marker that opens
/// it: Calibre puts the id on `<small>[4]</small>` at the start of the note,
/// not on the note. An inline anywhere else in its paragraph is a place in the
/// text, like the back-link's target at the end of the paragraph that cites
/// the note.
fn note_of_marker(dom: &blitz_dom::BaseDocument, target: usize) -> usize {
    let Some(node) = dom.get_node(target) else { return target };
    match node.parent.and_then(|p| chapter::text_element(dom, p)) {
        Some(b) => {
            // From the DOM: an inline has no text layout of its own to read.
            let marker = node.text_content();
            let opens = !marker.trim().is_empty()
                && chapter::text_of(dom, b).trim_start().starts_with(marker.trim());
            if opens { b } else { target }
        }
        None => target,
    }
}

fn is_note(tag: &str, text: &str) -> bool {
    const NOTE_MAX: usize = 5000;
    matches!(tag, "p" | "li" | "aside" | "div" | "span" | "td")
        && !text.trim().is_empty()
        && text.chars().count() <= NOTE_MAX
}

/// The chapter `spine` falls in, by the book's own navigation when it has some.
/// What the digits typed into the go-to bar point at, as a fraction of the
/// book: a percentage when they end in `%` or the book has no page numbers
/// yet, a page of `pages` otherwise. `None` until there is a number.
fn goto_typed(typed: &str, pages: Option<usize>) -> Option<f32> {
    let n: f32 = typed.trim_end_matches('%').parse().ok()?;
    let at = match pages {
        Some(total) if !typed.ends_with('%') => (n - 1.0) / total.saturating_sub(1).max(1) as f32,
        _ => n / 100.0,
    };
    Some(at.clamp(0.0, 1.0))
}

fn chapter_name(toc: &[book::TocEntry], spine: usize) -> String {
    toc.iter()
        .filter(|e| e.spine <= spine)
        .next_back()
        .map(|e| e.label.clone())
        .unwrap_or_else(|| i18n::tr!("chapter-n", n = spine + 1))
}

/// A book's highlights and notes as Markdown, in reading order, under the
/// chapter each falls in. A bookmark has no words, so it is only worth a line
/// when it carries a note.
fn marks_markdown(title: &str, author: &str, toc: &[book::TocEntry], marks: &[db::Mark]) -> String {
    let mut out = format!("# {title}\n");
    if !author.is_empty() {
        out += &format!("\n*{author}*\n");
    }
    let mut heading = None;
    for m in marks.iter().filter(|m| !m.is_bookmark() || !m.note.is_empty()) {
        let here = chapter_name(toc, cfi::Cfi::parse(&m.cfi).map_or(0, |c| c.spine));
        if heading.as_ref() != Some(&here) {
            out += &format!("\n## {here}\n");
            heading = Some(here);
        }
        if !m.text.is_empty() {
            out += "\n";
            for line in m.text.lines() {
                out += &format!("> {line}\n");
            }
        }
        if !m.note.is_empty() {
            out += &format!("\n{}\n", m.note);
        }
    }
    out
}

/// Where the outgoing and incoming pages sit, `t` of the way through a turn.
///
/// Turning forward walks the old page off to the left and brings the new one in
/// from the right; turning back is the same motion mirrored, which is the whole
/// point — the direction is the feedback.
fn slide_offsets(forward: bool, t: f32, width: f32) -> (f32, f32) {
    let dir = if forward { -1.0 } else { 1.0 };
    (dir * t * width, (dir * t - dir) * width)
}

/// Which cards sit on `page`, from a laid-out grid.
fn visible_cards(doc: &Chapter, page: usize) -> std::ops::Range<usize> {
    let top = doc.pages.top_of(page);
    let bottom = top + doc.pages.extent_of(page);
    let on_page: Vec<usize> = chapter::indexed_tops(doc.dom())
        .into_iter()
        .filter(|&(_, y)| y >= top - 1.0 && y < bottom)
        .map(|(i, _)| i)
        .collect();
    match (on_page.first(), on_page.last()) {
        (Some(&a), Some(&b)) => a..b + 1,
        _ => 0..0,
    }
}

/// Where a library selection lands when the page turns under it: the same
/// place on the new page, or the new page's last card if it is shorter.
fn ride_along(selected: usize, old: std::ops::Range<usize>, new: std::ops::Range<usize>) -> usize {
    if new.is_empty() || new.contains(&selected) {
        return selected;
    }
    (new.start + selected.saturating_sub(old.start)).min(new.end - 1)
}

/// The page a `data-index` row falls on. Both list views mark their rows this
/// way, so the selection can be followed onto its page.
fn page_of_index(doc: &Chapter, index: usize) -> Option<usize> {
    let node = find_by_attr(doc.dom(), 0, "data-index", &index.to_string())?;
    Some(doc.pages.page_containing(chapter::node_top(doc.dom(), node)?))
}

/// First element under `id` carrying `attr="value"`.
fn find_by_attr(
    dom: &blitz_dom::BaseDocument,
    id: usize,
    attr: &str,
    value: &str,
) -> Option<usize> {
    let node = dom.get_node(id)?;
    if let blitz_dom::NodeData::Element(el) = &node.data {
        if el.attrs.iter().any(|a| &*a.name.local == attr && &*a.value == value) {
            return Some(id);
        }
    }
    node.children
        .iter()
        .find_map(|c| find_by_attr(dom, *c, attr, value))
}

/// Walk up from a hit node to the nearest ancestor carrying `attr`.
pub fn ancestor_attr(dom: &blitz_dom::BaseDocument, mut id: usize, attr: &str) -> Option<String> {
    loop {
        let node = dom.get_node(id)?;
        if let blitz_dom::NodeData::Element(el) = &node.data {
            if let Some(a) = el.attrs.iter().find(|a| &*a.name.local == attr) {
                return Some(a.value.to_string());
            }
        }
        id = node.parent?;
    }
}

impl ApplicationHandler<accesskit_winit::Event> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        // Without an app_id the window reports an empty class, and every
        // Hyprland rule keyed on it silently never matches — including the one
        // shipped in assets/omarchy (CONTEXT.md §11).
        #[cfg(target_os = "linux")]
        use winit::platform::wayland::WindowAttributesExtWayland;
        // Hidden until the AccessKit adapter exists: it has to be in place
        // before the window is first shown.
        let attrs = Window::default_attributes()
            .with_visible(false)
            .with_title("Omaread")
            .with_inner_size(winit::dpi::LogicalSize::new(900.0, 1000.0));
        #[cfg(target_os = "linux")]
        let attrs = attrs.with_name("omaread", "omaread");
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        self.a11y = self
            .proxy
            .clone()
            .map(|p| accesskit_winit::Adapter::with_event_loop_proxy(event_loop, &window, p));
        window.set_visible(true);

        let size = window.inner_size();
        self.size = (size.width.max(1), size.height.max(1));
        self.scale = window.scale_factor() as f32;

        eprintln!(
            "omaread: window {}x{} physical, scale {} -> {:.0}x{:.0} CSS px; measure {:.0}px",
            self.size.0, self.size.1, self.scale,
            self.size.0 as f32 / self.scale, self.size.1 as f32 / self.scale,
            33.0 * self.style.font_px(),
        );
        self.renderer.resume(window.clone(), self.size.0, self.size.1);
        let (tx, rx) = channel();
        let w = window.clone();
        style::watch_omarchy(move || {
            let _ = tx.send(());
            w.request_redraw();
        });
        self.theme_rx = Some(rx);
        self.window = Some(window);

        // The library needs no chapter; only reopen a book if one is open.
        if matches!(self.view, View::Reading) {
            let start = self.index;
            self.load_chapter(start);
        }
        if let Ok(p) = std::env::var("OMAREAD_START_PAGE") {
            if let Ok(n) = p.parse::<usize>() {
                self.page = n.saturating_sub(1).min(self.page_count().saturating_sub(1));
            }
        }
        self.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if let (Some(a), Some(w)) = (self.a11y.as_mut(), self.window.as_ref()) {
            a.process_event(w, &event);
        }
        match event {
            // The pointer left, so nothing is under it.
            WindowEvent::CursorLeft { .. } => {
                if self.hover_card.take().is_some()
                    | self.hover_hud.take().is_some()
                    | self.hover_menu.take().is_some()
                {
                    self.request_redraw();
                }
            }

            // X11 and XWayland only: winit 0.30 has no drag and drop on Wayland.
            WindowEvent::DroppedFile(path) if path.is_dir() => self.add_dir(path),
            WindowEvent::DroppedFile(path) if library::is_book(&path) => self.open_file(&path),

            WindowEvent::CloseRequested => {
                self.save_position();
                event_loop.exit();
            }

            // Moved to another monitor, or `omarchy hyprland monitor scaling`.
            // A resize follows with the new physical size; the chrome has to
            // be laid out again at the new scale either way.
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale = scale_factor as f32;
                self.drop_chrome();
                self.relayout();
                self.request_redraw();
            }

            // `omarchy display text size` changes the desktop while we are in
            // the background; coming back is when to look.
            WindowEvent::Focused(true) => {
                let ui = style::omarchy_ui_scale();
                if ui != self.ui {
                    self.ui = ui;
                    self.drop_chrome();
                    self.request_redraw();
                }
            }

            WindowEvent::Resized(size) => {
                self.size = (size.width.max(1), size.height.max(1));
                self.renderer.set_size(self.size.0, self.size.1);
                self.relayout();
                self.request_redraw();
            }

            WindowEvent::CursorMoved { position, .. } => {
                let moved = self.cursor != (position.x as f32, position.y as f32);
                self.cursor = (position.x as f32, position.y as f32);
                if moved {
                    if self.dragging {
                        self.extend_selection();
                    }
                    if self.scrubbing {
                        self.scrub_to_pointer();
                    }
                    // Repaint only when the card under the pointer changes, not
                    // on every motion event.
                    let over = self.card_under_pointer();
                    if over != self.hover_card {
                        self.hover_card = over;
                        self.request_redraw();
                    }
                    self.poke_hud();
                    let on = self.hud_under_pointer();
                    if on != self.hover_hud {
                        self.hover_hud = on;
                        self.request_redraw();
                    }
                    let row = self.menu_under_pointer();
                    if row != self.hover_menu {
                        self.hover_menu = row;
                        self.request_redraw();
                    }
                }
            }

            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: winit::event::MouseButton::Right,
                ..
            } if self.view == View::Reading => {
                // The same check a left press makes, so a right click inside a
                // highlight offers to remove it.
                self.sel_mark = self.mark_at(self.pointer_in_flow());
                self.open_context_menu();
            }

            WindowEvent::MouseInput { state, button, .. }
                if button == winit::event::MouseButton::Left =>
            {
                match state {
                    // A menu is over everything, so it gets the press first.
                    ElementState::Pressed if self.menu_action() => {}
                    // In the page, a press anchors a selection; in the library
                    // and the contents it opens whatever is under the pointer.
                    ElementState::Pressed if self.view == View::Reading => {
                        // The HUD is on top of the page, so it gets the click,
                        // then a link, and only then does the page take it.
                        if !self.hud_action() && !self.link_action() {
                            self.goto = None;
                            self.press_at = Some(self.cursor);
                            self.begin_selection();
                        }
                    }
                    ElementState::Pressed => self.on_click(),
                    // Let go of the go-to bar's track: go there.
                    ElementState::Released if self.scrubbing => {
                        self.scrubbing = false;
                        self.goto_jump();
                    }
                    ElementState::Released => {
                        let dragged = std::mem::take(&mut self.dragging);
                        let (px, py) = self.press_at.take().unwrap_or((f32::MAX, f32::MAX));
                        let click = (px - self.cursor.0).abs() < 5.0 && (py - self.cursor.1).abs() < 5.0;
                        // A press with no drag is a click, not a selection.
                        if self.sel.is_some_and(|(a, f)| a == f) {
                            self.sel = None;
                            self.request_redraw();
                        } else if dragged && self.selected().is_some() {
                            // A finished selection offers what can be done with it.
                            self.open_context_menu();
                        }
                        // A click on a figure enlarges it; on a highlight with a
                        // note, shows the note.
                        if click && self.view == View::Reading && self.menu.is_none() {
                            if let Some(fig) = self.figure_at() {
                                self.open_figure(&fig);
                            } else if let Some(note) = self.sel_mark.and_then(|id| self.mark_note(id)) {
                                self.open_note(&note);
                            }
                        }
                    }
                }
            }

            WindowEvent::MouseWheel { delta, .. } => {
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y,
                    MouseScrollDelta::PixelDelta(p) => -p.y as f32,
                };
                if dy.abs() > 0.5 {
                    // A context menu belongs to the page it was opened on.
                    if self.view == View::Reading {
                        self.menu = None;
                    }
                    // The reading turn crosses chapters and saves a position;
                    // the library and the contents just page their document.
                    match self.view {
                        View::Reading => self.turn(dy > 0.0),
                        _ => self.turn_view(dy > 0.0),
                    }
                }
            }

            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                self.on_key(event_loop, event.logical_key);
            }

            // The guard is around the whole redraw, not only around painting
            // the documents: presenting happens outside that inner one, and
            // softbuffer unwraps a compositor that will not take the buffer. A
            // frame nobody could present is a frame worth losing, not a window.
            WindowEvent::RedrawRequested => {
                if catch_unwind(AssertUnwindSafe(|| self.redraw())).is_err() {
                    eprintln!("omaread: dropped a frame the compositor would not take");
                    self.recover();
                } else {
                    self.drops = 0;
                    self.retry_at = None;
                }
                // Every change on screen comes through here, so the screen
                // reader's picture is refreshed here too.
                self.update_a11y();
            }

            _ => {}
        }
    }

    fn user_event(&mut self, _: &ActiveEventLoop, event: accesskit_winit::Event) {
        match event.window_event {
            accesskit_winit::WindowEvent::InitialTreeRequested => self.update_a11y(),
            accesskit_winit::WindowEvent::ActionRequested(req) => self.a11y_action(req),
            accesskit_winit::WindowEvent::AccessibilityDeactivated => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.drain_resources();
        self.take_measured();
        self.take_picked();
        if self.theme_rx.as_ref().is_some_and(|rx| rx.try_iter().count() > 0) {
            self.reread_palette();
        }
        let now = Instant::now();

        if self.exit_at.is_some_and(|at| now >= at) {
            self.save_position();
            event_loop.exit();
            return;
        }

        // Every clock feeds one deadline, the soonest; with none the loop sleeps
        // until an event. An early return per clock froze the others: under
        // `OMAREAD_EXIT_AFTER` a page turn stopped on its first frame, a page
        // nudged left, and a finished turn left a deadline in the past that
        // spun the loop until the mouse moved.
        let mut wake = vec![self.exit_at];
        match self.retry_at {
            // Nothing else is worth drawing until a frame lands again.
            Some(at) if now < at => wake.push(Some(at)),
            retry => {
                if retry.is_some() {
                    self.retry_at = None;
                    self.request_redraw();
                }
                // A page turn animates, so it drives the clock while it lasts.
                if let Some(s) = self.slide {
                    match s.at.elapsed() >= SLIDE {
                        true => (self.slide, self.slide_from) = (None, None),
                        false => wake.push(Some(now + FRAME)),
                    }
                    self.request_redraw();
                }
                match self.hud_until {
                    Some(at) if now >= at => {
                        self.hud_until = None;
                        // The go-to bar is in the HUD and stays until it is done.
                        self.hud_shown = self.goto.is_some();
                        self.request_redraw();
                    }
                    at => wake.push(at),
                }
            }
        }
        event_loop.set_control_flow(match wake.into_iter().flatten().min() {
            Some(at) => ControlFlow::WaitUntil(at),
            None => ControlFlow::Wait,
        });
    }
}
