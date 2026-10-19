/* Omaread chrome palette, rendered by Omarchy on a theme change.
 *
 * Install:  cp omaread.css.tpl ~/.config/omarchy/themed/
 * then switch theme once (omarchy-theme-set) to render it.
 *
 * Omarchy writes the result to
 *   ~/.local/state/omarchy/current/theme/omaread.css
 * which is where Omaread reads it. Press F5 in the library to pick up a new
 * theme without restarting.
 *
 * The app chrome always follows the system theme. The reading surface does too
 * by default (the "Omarchy" theme, CONTEXT.md §11); White/Sepia/Grey/Night stay
 * one `t` away.
 *
 * Omaread reads exactly these five properties and ignores everything else, so
 * it is safe to edit the mappings to taste. `--subtle` is required but not used:
 * Omaread fades `--fg` toward `--bg` instead, because `muted` is illegible as
 * text in several themes. `--accent` (selection, caret, progress) is optional;
 * without it Omaread uses Apple's blue.
 */

:root {
  --bg: {{ background }};
  --fg: {{ foreground }};
  --subtle: {{ muted }};
  --panel: {{ selection }};
  --accent: {{ accent }};
}
