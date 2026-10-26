//! True-black native IDE tokens. Calculator materials live with its geometry.

use egui::{Color32, Stroke, Visuals};

pub const BG_PANEL: Color32 = Color32::from_rgb(0x0D, 0x0D, 0x0D);
pub const BG_WIDGET: Color32 = Color32::from_rgb(0x11, 0x11, 0x11);
/// Pure OLED pitch-black contrast surface (`#000000`).
pub const BG_EXTREME: Color32 = Color32::from_rgb(0x00, 0x00, 0x00);
pub const BORDER_CRISP: Color32 = Color32::from_rgb(0x25, 0x25, 0x25);
pub const TEXT_NORMAL: Color32 = Color32::from_rgb(0xED, 0xED, 0xED);
pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x92, 0x92, 0x92);
pub const ACCENT_BLUE: Color32 = Color32::from_rgb(0x64, 0xA9, 0xFF);
pub const ACCENT_GREEN: Color32 = Color32::from_rgb(0x48, 0xC7, 0x8E);
pub const ACCENT_RED: Color32 = Color32::from_rgb(0xF0, 0x71, 0x78);
pub const ACCENT_AMBER: Color32 = Color32::from_rgb(0xF5, 0x9E, 0x0B);

// Calculator Key Colors
pub const CALC_KEY_FN_BG: Color32 = Color32::from_gray(39);
pub const CALC_KEY_FN_FG: Color32 = Color32::from_gray(245);
pub const CALC_KEY_FN_SEC: Color32 = Color32::from_rgb(62, 185, 226); // Sky/cyan blue
pub const CALC_KEY_NUM_BG: Color32 = Color32::from_gray(226);
pub const CALC_KEY_NUM_FG: Color32 = Color32::from_gray(29);
pub const CALC_KEY_NUM_SEC: Color32 = Color32::from_rgb(0, 66, 153); // Cobalt blue
pub const CALC_KEY_ALPHA_FG: Color32 = Color32::from_rgb(222, 82, 7); // Vibrant orange
pub const CALC_KEY_SHIFT: Color32 = Color32::from_rgb(0, 61, 156);
pub const CALC_KEY_ALPHA: Color32 = Color32::from_rgb(244, 103, 6);
pub const CALC_KEY_ON: Color32 = Color32::from_gray(226);
pub const CALC_KEY_ENTER: Color32 = Color32::from_gray(226);

// Calculator D-Pad Navigation Colors
pub const CALC_DPAD_RIM: Color32 = Color32::from_gray(80);
pub const CALC_DPAD_BODY: Color32 = Color32::from_gray(29);
pub const CALC_DPAD_PIVOT: Color32 = Color32::from_gray(45);
pub const CALC_DPAD_PIVOT_STROKE: Color32 = Color32::from_gray(65);
pub const CALC_DPAD_MARKER: Color32 = Color32::from_gray(95);
pub const CALC_DPAD_MARKER_ACTIVE: Color32 = Color32::from_gray(230);

/// Code editor surfaces.
pub mod editor {
    use egui::Color32;

    /// Text area background.
    pub const BG: Color32 = Color32::from_rgb(0x0A, 0x0A, 0x0A);
    /// Gutter background (slightly lifted from the text area).
    pub const GUTTER_BG: Color32 = Color32::from_rgb(0x0A, 0x0A, 0x0A);
    pub const GUTTER_TEXT: Color32 = Color32::from_rgb(0x78, 0x78, 0x78);
    pub const GUTTER_TEXT_ACTIVE: Color32 = Color32::from_rgb(0xC6, 0xCC, 0xD2);
    /// Band behind the line holding the cursor.
    pub const CURRENT_LINE: Color32 = Color32::from_rgb(0x15, 0x15, 0x15);
    /// Band behind the line the debugger is paused on.
    pub const PAUSED_LINE: Color32 = Color32::from_rgba_premultiplied(0x3A, 0x2C, 0x0A, 0xB0);
    pub const SELECTION: Color32 = Color32::from_rgb(0x20, 0x34, 0x48);
    pub const BRACKET_MATCH: Color32 = Color32::from_rgb(0x2A, 0x2A, 0x2A);
    pub const FIND_MATCH: Color32 = Color32::from_rgba_premultiplied(0x5A, 0x48, 0x10, 0x90);
    pub const BREAKPOINT: Color32 = Color32::from_rgb(0xE5, 0x48, 0x4D);
    pub const CURSOR: Color32 = Color32::from_rgb(0xAE, 0xCB, 0xFA);
    pub const FOCUS_BORDER: Color32 = Color32::from_rgb(0x64, 0xA9, 0xFF);
}

/// Syntax colors for the dark editor surface. Hues are kept apart so every
/// token class is distinguishable, saturation stays moderate, and every
/// color meets WCAG contrast against [`editor::BG`] (4.5:1 for code, 3:1
/// for comments, which are meant to recede); see the tests below.
pub mod syntax {
    use egui::Color32;

    pub const TEXT: Color32 = Color32::from_rgb(0xD4, 0xD8, 0xDD);
    pub const KEYWORD: Color32 = Color32::from_rgb(0x6C, 0xA8, 0xE6);
    pub const OPERATOR_KEYWORD: Color32 = Color32::from_rgb(0x8F, 0xB8, 0xE8);
    pub const COMMAND: Color32 = Color32::from_rgb(0x4E, 0xC9, 0xB0);
    pub const CAS_COMMAND: Color32 = Color32::from_rgb(0xC5, 0x8F, 0xC9);
    pub const USER_FUNCTION: Color32 = Color32::from_rgb(0xDC, 0xD0, 0x9A);
    pub const PARAMETER: Color32 = Color32::from_rgb(0x9C, 0xDC, 0xFE);
    pub const SYSTEM_VARIABLE: Color32 = Color32::from_rgb(0xE0, 0xA8, 0x7C);
    pub const CONSTANT: Color32 = Color32::from_rgb(0xE0, 0xA8, 0x7C);
    pub const NUMBER: Color32 = Color32::from_rgb(0xB5, 0xCE, 0xA8);
    pub const STRING: Color32 = Color32::from_rgb(0xCE, 0x91, 0x78);
    pub const STRING_ESCAPE: Color32 = Color32::from_rgb(0xE8, 0xC9, 0x7A);
    pub const SYMBOLIC: Color32 = Color32::from_rgb(0xD6, 0xA5, 0x8A);
    pub const INVALID: Color32 = Color32::from_rgb(0xF4, 0x87, 0x71);
    pub const INVALID_BG: Color32 = Color32::from_rgba_premultiplied(0x3C, 0x16, 0x12, 0x80);
    pub const COMMENT: Color32 = Color32::from_rgb(0x74, 0x9C, 0x62);
    pub const PRAGMA: Color32 = Color32::from_rgb(0xB0, 0xA0, 0xD0);
    pub const CAS_BLOCK: Color32 = Color32::from_rgb(0xB3, 0x92, 0xB7);
    pub const OPERATOR: Color32 = Color32::from_rgb(0xBC, 0xC3, 0xCB);
    pub const PUNCTUATION: Color32 = Color32::from_rgb(0x9A, 0xA3, 0xAC);
    pub const BRACKET: Color32 = Color32::from_rgb(0xC9, 0xCF, 0xD6);
}

/// WCAG 2.x contrast ratio between two opaque colors.
pub fn contrast_ratio(a: Color32, b: Color32) -> f32 {
    fn channel(c: u8) -> f32 {
        let c = c as f32 / 255.0;
        if c <= 0.039_28 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }
    fn luminance(c: Color32) -> f32 {
        0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
    }
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

pub const ADWAITA_SANS_CANDIDATES: &[&str] = &[
    "/usr/share/fonts/adwaita-sans-fonts/AdwaitaSans-Regular.ttf", // Fedora / RHEL
    "/usr/share/fonts/truetype/adwaita/AdwaitaSans-Regular.ttf",   // Debian / Ubuntu
    "/usr/share/fonts/adwaita/AdwaitaSans-Regular.ttf",            // Arch Linux
    "/usr/share/fonts/TTF/AdwaitaSans-Regular.ttf",                // Arch TTF
    "/usr/share/fonts/truetype/AdwaitaSans-Regular.ttf",           // openSUSE / generic
    "/usr/share/fonts/opentype/adwaita/AdwaitaSans-Regular.otf",   // Debian OpenType
    "/app/share/fonts/AdwaitaSans-Regular.ttf",                    // Flatpak app
    "/app/share/fonts/adwaita/AdwaitaSans-Regular.ttf",            // Flatpak app subdir
    "/run/host/fonts/adwaita-sans-fonts/AdwaitaSans-Regular.ttf",  // Flatpak host (Fedora)
    "/run/host/fonts/truetype/adwaita/AdwaitaSans-Regular.ttf",    // Flatpak host (Debian)
    "/run/host/fonts/adwaita/AdwaitaSans-Regular.ttf",             // Flatpak host (Arch)
    "/var/lib/flatpak/exports/share/fonts/AdwaitaSans-Regular.ttf",
];

pub const ADWAITA_MONO_CANDIDATES: &[&str] = &[
    "/usr/share/fonts/adwaita-mono-fonts/AdwaitaMono-Regular.ttf", // Fedora / RHEL
    "/usr/share/fonts/truetype/adwaita/AdwaitaMono-Regular.ttf",   // Debian / Ubuntu
    "/usr/share/fonts/adwaita/AdwaitaMono-Regular.ttf",            // Arch Linux
    "/usr/share/fonts/TTF/AdwaitaMono-Regular.ttf",                // Arch TTF
    "/usr/share/fonts/truetype/AdwaitaMono-Regular.ttf",           // openSUSE / generic
    "/usr/share/fonts/opentype/adwaita/AdwaitaMono-Regular.otf",   // Debian OpenType
    "/app/share/fonts/AdwaitaMono-Regular.ttf",                    // Flatpak app
    "/app/share/fonts/adwaita/AdwaitaMono-Regular.ttf",            // Flatpak app subdir
    "/run/host/fonts/adwaita-mono-fonts/AdwaitaMono-Regular.ttf",  // Flatpak host (Fedora)
    "/run/host/fonts/truetype/adwaita/AdwaitaMono-Regular.ttf",    // Flatpak host (Debian)
    "/run/host/fonts/adwaita/AdwaitaMono-Regular.ttf",             // Flatpak host (Arch)
    "/var/lib/flatpak/exports/share/fonts/AdwaitaMono-Regular.ttf",
];

/// Discovers font bytes across XDG user directories and candidate Linux distribution paths.
pub fn discover_font_bytes(distro_candidates: &[&str], filename: &str) -> Option<Vec<u8>> {
    if let Ok(data_home) = std::env::var("XDG_DATA_HOME") {
        let base = std::path::PathBuf::from(data_home).join("fonts");
        if let Ok(bytes) = std::fs::read(base.join(filename)) {
            return Some(bytes);
        }
        if let Ok(bytes) = std::fs::read(base.join("adwaita").join(filename)) {
            return Some(bytes);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let home_p = std::path::Path::new(&home);
        for sub in [
            ".local/share/fonts",
            ".local/share/fonts/adwaita",
            ".fonts",
            ".fonts/adwaita",
        ] {
            if let Ok(bytes) = std::fs::read(home_p.join(sub).join(filename)) {
                return Some(bytes);
            }
        }
    }
    for &candidate in distro_candidates {
        if let Ok(bytes) = std::fs::read(candidate) {
            return Some(bytes);
        }
    }
    None
}

/// Font setup: egui's bundled fonts (Hack for code) first, then DejaVu
/// Sans as a fallback for every family, so symbols such as ✓ ▲ ▼ ☐ in the
/// UI and ⅈ ℯ ᴇ in PPL source render instead of showing as empty boxes.
pub fn font_definitions() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "dejavu-sans".to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(
            ppl_graphics::text::dejavu_sans_ttf(),
        )),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("dejavu-sans".to_owned());
    }
    // Read once at context creation; no system process or font scan per frame.
    if let Some(bytes) = discover_font_bytes(ADWAITA_SANS_CANDIDATES, "AdwaitaSans-Regular.ttf") {
        fonts.font_data.insert(
            "adwaita-sans".into(),
            std::sync::Arc::new(egui::FontData::from_owned(bytes)),
        );
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "adwaita-sans".into());
    }

    if let Some(bytes) = discover_font_bytes(ADWAITA_MONO_CANDIDATES, "AdwaitaMono-Regular.ttf") {
        fonts.font_data.insert(
            "adwaita-mono".into(),
            std::sync::Arc::new(egui::FontData::from_owned(bytes)),
        );
        fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default()
            .insert(0, "adwaita-mono".into());
    }
    fonts
}

/// Configures the egui context with the true-black visual style.
pub fn apply_theme(ctx: &egui::Context) {
    ctx.set_fonts(font_definitions());
    ctx.set_visuals(classic_engineering_visuals());
    ctx.style_mut(|style| {
        use egui::{FontFamily, FontId, TextStyle};
        // One consistent type scale for panels, tabs and status text.
        style.text_styles = [
            (
                TextStyle::Small,
                FontId::new(11.0, FontFamily::Proportional),
            ),
            (TextStyle::Body, FontId::new(13.0, FontFamily::Proportional)),
            (
                TextStyle::Button,
                FontId::new(13.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Heading,
                FontId::new(16.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(13.0, FontFamily::Monospace),
            ),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(6.0, 4.0);
        style.spacing.button_padding = egui::vec2(6.0, 3.0);
        // Solid, always-visible scroll bars wide enough to grab.
        style.spacing.scroll = egui::style::ScrollStyle {
            bar_width: 10.0,
            floating: false,
            ..egui::style::ScrollStyle::solid()
        };
    });
}

/// Returns the true-black `Visuals` configuration.
pub fn classic_engineering_visuals() -> Visuals {
    let mut visuals = Visuals::dark();

    visuals.panel_fill = BG_PANEL;
    visuals.window_fill = BG_WIDGET;
    visuals.extreme_bg_color = BG_EXTREME;
    visuals.faint_bg_color = BG_PANEL;

    let crisp_stroke = Stroke::new(1.0_f32, BORDER_CRISP);
    visuals.window_stroke = crisp_stroke;
    visuals.window_corner_radius = egui::CornerRadius::ZERO;
    visuals.menu_corner_radius = egui::CornerRadius::ZERO;

    // Noninteractive widgets (panels, headers, labels)
    visuals.widgets.noninteractive.bg_fill = BG_WIDGET;
    visuals.widgets.noninteractive.bg_stroke = crisp_stroke;
    visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::ZERO;
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT_NORMAL);

    // Inactive interactive widgets (buttons, tabs, inputs at rest)
    visuals.widgets.inactive.bg_fill = BG_WIDGET;
    visuals.widgets.inactive.bg_stroke = crisp_stroke;
    visuals.widgets.inactive.corner_radius = egui::CornerRadius::ZERO;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT_NORMAL);

    // Hovered widgets
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(0x18, 0x18, 0x18);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, ACCENT_BLUE);
    visuals.widgets.hovered.corner_radius = egui::CornerRadius::ZERO;
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);

    // Active (clicked/pressed) widgets
    visuals.widgets.active.bg_fill = Color32::from_rgb(0x20, 0x20, 0x20);
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT_BLUE);
    visuals.widgets.active.corner_radius = egui::CornerRadius::ZERO;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);

    // Open/expanded widgets (comboboxes, menus)
    visuals.widgets.open.bg_fill = BG_WIDGET;
    visuals.widgets.open.bg_stroke = crisp_stroke;
    visuals.widgets.open.corner_radius = egui::CornerRadius::ZERO;
    visuals.widgets.open.fg_stroke = Stroke::new(1.0_f32, TEXT_NORMAL);

    visuals.selection.bg_fill = editor::SELECTION;
    visuals.selection.stroke = Stroke::new(1.0_f32, TEXT_NORMAL);
    visuals.text_cursor.stroke = Stroke::new(1.5_f32, editor::CURSOR);
    visuals.window_shadow = egui::epaint::Shadow::NONE;
    visuals.override_text_color = Some(TEXT_NORMAL);
    visuals.text_cursor.blink = true;

    visuals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syntax_palette_is_readable_on_the_editor_background() {
        use syntax::*;
        for (name, c) in [
            ("TEXT", TEXT),
            ("KEYWORD", KEYWORD),
            ("OPERATOR_KEYWORD", OPERATOR_KEYWORD),
            ("COMMAND", COMMAND),
            ("CAS_COMMAND", CAS_COMMAND),
            ("USER_FUNCTION", USER_FUNCTION),
            ("PARAMETER", PARAMETER),
            ("SYSTEM_VARIABLE", SYSTEM_VARIABLE),
            ("CONSTANT", CONSTANT),
            ("NUMBER", NUMBER),
            ("STRING", STRING),
            ("STRING_ESCAPE", STRING_ESCAPE),
            ("SYMBOLIC", SYMBOLIC),
            ("INVALID", INVALID),
            ("PRAGMA", PRAGMA),
            ("CAS_BLOCK", CAS_BLOCK),
            ("OPERATOR", OPERATOR),
            ("PUNCTUATION", PUNCTUATION),
            ("BRACKET", BRACKET),
        ] {
            let r = contrast_ratio(c, editor::BG);
            assert!(r >= 4.5, "{name} contrast {r:.2} < 4.5");
            let r = contrast_ratio(c, editor::CURRENT_LINE);
            assert!(r >= 4.5, "{name} on current line {r:.2} < 4.5");
        }
        let r = contrast_ratio(COMMENT, editor::BG);
        assert!(r >= 3.0, "COMMENT contrast {r:.2} < 3");
        assert!(contrast_ratio(editor::GUTTER_TEXT, editor::GUTTER_BG) >= 3.0);
    }

    #[test]
    fn fallback_font_covers_ui_and_ppl_symbols() {
        let fonts = font_definitions();
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let chain = &fonts.families[&family];
            assert_eq!(chain.last().map(String::as_str), Some("dejavu-sans"));
            // The primary fonts are unchanged (Hack stays the code font).
            assert_ne!(chain.first().map(String::as_str), Some("dejavu-sans"));
        }
        let ctx = egui::Context::default();
        ctx.set_fonts(font_definitions());
        let _ = ctx.run(Default::default(), |_| {});
        ctx.fonts(|f| {
            for ch in ['✓', '☐', '▲', '▼', 'ⅈ', 'ℯ', 'ᴇ', '▶', '≠', '→'] {
                assert!(
                    f.has_glyphs(&egui::FontId::monospace(14.0), &ch.to_string()),
                    "no glyph for {ch}"
                );
                assert!(
                    f.has_glyphs(&egui::FontId::proportional(13.0), &ch.to_string()),
                    "no UI glyph for {ch}"
                );
            }
        });
    }

    #[test]
    fn distinct_token_classes_have_distinct_colors() {
        use syntax::*;
        let classes = [
            TEXT,
            KEYWORD,
            COMMAND,
            CAS_COMMAND,
            USER_FUNCTION,
            PARAMETER,
            CONSTANT,
            NUMBER,
            STRING,
            STRING_ESCAPE,
            INVALID,
            COMMENT,
            OPERATOR,
            PUNCTUATION,
        ];
        for (i, a) in classes.iter().enumerate() {
            for b in &classes[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn test_theme_colors_and_zero_corners() {
        let visuals = classic_engineering_visuals();
        assert_eq!(visuals.panel_fill, BG_PANEL);
        assert_eq!(visuals.window_fill, BG_WIDGET);
        assert_eq!(visuals.extreme_bg_color, BG_EXTREME);
        assert_eq!(visuals.window_corner_radius, egui::CornerRadius::ZERO);
        assert_eq!(visuals.menu_corner_radius, egui::CornerRadius::ZERO);
        assert_eq!(
            visuals.widgets.inactive.corner_radius,
            egui::CornerRadius::ZERO
        );
        assert_eq!(
            visuals.widgets.active.corner_radius,
            egui::CornerRadius::ZERO
        );
        assert_eq!(
            visuals.widgets.hovered.corner_radius,
            egui::CornerRadius::ZERO
        );
    }

    #[test]
    fn test_oled_contrast_and_wcag_compliance() {
        assert_eq!(BG_EXTREME, Color32::from_rgb(0x00, 0x00, 0x00));
        assert_eq!(editor::SELECTION, Color32::from_rgb(0x20, 0x34, 0x48));

        // Normal text against extreme, panel, and selection backgrounds
        assert!(
            contrast_ratio(TEXT_NORMAL, BG_EXTREME) >= 7.0,
            "TEXT_NORMAL on BG_EXTREME must exceed WCAG AAA (7.0:1)"
        );
        assert!(
            contrast_ratio(TEXT_NORMAL, BG_PANEL) >= 4.5,
            "TEXT_NORMAL on BG_PANEL must exceed WCAG AA (4.5:1)"
        );
        assert!(
            contrast_ratio(TEXT_NORMAL, editor::SELECTION) >= 4.5,
            "TEXT_NORMAL on SELECTION must exceed WCAG AA (4.5:1)"
        );
        assert!(
            contrast_ratio(TEXT_MUTED, BG_EXTREME) >= 4.5,
            "TEXT_MUTED on BG_EXTREME must meet WCAG AA (4.5:1)"
        );

        // Every syntax token on BG_EXTREME and editor::SELECTION
        use syntax::*;
        for (name, c) in [
            ("TEXT", TEXT),
            ("KEYWORD", KEYWORD),
            ("OPERATOR_KEYWORD", OPERATOR_KEYWORD),
            ("COMMAND", COMMAND),
            ("CAS_COMMAND", CAS_COMMAND),
            ("USER_FUNCTION", USER_FUNCTION),
            ("PARAMETER", PARAMETER),
            ("SYSTEM_VARIABLE", SYSTEM_VARIABLE),
            ("CONSTANT", CONSTANT),
            ("NUMBER", NUMBER),
            ("STRING", STRING),
            ("STRING_ESCAPE", STRING_ESCAPE),
            ("SYMBOLIC", SYMBOLIC),
            ("INVALID", INVALID),
            ("PRAGMA", PRAGMA),
            ("CAS_BLOCK", CAS_BLOCK),
            ("OPERATOR", OPERATOR),
            ("PUNCTUATION", PUNCTUATION),
            ("BRACKET", BRACKET),
        ] {
            let r_bg = contrast_ratio(c, BG_EXTREME);
            assert!(
                r_bg >= 4.5,
                "{name} contrast {r_bg:.2} on BG_EXTREME must be >= 4.5"
            );
            let r_sel = contrast_ratio(c, editor::SELECTION);
            assert!(
                r_sel >= 4.5,
                "{name} contrast {r_sel:.2} on SELECTION must be >= 4.5"
            );
        }

        // Incidental comments
        assert!(contrast_ratio(COMMENT, BG_EXTREME) >= 3.0);
        assert!(contrast_ratio(COMMENT, editor::SELECTION) >= 3.0);
    }

    #[test]
    fn test_dpi_aware_logical_point_scaling() {
        for dpi in [1.0_f32, 1.25, 1.5, 2.0] {
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(dpi);
            apply_theme(&ctx);

            let style = ctx.style();
            use egui::{FontFamily, TextStyle};
            assert_eq!(
                style.text_styles[&TextStyle::Small].size,
                11.0,
                "Small text style must remain 11.0 logical points at {dpi}x"
            );
            assert_eq!(
                style.text_styles[&TextStyle::Body].size,
                13.0,
                "Body text style must remain 13.0 logical points at {dpi}x"
            );
            assert_eq!(
                style.text_styles[&TextStyle::Heading].size,
                16.0,
                "Heading text style must remain 16.0 logical points at {dpi}x"
            );
            assert_eq!(
                style.text_styles[&TextStyle::Monospace].size,
                13.0,
                "Monospace text style must remain 13.0 logical points at {dpi}x"
            );
            assert_eq!(
                style.text_styles[&TextStyle::Monospace].family,
                FontFamily::Monospace
            );
            assert_eq!(style.spacing.item_spacing, egui::vec2(6.0, 4.0));
            assert_eq!(style.spacing.scroll.bar_width, 10.0);
        }
    }

    #[test]
    fn test_font_discovery_fallback_resilience() {
        let missing = discover_font_bytes(&["/nonexistent/path/font.ttf"], "NonexistentFont.ttf");
        assert!(missing.is_none());

        let fonts = font_definitions();
        assert!(fonts.font_data.contains_key("dejavu-sans"));
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let chain = &fonts.families[&family];
            assert_eq!(
                chain.last().map(String::as_str),
                Some("dejavu-sans"),
                "dejavu-sans must be the terminal fallback in {family:?}"
            );
        }
    }
}
