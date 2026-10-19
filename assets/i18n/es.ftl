# Omaread — español.

## Biblioteca

books-count = { $n ->
    [one] { $n } libro
   *[other] { $n } libros
}
library-matching = { $n ->
    [one] { $n } resultado
   *[other] { $n } resultados
} para «{ $query }»
library-title = Biblioteca ({ $heading })
library-sorted-by = orden: { $sort }
library-empty = Todavía no hay nada. Pon archivos .epub, .azw3 o .mobi en una carpeta vigilada y pulsa r para volver a buscar.
library-file-missing = falta el archivo
sort-recent = recientes
sort-title = título
sort-author = autor
search-hint = Escribe para buscar
search-tag = etiqueta
suggest-book = libro
suggest-author = autor

## Ajustes

settings-theme = Tema
settings-folders = Carpetas donde buscar libros
settings-add-folder = Añadir una carpeta…
settings-help = Intro para añadir · clic en una carpeta para quitarla · F5 vuelve a buscar · Esc cierra
theme-omarchy = Omarchy
theme-white = Blanco
theme-sepia = Sepia
theme-grey = Gris
theme-night = Noche

## Lectura

hud-bookmark = Marcador
hud-contents = Índice
hud-highlight = Resaltar
hud-remove = Quitar
hud-columns = { $n } col.
hud-library = Biblioteca
hud-page-of = página { $page } de { $total }
hud-back-to-page = Volver a la página { $page }
hud-back-to-percent = Volver al { $percent } %

## Menú contextual

menu-copy = Copiar
menu-highlight = Resaltar
menu-remove-highlight = Quitar resaltado
menu-add-note = Añadir nota…
menu-bookmark-page = Marcar página
menu-remove-bookmark = Quitar marcador

## Índice, marcas y búsqueda

toc-contents = Índice
toc-marks = Marcas
toc-search = Buscar
toc-nothing-found = No se ha encontrado nada.
toc-marks-count = { $n ->
    [one] { $n } marcador o resaltado
   *[other] { $n } marcadores y resaltados
}
toc-note-typing = nota: { $note }|
toc-search-hits = «{ $query }» — { $n ->
    [one] { $n } resultado
   *[other] { $n } resultados
} en este libro
mark-bookmark = Marcador
mark-highlight = Resaltado
mark-chapter = capítulo { $n }
chapter-n = Capítulo { $n }
