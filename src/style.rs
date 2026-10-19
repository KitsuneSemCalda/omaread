//! The base stylesheet — Omaread's typography, supplied as a UA stylesheet.
//!
//! Under the whitelist CSS policy (CONTEXT.md §3) the publisher's stylesheets are
//! stripped, not cascaded. This file is therefore what every book looks like.

/// Text measure, in em. ~66 characters. Fixed by design, not a user setting.
pub const MEASURE_EM: f32 = 33.0;

/// Breathing room either side of the measure, in em. Also the minimum gutter
/// when the window is narrower than the measure.
pub const GUTTER_EM: f32 = 1.75;

/// Vertical margin above and below every page.
///
/// This belongs to the *page*, not the document: a page is a slice of a
/// continuous flow, so padding on `body` would only inset the first and last
/// page and leave every page in between running to the window edge.
pub const PAGE_MARGIN_EM: f32 = 2.5;

/// The same margin for the library and the contents, in CSS px. Those are
/// chrome: a book's margin scaled with the reading size put a 50–80px band of
/// nothing above the library's heading.
pub const CHROME_MARGIN: f32 = 20.0;

/// Reading themes. Four paper themes, plus the active Omarchy theme for visual
/// coherence with the desktop (CONTEXT.md §11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    White,
    Sepia,
    Grey,
    Night,
    /// Colours from `ReadingStyle::omarchy`; Sepia's when there is none.
    Omarchy,
}

impl Theme {
    /// A theme by its lowercase name, as settings and the menu spell it.
    pub fn named(name: &str) -> Option<Theme> {
        Some(match name {
            "white" => Theme::White,
            "sepia" => Theme::Sepia,
            "grey" => Theme::Grey,
            "night" => Theme::Night,
            "omarchy" => Theme::Omarchy,
            _ => return None,
        })
    }

    /// Page background as linear RGB components, for painting the window ground.
    pub fn background_rgb(self) -> [u8; 3] {
        match self {
            Theme::White => [0xfd, 0xfd, 0xfe],
            Theme::Sepia | Theme::Omarchy => [0xf8, 0xf1, 0xe3],
            Theme::Grey => [0xd6, 0xd6, 0xd2],
            Theme::Night => [0x14, 0x14, 0x16],
        }
    }

    /// Palette for the app chrome — library grid, panels.
    ///
    /// Off Omarchy this is the reader's own neutral ground; on Omarchy the
    /// chrome uses the rendered palette instead (CONTEXT.md §11).
    pub fn chrome(self) -> Chrome {
        let (bg, fg, subtle, panel) = match self {
            Theme::White => ("#fbfbfd", "#1c1c1e", "#8e8e93", "#ececf0"),
            Theme::Sepia | Theme::Omarchy => ("#f6efe1", "#4f321c", "#96785f", "#e8ddc6"),
            Theme::Grey => ("#d6d6d2", "#33332f", "#6e6e6a", "#c4c4bf"),
            Theme::Night => ("#111113", "#e5e5ea", "#6e6e73", "#232327"),
        };
        Chrome {
            bg: bg.into(),
            fg: fg.into(),
            subtle: subtle.into(),
            panel: panel.into(),
            accent: APPLE_BLUE.into(),
            font: BUNDLED_MONO,
        }
    }

    /// (background, text, subtle)
    fn colors(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Theme::White => ("#fdfdfe", "#1c1c1e", "#8e8e93"),
            Theme::Sepia | Theme::Omarchy => ("#f8f1e3", "#4f321c", "#96785f"),
            Theme::Grey => ("#d6d6d2", "#33332f", "#6e6e6a"),
            Theme::Night => ("#141416", "#e5e5ea", "#6e6e73"),
        }
    }
}

/// Reading surface parameters. Measure is fixed by design; only these three are
/// user-facing (CONTEXT.md §3).
#[derive(Clone)]
pub struct ReadingStyle {
    pub theme: Theme,
    /// Multiplier on the 20px base. Clamped to 0.8..=1.6.
    pub scale: f32,
    /// The palette Omarchy rendered (`omarchy_chrome`), if any. The chrome
    /// always uses it; the page only under `Theme::Omarchy`.
    pub omarchy: Option<Chrome>,
}

impl Default for ReadingStyle {
    fn default() -> Self {
        Self { theme: Theme::Sepia, scale: 1.0, omarchy: None }
    }
}

impl ReadingStyle {
    pub fn font_px(&self) -> f32 {
        20.0 * self.scale.clamp(0.8, 1.6)
    }

    /// The Omarchy palette, when it is the one the page is painted in.
    fn omarchy_page(&self) -> Option<&Chrome> {
        self.omarchy.as_ref().filter(|_| self.theme == Theme::Omarchy)
    }

    /// Page background, for painting the window ground.
    pub fn background_rgb(&self) -> [u8; 3] {
        match self.omarchy_page() {
            Some(c) => hex_rgb(&c.bg),
            None => self.theme.background_rgb(),
        }
    }

    /// A dark page: Night, or an Omarchy theme with a dark background.
    pub fn dark(&self) -> bool {
        let [r, g, b] = self.background_rgb();
        (299 * r as u32 + 587 * g as u32 + 114 * b as u32) < 128_000
    }

    /// The app chrome palette: Omarchy's when it rendered one, the reading
    /// theme's otherwise. Every chrome surface goes through here.
    pub fn chrome(&self) -> Chrome {
        self.omarchy.clone().unwrap_or_else(|| self.theme.chrome())
    }

    /// The UA stylesheet for the reading surface.
///
/// Every selector is prefixed with `html` on purpose. blitz-dom applies its own
/// default UA sheet *after* this one, so a bare `body {{ }}` here loses to its
/// `body {{ margin: 8px }}` and a bare `p {{ }}` loses to its `p {{ margin: 1em 0 }}`.
/// The descendant prefix raises specificity just enough to win.
    pub fn stylesheet(&self) -> String {
        let (bg, fg, subtle) = match self.omarchy_page() {
            Some(c) => (c.bg.clone(), c.fg.clone(), c.subtle.clone()),
            None => {
                let (bg, fg, subtle) = self.theme.colors();
                (bg.into(), fg.into(), subtle.into())
            }
        };
        let px = self.font_px();
        let gutter = GUTTER_EM;
        let measure = MEASURE_EM + 2.0 * GUTTER_EM;
        // Code is set in the chrome's face: the desktop's monospace on
        // Omarchy, the bundled one elsewhere (`Chrome::font`).
        let mono = self.chrome().font;
        // Line art is often black on transparent (SICP's register machines),
        // drawn for white paper; on a dark page it vanishes. Give images the
        // text colour as a backdrop there, with room so ink at the edge is not
        // cut flush. An opaque image hides all but that margin.
        let img_bg = match self.dark() {
            true => format!("background: {fg}; padding: 0.3em; border-radius: 4px;"),
            false => String::new(),
        };

        format!(
            r#"
html {{
  box-sizing: border-box;
  font-family: "Literata", "Charis SIL", serif;
  font-size: {px}px;
  line-height: 1.6;
  background: {bg};
  color: {fg};
}}
*, *::before, *::after {{ box-sizing: inherit; }}

html body {{
  /* The measure is 33em of text; border-box means the gutters live inside the
     max-width, so the column is exactly 33em wide however the padding changes.
     `margin: 0 auto` centres it in one-column mode at every window width. */
  max-width: {measure}em;
  margin: 0 auto;
  /* Horizontal only. Vertical breathing room is the page's job, not the
     document's — see PAGE_MARGIN_EM. */
  padding: 0 {gutter}em;
  text-align: justify;
  hyphens: auto;
}}

/* Editorial convention: indent, no inter-paragraph gap. */
p {{ margin: 0; text-indent: 1.2em; }}
h1 + p, h2 + p, h3 + p, h4 + p, h5 + p, h6 + p,
blockquote + p, hr + p, p:first-child {{ text-indent: 0; }}

html h1,
html h2,
html h3,
html h4,
html h5,
html h6 {{
  text-align: left;
  hyphens: none;
  font-weight: 600;
  margin: 2em 0 0.6em 0;
  line-height: 1.25;
}}
h1 {{ font-size: 1.5em; }}
h2 {{ font-size: 1.3em; }}
h3 {{ font-size: 1.15em; }}
h4, h5, h6 {{ font-size: 1em; }}

blockquote {{ margin: 1em 2em; font-size: 0.95em; }}
blockquote p {{ text-indent: 0; }}

/* No sideways in a paginated reader: code wraps, never scrolls. */
html pre, html code, html kbd, html samp {{ font-family: {mono}; }}
html pre {{
  white-space: pre-wrap;
  overflow-wrap: break-word;
  font-size: 0.9em;
  line-height: 1.45;
  text-align: left;
  hyphens: none;
  padding: 0.8em 1em;
  margin: 1.2em 0;
  border-radius: 6px;
  background: rgba(127, 127, 127, 0.12);
}}
code {{ font-size: 0.9em; }}

html img,
html svg {{
  /* Not block: blitz stretches a block image to the column, so a 2×9 glyph
     image fills a page. A table box is centred by auto margins and keeps
     the image's own size. */
  display: table;
  margin: 1.2em auto;
  max-width: 100%;
  max-height: 85vh;
  {img_bg}
}}
figure {{ margin: 1.4em 0; }}
html figcaption {{
  font-size: 0.85em;
  color: {subtle};
  text-align: center;
  text-indent: 0;
  hyphens: none;
}}

ul, ol {{ margin: 1em 0; padding-left: 1.6em; }}
li {{ text-indent: 0; margin: 0.3em 0; }}

html table {{
  border-collapse: collapse;
  margin: 1.4em 0;
  font-size: 0.9em;
  text-align: left;
  hyphens: none;
}}
html th,
html td {{
  border-bottom: 1px solid rgba(127, 127, 127, 0.3);
  padding: 0.45em 0.7em;
  text-indent: 0;
}}
th {{ font-weight: 600; }}

html a {{ color: inherit; text-decoration-color: {subtle}; }}
html hr {{
  border: 0;
  border-top: 1px solid rgba(127, 127, 127, 0.3);
  margin: 2em auto;
  width: 30%;
}}
/* A note reference is a superscript, and blitz's own UA sheet does not make it
   one — the number sat on the baseline at full size, reading as part of the
   sentence. */
sup, sub {{ line-height: 0; font-size: 0.72em; }}
sup {{ vertical-align: super; }}
sub {{ vertical-align: sub; }}
"#
        )
    }
}

/// Strip the publisher's CSS.
///
/// This *is* the whitelist policy (CONTEXT.md §3), and it also removes the
/// commonest cause of blitz-dom panicking on real books: a `<link>` or `<style>`
/// whose href cannot be resolved.
pub fn strip_publisher_css(html: &str) -> String {
    let without_style = drop_spans(html, "<style", "</style>");
    drop_link_elements(&without_style)
}

fn drop_link_elements(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(i) = rest.find("<link") {
        out.push_str(&rest[..i]);
        match rest[i..].find('>') {
            Some(j) => rest = &rest[i + j + 1..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Drop every `open..close` span, inclusive of the delimiters.
fn drop_spans(s: &str, open: &str, close: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(open) {
        out.push_str(&rest[..i]);
        match rest[i..].find(close) {
            Some(j) => rest = &rest[i + j + close.len()..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// User settings that outlive one book — currently the font size, which is a
/// property of the reader's eyes, not of the book (§3 calls it one of three
/// controls, and a control that forgets is not one).
///
/// ponytail: `key=value` lines, hand-parsed. `folders.txt` said to add a real
/// config format when there was a second thing to configure; this is it, and it
/// is twelve lines rather than a TOML dependency.
fn settings_file() -> std::path::PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("omaread/settings.txt")
}

pub fn setting(key: &str) -> Option<String> {
    setting_in(&settings_file(), key)
}

/// Write one setting, leaving the others alone.
pub fn set_setting(key: &str, value: &str) {
    set_setting_in(&settings_file(), key, value);
}

fn setting_in(path: &std::path::Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines()
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| k.trim() == key)
        .map(|(_, v)| v.trim().to_string())
}

fn set_setting_in(path: &std::path::Path, key: &str, value: &str) {
    let old = std::fs::read_to_string(path).unwrap_or_default();
    let mut lines: Vec<String> = old
        .lines()
        .filter(|l| l.split_once('=').is_none_or(|(k, _)| k.trim() != key))
        .map(str::to_string)
        .collect();
    lines.push(format!("{key}={value}"));

    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Settings are a convenience; failing to save one must not stop anything.
    if let Err(e) = std::fs::write(path, lines.join("\n") + "\n") {
        eprintln!("omaread: could not save {key}: {e}");
    }
}

/// The accent off Omarchy: the system blue Apple Books uses.
pub const APPLE_BLUE: &str = "#0a84ff";

/// The chrome face off Omarchy. Bundled, so it is never fontconfig's guess.
const BUNDLED_MONO: &str = r#""IBM Plex Mono", monospace"#;

/// App chrome: library, contents, HUD, menus. Colours are CSS `#rrggbb`
/// strings so the stylesheets and the painted overlays cannot drift apart.
#[derive(Clone, Debug, PartialEq)]
pub struct Chrome {
    pub bg: String,
    pub fg: String,
    pub subtle: String,
    pub panel: String,
    /// Selection, caret, progress: the theme's own on Omarchy.
    pub accent: String,
    /// Chrome is set in monospace, as Omarchy's own shell is. On Omarchy that
    /// is fontconfig's `monospace` alias, which `omarchy-font-set` writes and
    /// fontique resolves the generic family through.
    pub font: &'static str,
}

impl Chrome {
    /// `fg` over `bg` at `alpha`, flattened to an opaque `#rrggbb`. Omarchy's
    /// control states are foreground tints (`[controls]` in its shell.toml):
    /// 0.04 at rest, 0.08 hovered, 0.18 selected, 0.4 for a border.
    pub fn tint(&self, alpha: f32) -> String {
        mix(&self.fg, &self.bg, alpha)
    }
}

/// Where Omarchy leaves the palette it rendered for us.
const OMARCHY_CSS: &str = ".local/state/omarchy/current/theme/omaread.css";

/// The chrome palette Omarchy rendered, if there is one.
///
/// Omarchy renders every `~/.config/omarchy/themed/*.tpl` into the active theme
/// directory on a theme change, so the whole integration is a file read and four
/// custom properties (CONTEXT.md §11). Nothing here parses a theme, and nothing
/// has to track upstream's format. `None` off Omarchy, or when the template is
/// not installed — the reader then uses its own palette.
pub fn omarchy_chrome() -> Option<Chrome> {
    let home = std::env::var_os("HOME")?;
    let path = std::path::Path::new(&home).join(OMARCHY_CSS);
    let chrome = parse_chrome(&std::fs::read_to_string(path).ok()?)?;
    // Not the rendered `--subtle`: that is a terminal's `muted`, which several
    // themes put barely off the background (nord, hackerman) — illegible for a
    // HUD label or a caption. Text faded toward the ground reads in all of them.
    Some(Chrome { subtle: chrome.tint(0.6), ..chrome })
}

/// Call `changed` whenever Omarchy puts a new theme in place, from a thread of
/// its own, for as long as the process lives.
///
/// `omarchy theme set` renders every template into `next-theme` and only then
/// renames it to `theme`, so that rename is the moment the palette is ready.
/// The window regaining focus is not: the theme menu gives it back before the
/// templates are done, and the old palette was read. Nothing happens off
/// Omarchy.
pub fn watch_omarchy(changed: impl Fn() + Send + 'static) {
    let Some(home) = std::env::var_os("HOME") else { return };
    // `…/current/theme/omaread.css` → `…/current`, where the rename lands.
    if let Some(dir) = std::path::Path::new(&home).join(OMARCHY_CSS).ancestors().nth(2) {
        watch_theme_swap(dir.to_owned(), changed);
    }
}

/// `changed` on every `theme` created or renamed into `dir`.
fn watch_theme_swap(dir: std::path::PathBuf, changed: impl Fn() + Send + 'static) {
    use rustix::fs::inotify;
    std::thread::spawn(move || {
        let Ok(fd) = inotify::init(inotify::CreateFlags::CLOEXEC) else { return };
        let watch = inotify::WatchFlags::MOVED_TO | inotify::WatchFlags::CREATE;
        if inotify::add_watch(&fd, &dir, watch).is_err() {
            return;
        }
        let mut buf = [std::mem::MaybeUninit::uninit(); 1024];
        let mut events = inotify::Reader::new(&fd, &mut buf);
        loop {
            match events.next() {
                Ok(e) if e.file_name().is_some_and(|n| n.to_bytes() == b"theme") => changed(),
                Ok(_) | Err(rustix::io::Errno::INTR) => {}
                Err(_) => return,
            }
        }
    });
}

/// Where `omarchy display text size` keeps the desktop's text size.
const OMARCHY_SHELL: &str = ".config/omarchy/shell.toml";

/// How much bigger than stock the chrome should be, by Omarchy's own rule.
///
/// `omarchy display text size` is one knob for the whole desktop: a px
/// `base-size` under `[font]`, anchored at 12px, from which the shell, GTK's
/// `text-scaling-factor` and the terminals all derive. Ours derives the same
/// way — 12px is 1.0 — and multiplies the monitor scale, which winit already
/// hands over. 1.0 off Omarchy, or with the size never set.
pub fn omarchy_ui_scale() -> f32 {
    let home = std::env::var_os("HOME");
    home.and_then(|h| std::fs::read_to_string(std::path::Path::new(&h).join(OMARCHY_SHELL)).ok())
        .and_then(|toml| base_size(&toml))
        .map_or(1.0, |px| px / 12.0)
}

/// `base-size` under `[font]`, held to the 9–20px Omarchy itself accepts.
///
/// ponytail: a line scan, not a TOML parser. It is one key in one section;
/// take the `toml` crate if a second Omarchy setting is ever read.
fn base_size(toml: &str) -> Option<f32> {
    let mut in_font = false;
    for line in toml.lines().map(|l| l.split('#').next().unwrap_or("").trim()) {
        if line.starts_with('[') {
            in_font = line == "[font]";
        } else if let Some(("base-size", v)) = line.split_once('=').map(|(k, v)| (k.trim(), v)) {
            if in_font {
                return v.trim().parse::<f32>().ok().map(|px| px.clamp(9.0, 20.0));
            }
        }
    }
    None
}

/// `#rrggbb` as components. Only ever given strings `parse_chrome` accepted.
fn hex_rgb(hex: &str) -> [u8; 3] {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0);
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

/// `t` of the way from `b` to `a`, as `#rrggbb`.
fn mix(a: &str, b: &str, t: f32) -> String {
    let (a, b) = (hex_rgb(a), hex_rgb(b));
    let c = |i: usize| (a[i] as f32 * t + b[i] as f32 * (1.0 - t)).round() as u8;
    format!("#{:02x}{:02x}{:02x}", c(0), c(1), c(2))
}

/// Pull `--bg`, `--fg`, `--subtle`, `--panel` and `--accent` out of a rendered
/// stylesheet.
///
/// Anything not a `#rrggbb` is rejected rather than passed on: a half-rendered
/// template would otherwise paint the whole library black, since that is what an
/// unparseable colour becomes. `--accent` alone may be missing — a template
/// installed before it existed — and is then Apple's blue.
fn parse_chrome(css: &str) -> Option<Chrome> {
    let value = |name: &str| -> Option<String> {
        let key = format!("--{name}:");
        let at = css.find(&key)? + key.len();
        let raw = css[at..].split(';').next()?.trim();
        let hex = raw.len() == 7
            && raw.starts_with('#')
            && raw[1..].chars().all(|c| c.is_ascii_hexdigit());
        hex.then(|| raw.to_string())
    };
    Some(Chrome {
        bg: value("bg")?,
        fg: value("fg")?,
        subtle: value("subtle")?,
        panel: value("panel")?,
        accent: value("accent").unwrap_or_else(|| APPLE_BLUE.into()),
        font: "monospace",
    })
}

#[cfg(test)]
mod tests {
    /// What `omarchy theme set` does once the templates are rendered: a whole
    /// `next-theme` renamed to `theme` over the old one. The rename is the
    /// signal, and it has to arrive.
    #[test]
    fn a_theme_swapped_into_place_is_noticed() {
        let dir = std::env::temp_dir().join(format!("omaread-theme-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("theme")).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        super::watch_theme_swap(dir.clone(), move || { let _ = tx.send(()); });
        // The watch is set up on its own thread; let it get there.
        std::thread::sleep(std::time::Duration::from_millis(100));

        std::fs::create_dir(dir.join("next-theme")).unwrap();
        std::fs::write(dir.join("next-theme/omaread.css"), ":root {}").unwrap();
        std::fs::remove_dir_all(dir.join("theme")).unwrap();
        std::fs::rename(dir.join("next-theme"), dir.join("theme")).unwrap();

        let got = rx.recv_timeout(std::time::Duration::from_secs(2));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(got.is_ok(), "the rename went unnoticed");
        assert!(rx.try_recv().is_err(), "making next-theme is not the signal");
    }

    /// Only `[font]`'s base-size counts, and only within Omarchy's own range.
    #[test]
    fn the_omarchy_text_size_is_read_from_the_font_section() {
        use super::base_size;
        assert_eq!(base_size("[font]\nbase-size = 14\n"), Some(14.0));
        assert_eq!(base_size("[bar]\nbase-size = 18\n[font]\nfamily = x\n"), None);
        assert_eq!(base_size("[font]\nbase-size = 40 # huge\n"), Some(20.0));
        assert_eq!(base_size(""), None);
    }

    /// A setting has to survive being written next to another one, or changing
    /// the font size would forget the watched folders' neighbours.
    #[test]
    fn settings_round_trip_without_clobbering_each_other() {
        // Takes a path rather than setting XDG_CONFIG_HOME: mutating the
        // environment would race every other test in the process.
        let dir = std::env::temp_dir().join(format!("omaread-set-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.txt");

        super::set_setting_in(&path, "font-scale", "1.2");
        super::set_setting_in(&path, "something-else", "keep me");
        super::set_setting_in(&path, "font-scale", "0.9");

        assert_eq!(super::setting_in(&path, "font-scale").as_deref(), Some("0.9"));
        assert_eq!(super::setting_in(&path, "something-else").as_deref(), Some("keep me"));
        assert_eq!(super::setting_in(&path, "never-set"), None);
        // A missing file is not an error, just no settings.
        assert_eq!(super::setting_in(&dir.join("nope.txt"), "font-scale"), None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The rendered template is the whole Omarchy integration, so its parse is
    /// the thing that must not quietly do the wrong thing.
    #[test]
    fn the_omarchy_palette_is_read_or_refused() {
        let good = ":root {\n  --bg: #2e3440;\n  --fg: #d8dee9;\n\
                    --subtle: #4c566a;\n  --panel: #434c5e;\n  --accent: #81a1c1;\n}\n";
        assert_eq!(
            super::parse_chrome(good),
            Some(super::Chrome {
                bg: "#2e3440".into(),
                fg: "#d8dee9".into(),
                subtle: "#4c566a".into(),
                panel: "#434c5e".into(),
                accent: "#81a1c1".into(),
                font: "monospace",
            })
        );
        // A template rendered before `--accent` existed still works.
        let old = good.replace("  --accent: #81a1c1;\n", "");
        assert_eq!(super::parse_chrome(&old).map(|c| c.accent), Some(super::APPLE_BLUE.into()));

        // A key that merely starts the same must not be mistaken for `--bg`.
        assert_eq!(super::parse_chrome("--background: #2e3440;"), None);
        // Missing one property is not a palette.
        assert_eq!(super::parse_chrome("--bg: #2e3440; --fg: #fff;"), None);
        // An unrendered template would otherwise become black.
        assert_eq!(
            super::parse_chrome(
                "--bg: {{ background }}; --fg: #d8dee9; --subtle: #4c566a; --panel: #434c5e;"
            ),
            None
        );
    }

    use super::*;

    #[test]
    fn strips_style_and_link() {
        let html = r#"<html><head>
            <link rel="stylesheet" href="stylesheet.css"/>
            <style>p { color: red }</style>
            <title>keep</title></head><body><p>text</p></body></html>"#;
        let out = strip_publisher_css(html);
        assert!(!out.contains("<link"), "link survived: {out}");
        assert!(!out.contains("<style"), "style survived: {out}");
        assert!(!out.contains("color: red"), "css body survived: {out}");
        assert!(out.contains("<title>keep</title>"));
        assert!(out.contains("<p>text</p>"));
    }

    #[test]
    fn survives_unterminated_markup() {
        assert!(!strip_publisher_css("<p>a</p><style>oops").contains("oops"));
        assert!(!strip_publisher_css("<p>a</p><link rel=x").contains("<link"));
    }

    #[test]
    fn font_scale_is_clamped() {
        let s = ReadingStyle { theme: Theme::White, scale: 99.0, omarchy: None };
        assert_eq!(s.font_px(), 32.0);
        let s = ReadingStyle { theme: Theme::White, scale: 0.01, omarchy: None };
        assert_eq!(s.font_px(), 16.0);
    }

    #[test]
    fn stylesheet_reflects_theme_and_scale() {
        let s = ReadingStyle { theme: Theme::Night, scale: 1.2, omarchy: None };
        let css = s.stylesheet();
        assert!(css.contains("#141416"), "night background missing");
        assert!(css.contains("font-size: 24px"), "scale not applied");
        assert!(css.contains(r#"font-family: "IBM Plex Mono""#), "bundled mono off Omarchy");
        assert!(s.dark());
        assert!(css.contains("background: #e5e5ea"), "no backdrop for transparent figures");
        let sepia = ReadingStyle { theme: Theme::Sepia, ..s };
        assert!(!sepia.stylesheet().contains("padding: 0.3em"));
    }

    /// The page follows the Omarchy palette only when that theme is chosen,
    /// and falls back to Sepia off Omarchy rather than painting nothing.
    #[test]
    fn omarchy_theme_paints_the_page_in_the_desktop_palette() {
        let nord = parse_chrome("--bg: #2e3440; --fg: #d8dee9; --subtle: #4c566a; --panel: #434c5e;");
        let s = ReadingStyle { theme: Theme::Omarchy, scale: 1.0, omarchy: nord };
        let css = s.stylesheet();
        assert!(css.contains("background: #2e3440") && css.contains("color: #d8dee9"));
        assert_eq!(s.background_rgb(), [0x2e, 0x34, 0x40]);
        assert!(s.dark());

        let paper = ReadingStyle { theme: Theme::Sepia, ..s };
        assert!(!paper.stylesheet().contains("#2e3440"), "a paper theme took the desktop's");
        assert_eq!(paper.chrome().bg, "#2e3440", "chrome stops following Omarchy");
        assert!(paper.stylesheet().contains("font-family: monospace;"), "desktop mono in any theme");

        let off = ReadingStyle { theme: Theme::Omarchy, ..ReadingStyle::default() };
        assert_eq!(off.background_rgb(), Theme::Sepia.background_rgb());
        assert!(!off.dark());
        assert_eq!(mix("#ffffff", "#000000", 0.5), "#808080");
    }
}
