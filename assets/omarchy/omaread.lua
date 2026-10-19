-- Omaread — suggested Hyprland window rules and keybinding.
--
-- Omarchy configures Hyprland in Lua. Install:
--   cp ~/Projects/omaread/assets/omarchy/omaread.lua ~/.config/hypr/omaread.lua
-- then add, below the other personal requires in ~/.config/hypr/hyprland.lua:
--   require("hypr.omaread")
--
-- The rules key on app_id `omaread`, which the binary sets on its window.

-- A book is a page, not a tiled column: float it at a page-ish aspect and
-- centre it. Tiling a reader next to a terminal gives you a 40-column measure,
-- which the two-column fallback then also refuses (CONTEXT.md §3).
--
-- Reading is a full-attention activity; no shadow, no blur behind 400 pages of
-- prose.
o.window("^omaread$", {
  float = true,
  center = true,
  size = { 900, 1000 },
  no_shadow = true,
  no_blur = true,
})

-- Super + Shift + L opens the library, or focuses it if it is already open.
-- Not Super + Shift + E: Omarchy binds that to Email.
o.bind("SUPER + SHIFT + L", "Omaread", { launch = "omaread", focus = "^omaread$" })
