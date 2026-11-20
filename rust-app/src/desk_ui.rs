use super::*;

pub const WORKSPACE: Color32 = Color32::from_gray(19);
pub const SURFACE: Color32 = Color32::from_gray(27);
pub const BORDER: Color32 = Color32::from_gray(58);
pub const TEXT: Color32 = Color32::from_gray(246);
pub const MUTED: Color32 = Color32::from_gray(172);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    Presets,
    Internet,
    Api,
    ApiDocs,
}

pub fn window(
    ctx: &Context,
    tool: Tool,
    title: &str,
    size: [f32; 2],
    body: impl FnOnce(&mut Ui),
) -> bool {
    let screen = ctx.screen_rect();
    let width = (screen.width() - 30.).clamp(280., size[0]);
    let center = screen.center();
    let mut closed = false;
    egui::Window::new(title)
        .id(egui::Id::new(tool))
        .default_pos(Pos2::new(
            center.x - width / 2.,
            (center.y - size[1] / 2.).max(screen.top() + 12.),
        ))
        .default_width(width)
        .resizable(false)
        .title_bar(false)
        .frame(
            Frame::new()
                .fill(WORKSPACE)
                .stroke(Stroke::new(1_f32, BORDER))
                .corner_radius(16.)
                .outer_margin(1.)
                .inner_margin(22.)
                .shadow(egui::epaint::Shadow {
                    offset: [0, 10],
                    blur: 32,
                    spread: 4,
                    color: Color32::from_black_alpha(120),
                }),
        )
        .show(ctx, |ui| {
            style(ui);
            ui.set_width(width - 46.);
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).size(25.).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    closed = ui
                        .add(control(RichText::new("×").size(17.)).min_size(Vec2::splat(34.)))
                        .on_hover_text(tr!("닫기", "Close"))
                        .clicked();
                });
            });
            ui.add_space(8.);
            egui::ScrollArea::vertical()
                .id_salt("tool-scroll")
                .max_height((screen.height() - 150.).clamp(125., 600.))
                .auto_shrink([false, true])
                .show(ui, body);
        });
    closed
}

pub fn heading(ui: &mut Ui, label: &str) {
    ui.label(RichText::new(label).size(16.).strong());
}

pub fn note(ui: &mut Ui, label: &str) {
    ui.add(egui::Label::new(RichText::new(label).color(MUTED)).wrap());
}

pub fn message(ui: &mut Ui, text: &str) {
    if !text.is_empty() {
        card().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.add(egui::Label::new(text).wrap());
        });
    }
}

pub fn tool_button(ui: &mut Ui, label: &str) -> bool {
    ui.add(control(label).min_size(Vec2::new(ui.available_width(), 34.)))
        .clicked()
}

pub fn style(ui: &mut Ui) {
    let style = ui.style_mut();
    style.spacing.item_spacing = Vec2::new(8., 8.);
    style.spacing.button_padding = Vec2::new(12., 6.);
    style.spacing.interact_size.y = 30.;
    style.visuals = monochrome(false);
    let visuals = &mut style.visuals;
    visuals.window_fill = WORKSPACE;
    visuals.panel_fill = WORKSPACE;
    visuals.extreme_bg_color = Color32::from_gray(13);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1_f32, TEXT);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1_f32, BORDER);
    for (widget, fill) in [
        (&mut visuals.widgets.inactive, SURFACE),
        (&mut visuals.widgets.hovered, Color32::from_gray(45)),
        (&mut visuals.widgets.active, Color32::from_gray(58)),
        (&mut visuals.widgets.open, Color32::from_gray(45)),
    ] {
        widget.bg_fill = fill;
        widget.weak_bg_fill = fill;
        widget.bg_stroke = Stroke::new(1_f32, BORDER);
        widget.fg_stroke = Stroke::new(1_f32, TEXT);
        widget.corner_radius = egui::CornerRadius::same(8);
    }
}

pub fn control(label: impl Into<RichText>) -> egui::Button<'static> {
    egui::Button::new(label.into())
        .min_size(Vec2::new(30., 30.))
        .corner_radius(8.)
}

pub fn filled(label: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(label).size(14.).strong().color(BLACK))
        .fill(TEXT)
        .stroke(Stroke::NONE)
        .min_size(Vec2::new(30., 34.))
        .corner_radius(8.)
}

pub fn card() -> Frame {
    Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1_f32, BORDER))
        .corner_radius(10.)
        .inner_margin(13.)
}
