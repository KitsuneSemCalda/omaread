//! The reading HUD: the book's title and how far through it you are.
//!
//! Chrome that appears on a mouse move and goes away again on its own, so the
//! page is uninterrupted while you read (CONTEXT.md §3 calls this a gesture
//! rather than a setting). Like every other surface here it is HTML/CSS through
//! the blitz pipeline; it reaches the screen over the page rather than instead
//! of it via `paint::overlay`.

use crate::grid::escape;
use crate::i18n::tr;
use crate::style::Chrome;

/// Height of a bar and the window inset, in CSS px. Both bars are in normal
/// flow, so the bottom one is placed by a computed margin.
const BAR: f32 = 34.0;
const INSET: f32 = 16.0;

/// `readout` is whatever the foot of the page should say — a percentage or a
/// page number; clicking it swaps which. `window_height` is in CSS pixels.
pub fn html(
    title: &str,
    readout: &str,
    columns: usize,
    on_highlight: bool,
    window_height: f32,
    // Where a followed link was followed *from*, already worded. `None` when
    // there is nowhere to go back to, and then there is no button either.
    back: Option<&str>,
    // This page carries a bookmark: the button says so, in the accent.
    bookmarked: bool,
    // The go-to bar (`g`): where it points, 0.0..=1.0, and what is there,
    // already worded. It takes the bottom bar's place while it is open.
    scrub: Option<(f32, &str)>,
) -> String {
    // Top bar occupies [INSET, INSET + BAR]; this drops the bottom one to
    // [H - INSET - BAR, H - INSET].
    let push = (window_height - 2.0 * INSET - 2.0 * BAR).max(0.0);

    // Once the pointer is inside a highlight, the useful offer is removing it.
    let (mark_action, mark_label) = match on_highlight {
        true => ("unhighlight", tr!("hud-remove")),
        false => ("highlight", tr!("hud-highlight")),
    };

    let (on, bookmark) = match bookmarked {
        true => (" on", tr!("hud-bookmarked")),
        false => ("", tr!("hud-bookmark")),
    };

    format!(
        r#"<!DOCTYPE html>
<html><body>
<div class="hud">
  <span class="btn{on}" data-hud="bookmark"><span class="ico" data-icon="bookmark"></span><span class="lbl">{bookmark}</span></span>
  <span class="btn" data-hud="contents"><span class="ico" data-icon="contents"></span><span class="lbl">{contents}</span></span>
  <span class="btn" data-hud="{mark_action}"><span class="ico" data-icon="highlight"></span><span class="lbl">{mark_label}</span></span>
  <span class="grow"></span>
  <span class="btn" data-hud="smaller">A−</span>
  <span class="btn" data-hud="bigger">A+</span>
  <span class="btn" data-hud="columns">{columns}</span>
</div>
<div class="hud" style="margin-top: {push}px">
  {foot}
</div>
</body></html>"#,
        contents = tr!("hud-contents"),
        columns = tr!("hud-columns", n = columns),
        foot = match scrub {
            // A rail with the part behind the thumb filled in. The track is
            // what the window hit-tests, so it is tall even though the rail is not.
            Some((at, label)) => format!(
                r#"<span class="track" data-hud="scrub"><span class="fill" style="width: {pct}%"></span><span class="thumb"></span><span class="rest"></span></span>
  <span class="where">{label}</span>"#,
                pct = at.clamp(0.0, 1.0) * 100.0,
                label = escape(label),
            ),
            None => format!(
                r#"<span class="btn" data-hud="library"><span class="ico" data-icon="back"></span><span class="lbl">{library}</span></span>
  <span class="title">{title}</span>
  {back}<span class="btn" data-hud="readout">{readout}</span>"#,
                title = escape(title),
                readout = escape(readout),
                library = tr!("hud-library"),
                back = match back {
                    Some(label) => {
                        format!(r#"<span class="btn" data-hud="back">{}</span>"#, escape(label))
                    }
                    None => String::new(),
                },
            ),
        },
    )
}

/// Width of the context menu, in CSS px. The window places it at the pointer,
/// so it has to know how wide it is.
pub const CONTEXT_W: f32 = 240.0;

/// The menu a right click or a finished drag opens over the page. Only what
/// applies is offered. It borrows the settings panel's sheet and `data-set`
/// rows, so hover and hit-testing are the panel's too.
pub fn context_html(selection: bool, on_highlight: bool, bookmarked: bool) -> String {
    let mut rows = String::new();
    if selection {
        rows += &row("copy", "menu-copy", "y");
        rows += &row("highlight", "menu-highlight", "h");
    }
    if on_highlight {
        rows += &row("unhighlight", "menu-remove-highlight", "");
    }
    if selection || on_highlight {
        rows += &row("note", "menu-add-note", "");
    }
    rows += &match bookmarked {
        true => row("bookmark", "menu-remove-bookmark", "b"),
        false => row("bookmark", "menu-bookmark-page", "b"),
    };
    context_doc(&rows)
}

/// The menu a right click on a library card opens. A ghost row has no file,
/// so no folder to show.
pub fn book_menu_html(on_disk: bool) -> String {
    let mut rows = String::new();
    if on_disk {
        rows += &row("reveal", "menu-show-in-folder", "");
    }
    rows += &row("remove", "menu-remove-book", "");
    context_doc(&rows)
}

fn row(set: &str, label: &str, key: &str) -> String {
    format!(
        r#"<div class="row" data-set="{set}" data-label="{0}">{0}<span class="key">{key}</span></div>"#,
        tr!(label)
    )
}

fn context_doc(rows: &str) -> String {
    format!(r#"<!DOCTYPE html><html><body><div class="menu">{rows}</div></body></html>"#)
}

/// A footnote or a bibliography entry, shown where you are rather than by
/// sending you to the page it lives on and leaving you to find your way back.
///
/// `top` pins the note's top edge, in CSS pixels; without it the sheet places it.
pub fn note_html(text: &str, top: Option<f32>) -> String {
    let gap = top.map_or(String::new(), |y| format!(r#" style="flex: 0 0 {y}px; min-height: 0""#));
    format!(r#"<!DOCTYPE html><html><body><div class="gap"{gap}></div><div class="note">{}</div></body></html>"#, escape(text))
}

/// A figure on its own, over the page. `src` is already absolute.
pub fn figure_html(src: &str) -> String {
    format!(r#"<!DOCTYPE html><html><body><div class="figure"><img src="{}"/></div></body></html>"#, escape(src))
}

/// The figure fills the window on the page's own ground: natural size, or
/// scaled down to fit, never up.
pub fn figure_stylesheet(ground: &str) -> String {
    format!(
        r#"
html {{ background: {ground}; }}
html body {{
  margin: 0;
  height: 100vh;
  display: flex;
  align-items: center;
  justify-content: center;
  background: {ground};
}}
html .figure img {{ display: block; max-width: 96vw; max-height: 96vh; }}
"#
    )
}

/// The popup's own sheet. Same overlay path as the bars: a document painted
/// over the page (CONTEXT.md §9), not a second window.
///
/// A square Omarchy panel, but the note is the book's own text, so it stays in
/// the book's face.
pub fn note_stylesheet(c: &Chrome) -> String {
    let Chrome { bg, fg, .. } = c;
    let border = c.tint(0.4);
    format!(
        r#"
html {{
  box-sizing: border-box;
  font-family: "Literata", "Charis SIL", serif;
  background: transparent;
  color: {fg};
}}
html *, html *::before, html *::after {{ box-sizing: inherit; }}
html body {{
  margin: 0;
  padding: 0 6vw;
  height: 100vh;
  display: flex;
  flex-direction: column;
  background: transparent;
}}
/* A short note sits a quarter down; a long one takes the room back. */
html .gap {{ flex: 0 1 26vh; min-height: 3vh; }}
html .note {{
  flex: none;
  padding: 22px 26px;
  border: 1px solid {border};
  background: {bg};
  font-size: 17px;
  line-height: 1.5;
  text-align: left;
}}
"#
    )
}

/// The keys overlay: what this view answers to, as (key, what it does) rows.
///
/// It rides the note popup's slot, so any key or click puts it away and
/// nothing else happens while it is up. `data-help` is how a resize tells it
/// from a note.
pub fn help_html(rows: &[(&str, String)]) -> String {
    let rows: String = rows
        .iter()
        .map(|(key, what)| {
            format!(
                r#"<div class="k"><span class="key">{}</span><span>{}</span></div>"#,
                escape(key),
                escape(what)
            )
        })
        .collect();
    format!(
        r#"<!DOCTYPE html><html><body><div class="help" data-help>
<div class="head">{title}</div>
{rows}
<div class="foot">{dismiss}</div>
</div></body></html>"#,
        title = tr!("help-title"),
        dismiss = tr!("help-dismiss"),
    )
}

/// An Omarchy panel, centred, keys on the left — the shape of Omarchy's own
/// keybindings menu (Super + K).
pub fn help_stylesheet(c: &Chrome) -> String {
    let Chrome { bg, fg, subtle, font, .. } = c;
    let border = c.tint(0.4);
    format!(
        r#"
html {{
  box-sizing: border-box;
  font-family: {font};
  background: transparent;
  color: {fg};
}}
html *, html *::before, html *::after {{ box-sizing: inherit; }}
html body {{ margin: 0; padding: 12vh 24px 0 24px; background: transparent; }}
html .help {{
  width: 440px;
  margin: 0 auto;
  padding: 14px 20px;
  border: 1px solid {border};
  background: {bg};
  font-size: 14px;
}}
html .head {{ font-weight: 600; padding-bottom: 10px; }}
html .k {{ display: flex; padding: 3px 0; }}
html .key {{ width: 130px; flex-shrink: 0; font-weight: 600; }}
html .foot {{ padding-top: 12px; font-size: 12px; color: {subtle}; }}
"#
    )
}

/// `html`-prefixed for the same reason every sheet here is: blitz-dom applies
/// its own defaults *after* ours (CONTEXT.md §9).
///
/// The page shows through, so `html` and `body` must stay transparent — only
/// the bar itself paints a ground.
pub fn stylesheet(c: &Chrome) -> String {
    let Chrome { bg, fg, subtle, accent, font, .. } = c;
    let (border, selected) = (c.tint(0.4), c.tint(0.18));
    format!(
        r#"
html {{
  box-sizing: border-box;
  font-family: {font};
  background: transparent;
  color: {fg};
}}
html *, html *::before, html *::after {{ box-sizing: inherit; }}
html body {{ margin: 0; padding: {INSET}px 24px; background: transparent; }}

html .hud {{
  display: flex;
  /* Not `baseline`: a flex container takes its baseline from its first item,
     and for the buttons with icons that is an empty 11px box, so they sat a few
     pixels above the plain-text ones. Centring does not care. */
  align-items: center;
  height: {BAR}px;
  padding: 0 12px;
  border: 1px solid {border};
  background: {bg};
  font-size: 14px;
}}
/* The title is the book, so it carries the weight. */
html .title {{
  flex-grow: 1;
  overflow: hidden;
  white-space: nowrap;
  font-weight: 600;
  padding-left: 12px;
}}
html .grow {{ flex-grow: 1; }}

/* Every control is a click target; `data-hud` is what the window hit-tests. */
html .btn {{
  color: {subtle};
  padding: 4px 9px;
  margin-left: 4px;
  white-space: nowrap;
}}
/* A control whose state is on — this page is bookmarked: Omarchy's selected
   fill, in the accent. The window paints its icon in the accent too. */
html .btn.on {{ color: {accent}; background: {selected}; }}

/* An empty box the window paints an icon into; it only has to hold the space.
   The button is a flex row so the slot gets a real layout box: an inline-block
   inside text becomes a parley inline box, whose `final_layout` size is 0, and
   the window would have nowhere to paint. */
html .btn {{ display: flex; align-items: center; }}
html .ico {{
  width: 11px;
  height: 11px;
  margin-right: 6px;
}}
html .lbl {{ white-space: nowrap; }}

/* The go-to bar. The fill's width is a share of the track less the thumb,
   which hangs into the padding, so 100% puts the thumb at the very end. */
html .track {{
  flex-grow: 1;
  display: flex;
  align-items: center;
  height: 24px;
  padding-right: 4px;
}}
html .fill {{ height: 4px; background: {accent}; }}
html .thumb {{ width: 4px; height: 16px; flex-shrink: 0; background: {fg}; }}
html .rest {{ flex-grow: 1; height: 4px; background: {border}; }}
html .where {{ flex-shrink: 0; white-space: nowrap; padding-left: 16px; }}
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bars are placed by computed margins, so the arithmetic is the layout.
    #[test]
    fn the_bars_sit_at_the_head_and_foot_of_the_window() {
        let out = html("A Book", "42%", 1, false, 1000.0, Some("Back to page 7"), false, None);
        let push = 1000.0 - 2.0 * INSET - 2.0 * BAR;
        assert!(out.contains(&format!("margin-top: {push}px")), "{out}");
        assert!(out.contains("42%"));
    }

    /// A resize finds the keys overlay by `data-help`, or rebuilds it as a note
    /// of its own text; and the overlay lays out at all.
    #[test]
    fn the_keys_overlay_is_tagged_and_lays_out() {
        let out = help_html(&[("<", "a & b".into())]);
        assert!(out.contains("data-help") && out.contains("&lt;") && out.contains("a &amp; b"));
        let doc = crate::chapter::layout_document(
            out,
            help_stylesheet(&crate::style::Theme::White.chrome()),
            None,
            crate::chapter::viewport(1200, 900, 1.0, false),
            900.0,
        )
        .expect("the overlay must lay out");
        assert!(crate::find_by_attr(doc.dom(), 0, "data-help", "").is_some());
    }

    /// A bookmarked page has to look bookmarked: the button turns on and says so.
    #[test]
    fn the_bookmark_button_shows_the_page_is_bookmarked() {
        let off = html("A Book", "5%", 1, false, 900.0, None, false, None);
        let on = html("A Book", "5%", 1, false, 900.0, None, true, None);
        assert!(off.contains(r#"<span class="btn" data-hud="bookmark">"#), "{off}");
        assert!(on.contains(r#"<span class="btn on" data-hud="bookmark">"#), "{on}");
        assert!(on.contains("Bookmarked") && !off.contains("Bookmarked"));
        assert!(stylesheet(&crate::style::Theme::White.chrome()).contains("html .btn.on {"));
    }

    /// A window shorter than the bars must not produce a negative margin.
    #[test]
    fn a_tiny_window_does_not_push_the_bar_off_the_top() {
        let out = html("A Book", "0%", 1, false, 10.0, Some("Back to page 7"), false, None);
        assert!(out.contains("margin-top: 0px"), "{out}");
    }

    /// Titles come from files off the internet, exactly as in the grid, and the
    /// readout is ours but goes through the same escaping.
    #[test]
    fn the_title_cannot_inject_markup() {
        let out = html(r#"<script>x</script>"#, "<b>", 2, false, 800.0, Some("Back to page 7"), false, None);
        assert!(!out.contains("<script"), "{out}");
        assert!(out.contains("&lt;script&gt;"));
        assert!(!out.contains("<b>"));
    }

    /// Every control the window has to hit-test must reach the markup, and the
    /// title has to be the bold one.
    #[test]
    fn the_controls_are_click_targets_and_the_title_is_bold() {
        let out = html("A Book", "page 8 of 19", 2, false, 900.0, Some("Back to page 7"), false, None);
        for what in [
            "bookmark", "contents", "highlight", "smaller", "bigger", "columns", "readout",
            "library",
        ] {
            assert!(out.contains(&format!(r#"data-hud="{what}""#)), "missing {what}: {out}");
        }
        assert!(out.contains("2 col"), "the columns control shows the current state");
        assert!(out.contains("page 8 of 19"));

        let css = stylesheet(&crate::style::Theme::White.chrome());
        assert!(css.contains("html .title {") && css.contains("font-weight: 600;"));
    }

    /// Once the pointer is inside a highlight, the useful offer is removing it.
    #[test]
    fn the_highlight_control_becomes_a_remove_control() {
        let plain = html("A Book", "5%", 1, false, 900.0, Some("Back to page 7"), false, None);
        assert!(plain.contains(r#"data-hud="highlight""#));
        assert!(!plain.contains("unhighlight"));

        let inside = html("A Book", "5%", 1, true, 900.0, Some("Back to page 7"), false, None);
        assert!(inside.contains(r#"data-hud="unhighlight""#), "{inside}");
        assert!(inside.contains(">Remove<"));
        // The icon slot stays, whichever way the button reads.
        assert!(inside.contains(r#"data-icon="highlight""#));
    }

    /// The context menu offers only what applies to where it was opened.
    #[test]
    fn the_context_menu_offers_only_what_applies() {
        let bare = context_html(false, false, false);
        assert!(!bare.contains(r#"data-set="copy""#) && !bare.contains("unhighlight"), "{bare}");
        assert!(!bare.contains(r#"data-set="note""#), "nothing to attach a note to: {bare}");
        assert!(bare.contains("Bookmark page"));

        let all = context_html(true, true, true);
        for what in ["copy", "highlight", "unhighlight", "note", "bookmark"] {
            assert!(all.contains(&format!(r#"data-set="{what}""#)), "missing {what}: {all}");
        }
        assert!(all.contains("Remove bookmark"));
    }

    /// Every icon slot the window paints into has to exist in the markup.
    #[test]
    fn the_icon_slots_are_there_for_the_window_to_paint_into() {
        let out = html("A Book", "5%", 2, false, 900.0, Some("Back to page 7"), false, None);
        for icon in ["bookmark", "contents", "highlight", "back"] {
            assert!(out.contains(&format!(r#"data-icon="{icon}""#)), "missing {icon}: {out}");
        }
        let css = stylesheet(&crate::style::Theme::White.chrome());
        assert!(css.contains("html .ico {"), "the slot needs a size to reserve");
    }

    /// Without a doctype html5ever reports "Unexpected token" once per parse —
    /// straight to stdout, from inside blitz, with no way to switch it off. The
    /// HUD is rebuilt on every page turn, so that was a stream of it.
    #[test]
    fn the_document_declares_a_doctype() {
        assert!(html("A", "1%", 1, false, 900.0, Some("Back to page 7"), false, None).starts_with("<!DOCTYPE html>"));
        assert!(crate::toc::html(&[], "Contents", "t", 0, 0).starts_with("<!DOCTYPE html>"));
        assert!(
            crate::grid::html(&[], "", crate::db::Sort::Recent, &[], None, 0..0)
                .starts_with("<!DOCTYPE html>")
        );
    }

    /// The page has to show through the parts of the overlay that are not bar.
    #[test]
    fn the_overlay_ground_is_transparent() {
        let css = stylesheet(&crate::style::Theme::White.chrome());
        assert_eq!(css.matches("background: transparent").count(), 2, "html and body");
    }
}
