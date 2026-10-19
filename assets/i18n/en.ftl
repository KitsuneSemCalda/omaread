# Omaread — English. The reference language: every other file carries these ids.

## Library

books-count = { $n ->
    [one] { $n } book
   *[other] { $n } books
}
library-matching = { $n } matching “{ $query }”
library-title = Library ({ $heading })
library-sorted-by = sorted by { $sort }
library-empty = Nothing here yet. Put some .epub, .azw3 or .mobi files in a watched folder and press r to rescan.
library-file-missing = file missing
sort-recent = Recent
sort-title = Title
sort-author = Author
search-hint = Type to search
search-tag = tag
suggest-book = book
suggest-author = author

## Settings panel

settings-theme = Theme
settings-folders = Folders scanned for books
settings-add-folder = Add a folder…
settings-help = Enter to add · click a folder to drop it · F5 rescans · Esc closes
theme-omarchy = Omarchy
theme-white = White
theme-sepia = Sepia
theme-grey = Grey
theme-night = Night

## Reading HUD

hud-bookmark = Bookmark
hud-contents = Contents
hud-highlight = Highlight
hud-remove = Remove
hud-columns = { $n } col
hud-library = Library
hud-page-of = page { $page } of { $total }
hud-back-to-page = Back to page { $page }
hud-back-to-percent = Back to { $percent }%

## Context menu

menu-copy = Copy
menu-highlight = Highlight
menu-remove-highlight = Remove highlight
menu-add-note = Add note…
menu-bookmark-page = Bookmark page
menu-remove-bookmark = Remove bookmark

## Contents, marks and search

toc-contents = Contents
toc-marks = Marks
toc-search = Search
toc-nothing-found = Nothing found.
toc-marks-count = { $n ->
    [one] { $n } bookmark or highlight
   *[other] { $n } bookmarks and highlights
}
toc-note-typing = note: { $note }|
toc-search-hits = “{ $query }” — { $n ->
    [one] { $n } hit
   *[other] { $n } hits
} in this book
mark-bookmark = Bookmark
mark-highlight = Highlight
mark-chapter = chapter { $n }
chapter-n = Chapter { $n }

## Screen reader: names for controls that show a glyph or nothing

a11y-settings = Settings
a11y-actions = Actions
a11y-smaller = Smaller text
a11y-bigger = Larger text
a11y-columns = Columns: { $n }
settings-drop-folder = Stop scanning { $folder }

## Notices

fixed-layout-notice = This book is laid out page by page, like a comic or a picture book. Omaread reflows it into its own pages, so it will not look as it was designed.
a11y-figure = Figure
link-leave = This link leaves the book for { $url }. Press Enter to open it in your browser; any other key or a click stays here.
