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
library-empty = Todavía no hay nada. Pon archivos .epub, .azw3 o .mobi en una carpeta vigilada y pulsa r para volver a buscar.
library-file-missing = falta el archivo
library-quarantined = bloqueó el lector
open-quarantined = { $title } bloqueó el lector en dos aperturas. Ábrelo otra vez para leerlo de todos modos.
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
settings-open-book = Abrir un libro…
picker-folder = Elige una carpeta de libros
picker-book = Abrir un libro
settings-help = Intro para añadir · clic en una carpeta para quitarla · F5 vuelve a buscar · Esc cierra
theme-omarchy = Omarchy
theme-white = Blanco
theme-sepia = Sepia
theme-grey = Gris
theme-night = Noche

## Lectura

hud-bookmark = Marcador
hud-bookmarked = Marcada
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
mark-page = página { $n }
marks-exported = Resaltados y notas guardados en { $path }
marks-export-failed = No se pudieron guardar los resaltados: { $error }
chapter-n = Capítulo { $n }

## Lector de pantalla: nombres de controles que muestran un glifo o nada

a11y-settings = Ajustes
a11y-actions = Acciones
a11y-smaller = Texto más pequeño
a11y-bigger = Texto más grande
a11y-columns = Columnas: { $n }
settings-drop-folder = Dejar de buscar en { $folder }

## Avisos

columns-too-narrow = La ventana es demasiado estrecha para dos columnas. Ensánchala o reduce el texto y aparecerá la segunda.
fixed-layout-notice = Este libro está maquetado página a página, como un cómic o un álbum ilustrado. Omaread lo recompone en sus propias páginas, así que no se verá como fue diseñado.
a11y-figure = Figura
link-leave = Este enlace sale del libro hacia { $url }. Pulsa Intro para abrirlo en el navegador; cualquier otra tecla o un clic te deja aquí.

## Teclas (? o F1)

help-title = Teclas
help-dismiss = Cualquier tecla o clic cierra esto
help-move = Moverse
help-open = Abrir el libro
help-search = Buscar
help-clear-quit = Borrar la búsqueda, o salir
help-page = Página anterior o siguiente
help-sort = Cambiar el orden
help-tag = Etiquetar el libro
help-open-file = Abrir un libro desde un archivo
help-rescan = Volver a buscar en las carpetas
help-settings = Ajustes
help-turn = Pasar página
help-chapter = Capítulo anterior o siguiente
help-contents = Índice
help-find = Buscar en el libro
help-marks = Marcadores y resaltados
help-bookmark = Marcar la página
help-highlight = Resaltar la selección
help-copy = Copiar la selección
help-theme = Tema siguiente
help-size = Tamaño del texto
help-columns = Una o dos columnas
help-menu = Menú
help-library = Biblioteca
help-quit = Salir
help-go = Ir allí
help-back = Volver al libro
help-note = Escribir una nota
help-delete = Borrar
help-export = Exportar resaltados a Markdown
help-help = Estas teclas
