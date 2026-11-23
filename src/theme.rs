use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Shadow,
    Stroke, TextStyle, Vec2, Visuals,
};
use std::collections::BTreeMap;
use std::sync::Arc;

// Font families. Instrument Sans carries the interface, Newsreader the few moments of
// display type, and Lucide is a fallback for glyphs used as icons.
pub const SANS: &str = "sans";
pub const SANS_MEDIUM: &str = "sans-medium";
pub const SANS_SEMIBOLD: &str = "sans-semibold";
pub const SERIF: &str = "serif";
pub const ICONS: &str = "icons";

pub fn sans() -> FontFamily {
    FontFamily::Name(SANS.into())
}

pub fn sans_medium() -> FontFamily {
    FontFamily::Name(SANS_MEDIUM.into())
}

pub fn sans_semibold() -> FontFamily {
    FontFamily::Name(SANS_SEMIBOLD.into())
}

pub fn serif() -> FontFamily {
    FontFamily::Name(SERIF.into())
}

pub fn icons() -> FontFamily {
    FontFamily::Name(ICONS.into())
}

pub fn body(size: f32) -> FontId {
    FontId::new(size, sans())
}

pub fn medium(size: f32) -> FontId {
    FontId::new(size, sans_medium())
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, sans_semibold())
}

pub fn serif_font(size: f32) -> FontId {
    FontId::new(size, serif())
}

pub fn icon_font(size: f32) -> FontId {
    FontId::new(size, icons())
}

pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

pub fn label_font() -> FontId {
    semibold(10.5)
}

/// Radius scale: paper, control, card, modal. Keeping one scale avoids a pill-shaped UI.
pub const RADIUS_PAPER: u8 = 2;
pub const RADIUS_CONTROL: u8 = 8;
pub const RADIUS_CARD: u8 = 10;
pub const RADIUS_MODAL: u8 = 14;

#[derive(Clone, Copy)]
pub struct Tokens {
    /// Window and status-bar ground.
    pub ground: Color32,
    /// Sidebar and fixed chrome.
    pub panel: Color32,
    /// Header strip, one step lighter than the panel.
    pub header: Color32,
    /// The dotted bench behind the page grid.
    pub canvas: Color32,
    /// Cards, inputs, raised slips.
    pub surface: Color32,
    pub surface_hover: Color32,
    pub surface_active: Color32,
    pub ink: Color32,
    pub muted: Color32,
    pub faint: Color32,
    pub line: Color32,
    pub line_strong: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_active: Color32,
    pub accent_soft: Color32,
    pub accent_tint: Color32,
    pub on_accent: Color32,
    pub danger: Color32,
    pub danger_soft: Color32,
    pub success: Color32,
    pub brass: Color32,
    pub shadow: Color32,
    pub scrim: Color32,
    pub paper: Color32,
    pub paper_shadow: Color32,
    pub dark: bool,
}

impl Tokens {
    pub const fn light() -> Self {
        Self {
            ground: Color32::from_rgb(241, 237, 228),
            panel: Color32::from_rgb(250, 246, 238),
            header: Color32::from_rgb(253, 250, 244),
            canvas: Color32::from_rgb(240, 236, 227),
            surface: Color32::from_rgb(255, 253, 248),
            surface_hover: Color32::from_rgb(246, 241, 231),
            surface_active: Color32::from_rgb(239, 232, 219),
            ink: Color32::from_rgb(32, 30, 25),
            muted: Color32::from_rgb(106, 99, 87),
            faint: Color32::from_rgb(142, 133, 118),
            line: Color32::from_rgb(221, 213, 198),
            line_strong: Color32::from_rgb(199, 188, 169),
            accent: Color32::from_rgb(30, 92, 73),
            accent_hover: Color32::from_rgb(23, 74, 59),
            accent_active: Color32::from_rgb(16, 59, 46),
            accent_soft: Color32::from_rgb(225, 237, 231),
            accent_tint: Color32::from_rgb(241, 247, 243),
            on_accent: Color32::WHITE,
            danger: Color32::from_rgb(157, 59, 50),
            danger_soft: Color32::from_rgb(247, 232, 228),
            success: Color32::from_rgb(46, 107, 77),
            brass: Color32::from_rgb(150, 112, 58),
            shadow: Color32::from_rgba_premultiplied(46, 40, 28, 24),
            scrim: Color32::from_rgba_premultiplied(18, 15, 10, 128),
            paper: Color32::WHITE,
            paper_shadow: Color32::from_rgba_premultiplied(52, 45, 32, 38),
            dark: false,
        }
    }

    pub const fn dark() -> Self {
        Self {
            ground: Color32::from_rgb(18, 19, 16),
            panel: Color32::from_rgb(25, 26, 22),
            header: Color32::from_rgb(27, 28, 24),
            canvas: Color32::from_rgb(18, 19, 16),
            surface: Color32::from_rgb(32, 31, 27),
            surface_hover: Color32::from_rgb(41, 40, 34),
            surface_active: Color32::from_rgb(48, 47, 39),
            ink: Color32::from_rgb(242, 237, 226),
            muted: Color32::from_rgb(167, 159, 144),
            faint: Color32::from_rgb(131, 123, 109),
            line: Color32::from_rgb(53, 52, 44),
            line_strong: Color32::from_rgb(74, 72, 62),
            accent: Color32::from_rgb(130, 185, 163),
            accent_hover: Color32::from_rgb(151, 201, 180),
            accent_active: Color32::from_rgb(172, 216, 197),
            accent_soft: Color32::from_rgb(34, 50, 43),
            accent_tint: Color32::from_rgb(28, 41, 36),
            on_accent: Color32::from_rgb(16, 35, 27),
            danger: Color32::from_rgb(222, 138, 125),
            danger_soft: Color32::from_rgb(51, 35, 32),
            success: Color32::from_rgb(130, 185, 163),
            brass: Color32::from_rgb(194, 160, 106),
            shadow: Color32::from_rgba_premultiplied(0, 0, 0, 96),
            scrim: Color32::from_rgba_premultiplied(0, 0, 0, 176),
            paper: Color32::from_rgb(255, 255, 255),
            paper_shadow: Color32::from_rgba_premultiplied(0, 0, 0, 110),
            dark: true,
        }
    }

    pub const fn for_mode(dark: bool) -> Self {
        if dark { Self::dark() } else { Self::light() }
    }

    pub fn alpha(color: Color32, alpha: u8) -> Color32 {
        Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
    }

    /// Subtle paper shadow used under thumbnails and raised cards.
    pub fn paper_shadow(&self) -> Shadow {
        Shadow {
            offset: [0, 2],
            blur: if self.dark { 8 } else { 6 },
            spread: 0,
            color: self.paper_shadow,
        }
    }

    pub fn card_shadow(&self) -> Shadow {
        Shadow {
            offset: [0, 4],
            blur: 14,
            spread: 0,
            color: self.shadow,
        }
    }

    pub fn modal_shadow(&self) -> Shadow {
        Shadow {
            offset: [0, 12],
            blur: 36,
            spread: 0,
            color: self.shadow,
        }
    }
}

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let mut add = |name: &str, bytes: &'static [u8]| {
        fonts.font_data.insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    };
    add("instrument-regular", include_bytes!("../assets/fonts/InstrumentSans-Regular.ttf"));
    add("instrument-medium", include_bytes!("../assets/fonts/InstrumentSans-Medium.ttf"));
    add("instrument-semibold", include_bytes!("../assets/fonts/InstrumentSans-SemiBold.ttf"));
    add("newsreader", include_bytes!("../assets/fonts/Newsreader-Regular.ttf"));
    add("lucide", include_bytes!("../assets/icons/lucide.ttf"));

    let fallbacks = ["egui-icons"];
    let family = |primary: &[&str]| -> Vec<String> {
        primary.iter().chain(fallbacks.iter()).map(|s| (*s).to_owned()).collect()
    };

    fonts.families.insert(FontFamily::Proportional, family(&["instrument-regular", "lucide"]));
    fonts.families.insert(FontFamily::Monospace, family(&["Hack", "lucide"]));
    fonts.families.insert(FontFamily::Name(SANS.into()), family(&["instrument-regular", "lucide"]));
    fonts
        .families
        .insert(FontFamily::Name(SANS_MEDIUM.into()), family(&["instrument-medium", "lucide"]));
    fonts.families.insert(
        FontFamily::Name(SANS_SEMIBOLD.into()),
        family(&["instrument-semibold", "lucide"]),
    );
    fonts.families.insert(FontFamily::Name(SERIF.into()), family(&["newsreader", "lucide"]));
    fonts.families.insert(FontFamily::Name(ICONS.into()), family(&["lucide"]));

    ctx.set_fonts(fonts);
}

/// Installs the Quiet Bindery palette, type scale, spacing, and interaction states.
pub fn apply(ctx: &egui::Context, dark: bool) {
    let t = Tokens::for_mode(dark);
    let mut v = if dark { Visuals::dark() } else { Visuals::light() };

    v.dark_mode = dark;
    v.panel_fill = t.panel;
    v.window_fill = t.surface;
    v.extreme_bg_color = t.surface_hover;
    v.text_edit_bg_color = Some(t.surface);
    v.faint_bg_color = t.surface_hover;
    v.code_bg_color = t.surface_hover;
    v.window_corner_radius = CornerRadius::same(RADIUS_MODAL);
    v.window_stroke = Stroke::new(1.0, t.line);
    v.window_shadow = t.modal_shadow();
    v.window_highlight_topmost = false;
    v.menu_corner_radius = CornerRadius::same(RADIUS_CARD);
    v.popup_shadow = t.modal_shadow();
    v.selection.bg_fill = t.accent_soft;
    v.selection.stroke = Stroke::new(1.5, t.accent);
    v.hyperlink_color = t.accent;
    v.warn_fg_color = t.brass;
    v.error_fg_color = t.danger;
    v.weak_text_color = Some(t.muted);
    v.override_text_color = Some(t.ink);
    v.interact_cursor = Some(egui::CursorIcon::PointingHand);

    let r = CornerRadius::same(RADIUS_CONTROL);
    v.widgets.noninteractive.bg_fill = t.surface;
    v.widgets.noninteractive.weak_bg_fill = t.surface;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, t.line);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, t.ink);
    v.widgets.noninteractive.corner_radius = r;
    v.widgets.noninteractive.expansion = 0.0;

    v.widgets.inactive.bg_fill = t.surface;
    v.widgets.inactive.weak_bg_fill = t.surface;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, t.line);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, t.ink);
    v.widgets.inactive.corner_radius = r;
    v.widgets.inactive.expansion = 0.0;

    v.widgets.hovered.bg_fill = t.surface_hover;
    v.widgets.hovered.weak_bg_fill = t.surface_hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, t.line_strong);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, t.ink);
    v.widgets.hovered.corner_radius = r;
    v.widgets.hovered.expansion = 0.0;

    v.widgets.active.bg_fill = t.accent_soft;
    v.widgets.active.weak_bg_fill = t.accent_soft;
    v.widgets.active.bg_stroke = Stroke::new(1.0, t.accent);
    v.widgets.active.fg_stroke = Stroke::new(1.0, t.accent);
    v.widgets.active.corner_radius = r;
    v.widgets.active.expansion = 0.0;

    v.widgets.open.bg_fill = t.surface_hover;
    v.widgets.open.weak_bg_fill = t.surface_hover;
    v.widgets.open.bg_stroke = Stroke::new(1.0, t.line_strong);
    v.widgets.open.fg_stroke = Stroke::new(1.0, t.ink);
    v.widgets.open.corner_radius = r;

    let mut text_styles = BTreeMap::new();
    text_styles.insert(TextStyle::Body, body(14.0));
    text_styles.insert(TextStyle::Button, medium(13.0));
    text_styles.insert(TextStyle::Small, body(11.5));
    text_styles.insert(TextStyle::Heading, serif_font(22.0));
    text_styles.insert(TextStyle::Monospace, mono(11.5));
    text_styles.insert(TextStyle::Name("display".into()), serif_font(40.0));
    text_styles.insert(TextStyle::Name("display-sm".into()), serif_font(30.0));
    text_styles.insert(TextStyle::Name("title".into()), serif_font(21.0));
    text_styles.insert(TextStyle::Name("label".into()), label_font());
    text_styles.insert(TextStyle::Name("meta".into()), body(11.5));

    ctx.set_visuals(v);
    ctx.all_styles_mut(|s| {
        s.text_styles = text_styles.clone();
        s.animation_time = 0.16;
        s.spacing.interact_size = Vec2::new(24.0, 38.0);
        s.spacing.button_padding = Vec2::new(12.0, 8.0);
        s.spacing.item_spacing = Vec2::new(8.0, 8.0);
        s.spacing.icon_spacing = 7.0;
        s.spacing.icon_width = 16.0;
        s.spacing.icon_width_inner = 9.0;
        s.spacing.menu_margin = Margin::same(6);
        s.spacing.menu_width = 286.0;
        s.spacing.window_margin = Margin::same(18);
        s.spacing.indent = 16.0;
        s.spacing.slider_width = 168.0;
        s.spacing.slider_rail_height = 5.0;
        s.spacing.tooltip_width = 340.0;
        s.spacing.combo_width = 160.0;
        s.spacing.text_edit_width = 220.0;
        s.spacing.extra_text_line_spacing = 2.0;
        s.spacing.scroll.bar_width = 9.0;
        s.spacing.scroll.handle_min_length = 32.0;
        s.spacing.scroll.bar_inner_margin = 4.0;
        s.spacing.scroll.floating = false;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(c: u8) -> f32 {
        let c = c as f32 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    }

    fn luminance(c: Color32) -> f32 {
        0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f32 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    fn assert_aa(t: Tokens) {
        for (name, fg, bg) in [
            ("ink on canvas", t.ink, t.canvas),
            ("ink on surface", t.ink, t.surface),
            ("ink on panel", t.ink, t.panel),
            ("muted on canvas", t.muted, t.canvas),
            ("muted on surface", t.muted, t.surface),
            ("muted on panel", t.muted, t.panel),
            ("accent on accent-soft", t.accent, t.accent_soft),
            ("on-accent on accent", t.on_accent, t.accent),
            ("danger on surface", t.danger, t.surface),
            ("danger on danger-soft", t.danger, t.danger_soft),
            ("muted on header", t.muted, t.header),
        ] {
            let ratio = contrast(fg, bg);
            assert!(ratio >= 4.5, "{name} contrast {ratio:.2} is below AA");
        }
    }

    #[test]
    fn light_palette_meets_aa() {
        assert_aa(Tokens::light());
    }

    #[test]
    fn dark_palette_meets_aa() {
        assert_aa(Tokens::dark());
    }

    #[test]
    fn theme_supports_both_modes_and_touch_targets() {
        let ctx = egui::Context::default();
        apply(&ctx, false);
        assert!(!ctx.global_style().visuals.dark_mode);
        assert_eq!(ctx.global_style().visuals.panel_fill, Tokens::light().panel);
        assert!(ctx.global_style().spacing.interact_size.y >= 36.0);
        assert!(ctx.global_style().spacing.scroll.bar_width >= 8.0);

        apply(&ctx, true);
        assert!(ctx.global_style().visuals.dark_mode);
        assert_eq!(ctx.global_style().visuals.panel_fill, Tokens::dark().panel);
    }
}
