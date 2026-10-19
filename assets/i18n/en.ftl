# Omaread — English. The reference language: every other file carries these ids.

## Library

books-count = { $n ->
    [one] { $n } book
   *[other] { $n } books
}
library-matching = { $n } matching “{ $query }”
library-matching-split = “{ $query }”: { $named } in title or author · { $text } in the text
library-in-text = in the text
library-title = Library ({ $heading })
library-empty = Nothing here yet. Put some .epub, .azw3 or .mobi files in a watched folder and press r to rescan.
library-file-missing = file missing
library-quarantined = crashed the reader
open-quarantined = { $title } crashed the reader on two openings. Open it again to read it anyway.
sort-recent = recent
sort-title = title
sort-author = author
search-hint = Type to search
search-tag = tag
suggest-book = book
suggest-author = author

## Settings panel

settings-theme = Theme
settings-folders = Folders scanned for books
settings-add-folder = Add a folder…
settings-open-book = Open a book…
picker-folder = Choose a folder of books
picker-book = Open a book
settings-help = Enter to add · click a folder to drop it · F5 rescans · Esc closes
theme-omarchy = Omarchy
theme-white = White
theme-sepia = Sepia
theme-grey = Grey
theme-night = Night

## Reading HUD

hud-bookmark = Bookmark
hud-bookmarked = Bookmarked
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
mark-page = page { $n }
marks-exported = Highlights and notes saved to { $path }
marks-export-failed = Could not save the highlights: { $error }
chapter-n = Chapter { $n }

## Screen reader: names for controls that show a glyph or nothing

a11y-settings = Settings
a11y-actions = Actions
a11y-smaller = Smaller text
a11y-bigger = Larger text
a11y-columns = Columns: { $n }
settings-drop-folder = Stop scanning { $folder }

## Notices

nothing-selected = Nothing is selected. Drag across some text first, then try again.
columns-too-narrow = The window is too narrow for two columns. Widen it, or make the text smaller, and the second column appears.
fixed-layout-notice = This book is laid out page by page, like a comic or a picture book. Omaread reflows it into its own pages, so it will not look as it was designed.
a11y-figure = Figure
link-leave = This link leaves the book for { $url }. Press Enter to open it in your browser; any other key or a click stays here.

## Keys overlay (? or F1)

help-title = Keys
help-dismiss = Any key or click closes this
help-move = Move
help-open = Open the book
help-search = Search
help-clear-quit = Clear the search, or quit
help-page = Previous or next page
help-sort = Change the order
help-tag = Tag the book
help-open-file = Open a book from a file
help-rescan = Rescan the folders
help-settings = Settings
help-turn = Turn the page
help-chapter = Previous or next chapter
help-goto = Go to a page or percentage
help-contents = Contents
help-find = Search the book
help-marks = Bookmarks and highlights
help-bookmark = Bookmark the page
help-highlight = Highlight the selection
help-copy = Copy the selection
help-theme = Next theme
help-size = Text size
help-columns = One or two columns
help-menu = Menu
help-library = Library
help-quit = Quit
help-go = Go there
help-back = Back to the book
help-note = Write a note
help-delete = Delete
help-export = Export highlights to Markdown
help-help = These keys
