-- =============================================================================
-- WYRD WALLPAPER: DECLARATIVE WINDOW CONFIGURATION
-- =============================================================================
-- Rendered strictly by wyrd-engine (LuaRuntime + WidgetTree + Renderer).
-- Automatically inherits tokens/styles from your active theme in settings.toml,
-- while defining standalone base styles so it always renders even without external themes.
-- =============================================================================

wyrd.style("global", {
    default    = true,
    background = "rgba(0, 0, 0, 0)",
    foreground = "#ebebf0",
    accent     = "#92b2fc",
    font       = "JetBrains Mono Nerd Font, MesloLGS Nerd Font, sans-serif",
    font_size  = 11,
})

wyrd.style("popup", {
    background = "rgba(22, 24, 32, 0.94)",
    foreground = "#ebebf0",
    outline    = "1px solid rgba(146, 178, 252, 0.24)",
    radius     = 20,
    padding    = { horizontal = 18, vertical = 18 },
    font       = "JetBrains Mono Nerd Font, MesloLGS Nerd Font, sans-serif",
    font_size  = 11,
})

wyrd.style("header_title", {
    foreground  = "#ebebf0",
    font_size   = 13,
    font_weight = "bold",
})

wyrd.style("chip", {
    background = "rgba(30, 32, 41, 0.85)",
    foreground = "#c4c6d0",
    accent     = "#92b2fc",
    outline    = "1px solid rgba(146, 178, 252, 0.22)",
    radius     = 9999,
    padding    = { horizontal = 12, vertical = 4 },
    font_size  = 11,
    hover      = {
        background = "rgba(46, 52, 66, 0.92)",
        foreground = "#ebebf0",
        outline    = "1px solid rgba(146, 178, 252, 0.52)",
    },
})

wyrd.style("quick_toggle", {
    background = "rgba(30, 32, 41, 0.90)",
    foreground = "#c4c6d0",
    outline    = "1px solid rgba(146, 178, 252, 0.24)",
    radius     = 14,
    padding    = { horizontal = 12, vertical = 8 },
    font_size  = 11,
    hover      = {
        background = "rgba(46, 52, 66, 0.92)",
        foreground = "#ebebf0",
        outline    = "1px solid rgba(146, 178, 252, 0.52)",
    },
})

wyrd.style("media_control_btn", {
    background = "rgba(30, 32, 41, 0.82)",
    foreground = "#c4c6d0",
    radius     = 9999,
    font_size  = 12,
    hover      = {
        background = "rgba(46, 52, 66, 0.92)",
        foreground = "#ebebf0",
    },
})

wyrd.style("media_control_btn_primary", {
    background = "#92b2fc",
    foreground = "#111318",
    radius     = 9999,
    font_size  = 12,
    hover      = {
        background = "#b9c2df",
        foreground = "#111318",
    },
})

local ok_settings, settings_mod = pcall(require, "settings")
local settings = (ok_settings and settings_mod and settings_mod.load and settings_mod.load()) or {
    theme = "catppuccin",
}

local theme_name = settings.theme or "catppuccin"
local ok_theme, theme_mod = pcall(require, "themes." .. theme_name)
if ok_theme and theme_mod and type(theme_mod.apply) == "function" then
    pcall(theme_mod.apply)
else
    pcall(function()
        require("themes.catppuccin").apply()
    end)
end

wyrd.style("wallpaper_window", {
    extends = "popup",
    border_radius = 20,
    padding = { 18, 20, 18, 20 },
})

wyrd.style("wallpaper_subtitle", {
    font_size = 12,
    opacity = 0.78,
})

wyrd.style("wallpaper_hint", {
    font_size = 11,
    opacity = 0.65,
})

wyrd.style("wallpaper_card_active", {
    extends = "quick_toggle",
    border_radius = 14,
    padding = { 4, 4, 4, 4 },
})

wyrd.style("wallpaper_card_side", {
    extends = "chip",
    border_radius = 12,
    opacity = 0.72,
    padding = { 3, 3, 3, 3 },
    hover = {
        opacity = 0.95,
    },
})

wyrd.style("wallpaper_thumb_slot", {
    extends = "chip",
    border_radius = 10,
    opacity = 0.80,
    padding = { 2, 2, 2, 2 },
    hover = {
        opacity = 1.0,
    },
})

wyrd.style("wallpaper_apply_btn", {
    extends = "media_control_btn_primary",
    border_radius = 10,
    font_size = 13,
    font_weight = "bold",
    padding = { 8, 16, 8, 16 },
})

wyrd.create({
    type = "popup",
    name = "wallpaper_picker",
    layer = "overlay",
    width = 920,
    height = 540,
    style = "wallpaper_window",
    widgets = {
        {
            type = "container",
            layout = {
                mode = "flex_col",
                gap = 14,
                width = "100%",
                height = "100%",
                justify = "space_between",
                padding = { top = 16, right = 20, bottom = 16, left = 20 },
            },
            children = {
                {
                    type = "container",
                    layout = { mode = "flex_row", align = "center", justify = "space_between" },
                    children = {
                        {
                            type = "container",
                            layout = { mode = "flex_row", align = "center", gap = 12 },
                            children = {
                                {
                                    type = "text",
                                    text = "󰸉  Wallpaper Selector",
                                    style = "header_title",
                                },
                                {
                                    type = "text",
                                    text = "{wallpaper.name}  •  {wallpaper.resolution}",
                                    style = "wallpaper_subtitle",
                                },
                            },
                        },
                        {
                            type = "container",
                            layout = { mode = "flex_row", align = "center", gap = 8 },
                            children = {
                                {
                                    type = "text",
                                    text = "{wallpaper.index} / {wallpaper.total}",
                                    style = "chip",
                                },
                                {
                                    type = "button",
                                    id = "wp_close_btn",
                                    text = "✕",
                                    action = "wallpaper:close",
                                    style = "media_control_btn",
                                    layout = { width = 30, height = 30, justify = "center", align = "center" },
                                },
                            },
                        },
                    },
                },
                {
                    type = "container",
                    layout = { mode = "flex_row", align = "center", justify = "center", gap = 14 },
                    children = {
                        {
                            type = "button",
                            id = "wp_prev_btn",
                            text = "󰅁",
                            action = "wallpaper:prev",
                            style = "media_control_btn",
                            layout = { width = 38, height = 38, justify = "center", align = "center" },
                        },
                        {
                            type = "button",
                            id = "wp_prev_card",
                            path = "{wallpaper.prev_thumb}",
                            action = "wallpaper:prev",
                            style = "wallpaper_card_side",
                            layout = { width = 185, height = 116, justify = "center", align = "center" },
                        },
                        {
                            type = "button",
                            id = "wp_active_card",
                            path = "{wallpaper.current_thumb}",
                            action = "wallpaper:apply",
                            style = "wallpaper_card_active",
                            layout = { width = 384, height = 240, justify = "center", align = "center" },
                        },
                        {
                            type = "button",
                            id = "wp_next_card",
                            path = "{wallpaper.next_thumb}",
                            action = "wallpaper:next",
                            style = "wallpaper_card_side",
                            layout = { width = 185, height = 116, justify = "center", align = "center" },
                        },
                        {
                            type = "button",
                            id = "wp_next_btn",
                            text = "󰅂",
                            action = "wallpaper:next",
                            style = "media_control_btn",
                            layout = { width = 38, height = 38, justify = "center", align = "center" },
                        },
                    },
                },
                {
                    type = "container",
                    layout = { mode = "flex_row", align = "center", justify = "center", gap = 10 },
                    children = {
                        {
                            type = "button",
                            id = "wp_slot_0",
                            path = "{wallpaper.slot_0_thumb}",
                            action = "wallpaper:select_slot_0",
                            style = "wallpaper_thumb_slot",
                            layout = { width = 152, height = 86, justify = "center", align = "center" },
                        },
                        {
                            type = "button",
                            id = "wp_slot_1",
                            path = "{wallpaper.slot_1_thumb}",
                            action = "wallpaper:select_slot_1",
                            style = "wallpaper_thumb_slot",
                            layout = { width = 152, height = 86, justify = "center", align = "center" },
                        },
                        {
                            type = "button",
                            id = "wp_slot_2",
                            path = "{wallpaper.slot_2_thumb}",
                            action = "wallpaper:select_slot_2",
                            style = "wallpaper_card_active",
                            layout = { width = 156, height = 88, justify = "center", align = "center" },
                        },
                        {
                            type = "button",
                            id = "wp_slot_3",
                            path = "{wallpaper.slot_3_thumb}",
                            action = "wallpaper:select_slot_3",
                            style = "wallpaper_thumb_slot",
                            layout = { width = 152, height = 86, justify = "center", align = "center" },
                        },
                        {
                            type = "button",
                            id = "wp_slot_4",
                            path = "{wallpaper.slot_4_thumb}",
                            action = "wallpaper:select_slot_4",
                            style = "wallpaper_thumb_slot",
                            layout = { width = 152, height = 86, justify = "center", align = "center" },
                        },
                    },
                },
                {
                    type = "container",
                    layout = { mode = "flex_row", align = "center", justify = "space_between" },
                    children = {
                        {
                            type = "text",
                            text = "←/→ or Scroll to browse   •   Click or Enter to apply   •   Esc to close",
                            style = "wallpaper_hint",
                        },
                        {
                            type = "container",
                            layout = { mode = "flex_row", align = "center", gap = 10 },
                            children = {
                                {
                                    type = "button",
                                    id = "wp_random_btn",
                                    text = "🎲  Random",
                                    action = "wallpaper:random",
                                    style = "quick_toggle",
                                    layout = { width = 115, height = 36, justify = "center", align = "center" },
                                },
                                {
                                    type = "button",
                                    id = "wp_apply_btn",
                                    text = "󰄬  Apply Wallpaper",
                                    action = "wallpaper:apply",
                                    style = "wallpaper_apply_btn",
                                    layout = { width = 175, height = 36, justify = "center", align = "center" },
                                },
                            },
                        },
                    },
                },
            },
        },
    },
})
