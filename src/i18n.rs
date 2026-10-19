//! UI strings, through Fluent (CONTEXT.md §12).
//!
//! Every user-facing string is a message in `assets/i18n/<lang>.ftl`, looked up
//! with `tr!`. The language is the POSIX locale's; English stands in for any
//! message a translation lacks, so a half-done translation still reads.
//!
//! Adding a language is a `.ftl` file and a line in `LANGS`. The test below
//! fails until the file carries every message English has.

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource};
use std::sync::OnceLock;

/// Shipped languages, English first.
const LANGS: &[(&str, &str)] = &[
    ("en", include_str!("../assets/i18n/en.ftl")),
    ("es", include_str!("../assets/i18n/es.ftl")),
];

type Bundle = FluentBundle<FluentResource>;

/// A message, with its arguments: `tr!("hud-page-of", page = 3, total = 9)`.
macro_rules! tr {
    ($id:expr) => {
        $crate::i18n::lookup($id, None)
    };
    ($id:expr, $($key:ident = $value:expr),+ $(,)?) => {{
        let mut args = fluent_bundle::FluentArgs::new();
        $(args.set(stringify!($key), $value);)+
        $crate::i18n::lookup($id, Some(&args))
    }};
}
pub(crate) use tr;

pub fn lookup(id: &str, args: Option<&FluentArgs>) -> String {
    static BUNDLES: OnceLock<(Bundle, Bundle)> = OnceLock::new();
    let (chosen, english) = BUNDLES.get_or_init(|| {
        // Tests read English whatever the machine's locale.
        let lang = if cfg!(test) { "en" } else { language(&locale()) };
        (bundle(lang), bundle("en"))
    });
    format(chosen, id, args)
        .or_else(|| format(english, id, args))
        // A missing message shows its id rather than nothing.
        .unwrap_or_else(|| id.to_string())
}

fn format(bundle: &Bundle, id: &str, args: Option<&FluentArgs>) -> Option<String> {
    let pattern = bundle.get_message(id)?.value()?;
    let mut errors = Vec::new();
    Some(bundle.format_pattern(pattern, args, &mut errors).into_owned())
}

fn bundle(lang: &str) -> Bundle {
    let src = LANGS.iter().find(|(l, _)| *l == lang).map_or(LANGS[0].1, |(_, s)| s);
    let mut bundle = FluentBundle::new_concurrent(vec![lang.parse().expect("language tag")]);
    // Fluent wraps arguments in U+2068/U+2069 for bidi; they would paint as
    // tofu, and nothing here is right-to-left.
    bundle.set_use_isolating(false);
    let resource = FluentResource::try_new(src.to_string()).expect("ftl parses");
    bundle.add_resource(resource).expect("no duplicate messages");
    bundle
}

/// The locale for messages, by POSIX precedence.
fn locale() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .find(|v| !v.is_empty())
        .unwrap_or_default()
}

/// `es_ES.UTF-8` → `es`, when Omaread speaks it; English otherwise.
fn language(locale: &str) -> &'static str {
    let lang = locale.split(['_', '.', '@']).next().unwrap_or_default();
    LANGS.iter().map(|(l, _)| *l).find(|l| *l == lang).unwrap_or("en")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(src: &str) -> Vec<&str> {
        src.lines()
            .filter_map(|l| l.split_once(" ="))
            .map(|(id, _)| id)
            .filter(|id| id.starts_with(|c: char| c.is_ascii_lowercase()))
            .collect()
    }

    #[test]
    fn every_language_parses_and_has_every_message() {
        let english = ids(LANGS[0].1);
        assert!(english.len() > 10, "{english:?}");
        for (lang, src) in LANGS {
            bundle(lang);
            let have = ids(src);
            let missing: Vec<_> = english.iter().filter(|id| !have.contains(id)).collect();
            assert!(missing.is_empty(), "{lang}.ftl lacks {missing:?}");
        }
    }

    #[test]
    fn plurals_and_arguments() {
        assert_eq!(tr!("books-count", n = 1), "1 book");
        assert_eq!(tr!("books-count", n = 3), "3 books");
        assert_eq!(tr!("hud-page-of", page = 3, total = 9), "page 3 of 9");

        let es = bundle("es");
        let mut args = FluentArgs::new();
        args.set("n", 1);
        assert_eq!(format(&es, "books-count", Some(&args)).unwrap(), "1 libro");
        args.set("n", 2);
        assert_eq!(format(&es, "books-count", Some(&args)).unwrap(), "2 libros");
        assert_eq!(tr!("no-such-message"), "no-such-message");
    }

    #[test]
    fn the_locale_picks_the_language() {
        assert_eq!(language("es_ES.UTF-8"), "es");
        assert_eq!(language("es"), "es");
        assert_eq!(language("en_US.UTF-8"), "en");
        assert_eq!(language("ca_ES@valencia"), "en", "not shipped yet");
        assert_eq!(language("C"), "en");
        assert_eq!(language(""), "en");
    }
}
