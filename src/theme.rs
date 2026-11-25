use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Stroke, Style, TextStyle, Visuals};

use crate::model::{OwnerCategory, PurposeTag, VisibilityState};

pub const BG_BASE: Color32 = Color32::from_rgb(13, 16, 23);
pub const BG_SURFACE: Color32 = Color32::from_rgb(20, 24, 34);
pub const BG_CARD: Color32 = Color32::from_rgb(25, 30, 43);
pub const BG_CARD_HOVER: Color32 = Color32::from_rgb(31, 38, 54);
pub const BG_CARD_SELECTED: Color32 = Color32::from_rgb(34, 45, 68);
pub const BG_CONSOLE: Color32 = Color32::from_rgb(10, 12, 18);

pub const BORDER_SUBTLE: Color32 = Color32::from_rgb(38, 46, 64);
pub const BORDER_ACCENT: Color32 = Color32::from_rgb(76, 132, 255);

pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(236, 240, 248);
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(154, 166, 190);
pub const TEXT_MUTED: Color32 = Color32::from_rgb(104, 116, 140);

pub const ACCENT_BLUE: Color32 = Color32::from_rgb(79, 142, 255);
pub const ACCENT_GREEN: Color32 = Color32::from_rgb(46, 204, 138);
pub const ACCENT_AMBER: Color32 = Color32::from_rgb(245, 179, 66);
pub const ACCENT_PURPLE: Color32 = Color32::from_rgb(164, 122, 255);
pub const ACCENT_CYAN: Color32 = Color32::from_rgb(56, 201, 232);
pub const ACCENT_RED: Color32 = Color32::from_rgb(240, 86, 106);

pub fn apply_theme(ctx: &egui::Context) {
    let mut style = Style::default();
    let mut visuals = Visuals::dark();

    visuals.panel_fill = BG_BASE;
    visuals.window_fill = BG_SURFACE;
    visuals.faint_bg_color = BG_SURFACE;
    visuals.extreme_bg_color = BG_CONSOLE;
    visuals.code_bg_color = BG_CONSOLE;

    visuals.widgets.noninteractive.bg_fill = BG_SURFACE;
    visuals.widgets.noninteractive.weak_bg_fill = BG_SURFACE;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER_SUBTLE);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_PRIMARY);

    visuals.widgets.inactive.bg_fill = BG_CARD;
    visuals.widgets.inactive.weak_bg_fill = BG_CARD;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER_SUBTLE);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT_PRIMARY);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(6);

    visuals.widgets.hovered.bg_fill = BG_CARD_HOVER;
    visuals.widgets.hovered.weak_bg_fill = BG_CARD_HOVER;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT_BLUE);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT_PRIMARY);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(6);

    visuals.widgets.active.bg_fill = BG_CARD_SELECTED;
    visuals.widgets.active.weak_bg_fill = BG_CARD_SELECTED;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT_BLUE);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, TEXT_PRIMARY);
    visuals.widgets.active.corner_radius = CornerRadius::same(6);

    visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(79, 142, 255, 70);
    visuals.selection.stroke = Stroke::new(1.0, ACCENT_BLUE);

    visuals.window_corner_radius = CornerRadius::same(10);
    visuals.window_stroke = Stroke::new(1.0, BORDER_SUBTLE);

    style.visuals = visuals;

    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.window_margin = Margin::same(14);
    style.spacing.scroll.bar_width = 7.0;

    style.text_styles = [
        (TextStyle::Heading, FontId::new(19.0, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace)),
        (TextStyle::Button, FontId::new(13.0, FontFamily::Proportional)),
        (TextStyle::Small, FontId::new(11.5, FontFamily::Proportional)),
    ]
    .into();

    ctx.set_style(style);
}

pub fn visibility_color(state: VisibilityState) -> Color32 {
    match state {
        VisibilityState::Visible => ACCENT_GREEN,
        VisibilityState::Minimized => ACCENT_AMBER,
        VisibilityState::Hidden => ACCENT_PURPLE,
    }
}

pub fn owner_color(cat: OwnerCategory) -> Color32 {
    match cat {
        OwnerCategory::Ide => ACCENT_BLUE,
        OwnerCategory::AiTool => ACCENT_CYAN,
        OwnerCategory::User => ACCENT_GREEN,
        OwnerCategory::Terminal => Color32::from_rgb(120, 175, 255),
        OwnerCategory::DevTool => ACCENT_AMBER,
        OwnerCategory::System => TEXT_MUTED,
        OwnerCategory::Other => TEXT_SECONDARY,
    }
}

pub fn purpose_color(tag: PurposeTag) -> Color32 {
    match tag {
        PurposeTag::AiAgent => ACCENT_CYAN,
        PurposeTag::DevServer => ACCENT_GREEN,
        PurposeTag::BuildOrTest => ACCENT_AMBER,
        PurposeTag::Script => ACCENT_BLUE,
        PurposeTag::GitOrVcs => Color32::from_rgb(240, 130, 90),
        PurposeTag::ShellHost => ACCENT_PURPLE,
        PurposeTag::IdleShell => TEXT_MUTED,
        PurposeTag::SystemHelper => TEXT_MUTED,
    }
}

pub fn badge(ui: &mut egui::Ui, text: &str, color: Color32) {
    let bg = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 28);
    let border = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 80);
    egui::Frame::new()
        .fill(bg)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(7, 2))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).color(color).size(11.0).strong());
        });
}
