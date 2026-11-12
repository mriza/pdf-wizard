use eframe::egui::{self, Color32, CornerRadius, Stroke, Vec2, Visuals};

pub const TOUCH_HEIGHT: f32 = 44.0;

#[derive(Clone, Copy)]
pub struct AntTokens {
    pub primary: Color32,
    pub primary_hover: Color32,
    pub primary_active: Color32,
    pub primary_bg: Color32,
    pub danger: Color32,
    pub danger_bg: Color32,
    pub success: Color32,
    pub bg_layout: Color32,
    pub bg_container: Color32,
    pub bg_elevated: Color32,
    pub border: Color32,
    pub border_secondary: Color32,
    pub text: Color32,
    pub text_secondary: Color32,
}

impl AntTokens {
    pub const fn light() -> Self {
        Self {
            primary: Color32::from_rgb(22, 119, 255),        // #1677FF
            primary_hover: Color32::from_rgb(64, 150, 255),  // #4096FF
            primary_active: Color32::from_rgb(9, 88, 217),   // #0958D9
            primary_bg: Color32::from_rgb(230, 244, 255),    // #E6F4FF
            danger: Color32::from_rgb(255, 77, 79),          // #FF4D4F
            danger_bg: Color32::from_rgb(255, 242, 240),     // #FFF2F0
            success: Color32::from_rgb(82, 196, 26),         // #52C41A
            bg_layout: Color32::from_rgb(245, 245, 245),     // #F5F5F5
            bg_container: Color32::from_rgb(255, 255, 255),  // #FFFFFF
            bg_elevated: Color32::from_rgb(250, 250, 250),   // #FAFAFA
            border: Color32::from_rgb(217, 217, 217),        // #D9D9D9
            border_secondary: Color32::from_rgb(240, 240, 240), // #F0F0F0
            text: Color32::from_rgb(31, 31, 31),
            text_secondary: Color32::from_rgb(115, 115, 115),
        }
    }

    pub const fn dark() -> Self {
        Self {
            primary: Color32::from_rgb(22, 104, 220),        // #1668DC
            primary_hover: Color32::from_rgb(60, 137, 232),  // #3C89E8
            primary_active: Color32::from_rgb(21, 84, 173),  // #1554AD
            primary_bg: Color32::from_rgb(17, 29, 44),       // #111D2C
            danger: Color32::from_rgb(220, 68, 70),          // #DC4446
            danger_bg: Color32::from_rgb(42, 18, 21),        // #2A1215
            success: Color32::from_rgb(73, 170, 25),         // #49AA19
            bg_layout: Color32::from_rgb(20, 20, 20),        // #141414
            bg_container: Color32::from_rgb(31, 31, 31),     // #1F1F1F
            bg_elevated: Color32::from_rgb(38, 38, 38),      // #262626
            border: Color32::from_rgb(66, 66, 66),           // #424242
            border_secondary: Color32::from_rgb(48, 48, 48), // #303030
            text: Color32::from_rgb(235, 235, 235),
            text_secondary: Color32::from_rgb(150, 150, 150),
        }
    }

    pub const fn for_mode(dark: bool) -> Self {
        if dark { Self::dark() } else { Self::light() }
    }
}

#[derive(Clone, Copy)]
pub enum BtnKind {
    Default,
    Primary,
    Active,
    Danger,
}

/// Applies Ant Design 5 color tokens, rounded corners, and touch-friendly spacing.
pub fn apply_ant_theme(ctx: &egui::Context, dark: bool) {
    let t = AntTokens::for_mode(dark);
    let mut v = if dark { Visuals::dark() } else { Visuals::light() };
    let r6 = CornerRadius::same(6);
    let r8 = CornerRadius::same(8);

    v.panel_fill = t.bg_layout;
    v.window_fill = t.bg_container;
    v.extreme_bg_color = t.bg_elevated;
    v.faint_bg_color = t.bg_container;
    v.window_corner_radius = r8;
    v.window_stroke = Stroke::new(1.0, t.border);
    v.menu_corner_radius = r8;
    v.selection.bg_fill = t.primary_bg;
    v.selection.stroke = Stroke::new(1.5, t.primary);

    v.widgets.noninteractive.bg_fill = t.bg_container;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, t.border_secondary);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.noninteractive.corner_radius = r6;

    v.widgets.inactive.bg_fill = t.bg_container;
    v.widgets.inactive.weak_bg_fill = t.bg_container;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, t.border);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.inactive.corner_radius = r6;

    v.widgets.hovered.bg_fill = t.bg_elevated;
    v.widgets.hovered.weak_bg_fill = t.bg_elevated;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, t.primary_hover);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, t.primary_hover);
    v.widgets.hovered.corner_radius = r6;

    v.widgets.active.bg_fill = t.primary_bg;
    v.widgets.active.weak_bg_fill = t.primary_bg;
    v.widgets.active.bg_stroke = Stroke::new(1.0, t.primary_active);
    v.widgets.active.fg_stroke = Stroke::new(1.0, t.primary_active);
    v.widgets.active.corner_radius = r6;

    ctx.set_visuals(v);
    ctx.all_styles_mut(|s| {
        s.spacing.interact_size = Vec2::new(TOUCH_HEIGHT, TOUCH_HEIGHT);
        s.spacing.button_padding = Vec2::new(14.0, 10.0);
        s.spacing.item_spacing = Vec2::new(8.0, 8.0);
        s.spacing.slider_width = 180.0;
        s.spacing.slider_rail_height = 14.0;
        s.spacing.scroll.bar_width = 14.0;
        s.spacing.scroll.handle_min_length = TOUCH_HEIGHT;
    });
}
