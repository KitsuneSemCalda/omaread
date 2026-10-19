#!/usr/bin/env bash
# The conformance corpus (CONTEXT.md §13): fetch it, then run `omaread --check`
# over every book in it. Sources are pinned, so a red run means omaread changed,
# not the corpus.
#
#   ci/corpus.sh [omaread binary] [cache dir]
set -euo pipefail

bin=$(realpath "${1:-target/release/omaread}")
dir=${2:-${XDG_CACHE_HOME:-$HOME/.cache}/omaread-corpus}

W3C=54092b4233253e9aac80e93ec4782b380b4b3403      # w3c/epub-tests
CALIBRE=a5f438bfa3feb463d855a7ce2282fab967e3e3da  # kovidgoyal/calibre
EPUBTEST=https://github.com/daisy/epub-accessibility-tests/releases/download
EPUBTEST_BOOKS=(
  fundamental-2.0/Fundamental-Accessibility-Tests-Basic-Functionality-v2.0.0.epub
  non-visual-reading-2.0.1/Fundamental-Accessibility-Tests-Non-Visual-Reading-v2.0.1.epub
  fundamental-2.0/Fundamental-Accessibility-Tests-Visual-Adjustments-v2.0.0.epub
  fundamental-2.0/Fundamental-Accessibility-Tests-Read-Aloud-v2.0.0.epub
  media-overlays-1.0/Advanced-Accessibility-Tests-Media-Overlays-v1.0.0.epub
  math-extdesc-1.1.1/Accessibility-Tests-Mathematics-v1.1.1.epub
  math-extdesc-1.1.1/Accessibility-Tests-Extended-Descriptions-v1.1.1.epub
  reading-history-navigation-experimental-1.0.0/Reading-History-Navigation-Experimental-v1.0.0.epub
)

mkdir -p "$dir/books"
cd "$dir"

# The W3C suite ships each test book unpacked; pack it the way an EPUB must be,
# `mimetype` first and stored.
if [ ! -e "w3c-$W3C" ]; then
  curl -fsSL "https://github.com/w3c/epub-tests/archive/$W3C.tar.gz" | tar xz
  mv "epub-tests-$W3C" "w3c-$W3C"
  for t in "w3c-$W3C"/tests/*/; do
    out="$PWD/books/w3c-$(basename "$t").epub"
    (cd "$t" && zip -qX0 "$out" mimetype && zip -qXr9D "$out" . -x mimetype)
  done
fi

for b in "${EPUBTEST_BOOKS[@]}"; do
  [ -e "books/epubtest-${b#*/}" ] || curl -fsSL -o "books/epubtest-${b#*/}" "$EPUBTEST/$b"
done

for b in resources/quick_start/eng.epub manual/resources/mathjax.epub; do
  out="books/calibre-$(basename "$b")"
  [ -e "$out" ] || curl -fsSL -o "$out" "https://raw.githubusercontent.com/kovidgoyal/calibre/$CALIBRE/$b"
done

# --check never touches the library, but keep a stray write out of $HOME anyway.
export XDG_DATA_HOME="$dir/xdg" XDG_CONFIG_HOME="$dir/xdg"
exec "$bin" --check books/*.epub
