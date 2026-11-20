use crate::desk_ui::{self, control, BORDER, MUTED, SURFACE, TEXT, WORKSPACE};

use super::*;

const STAGE: Color32 = Color32::from_gray(13);
const BATCH_SIZE: [f32; 2] = [424., 742.];

#[derive(Default)]
pub struct Batch {
    pub open: bool,
    pub leader: Option<String>,
    pub busy: bool,
    pub status: String,
    ids: Vec<String>,
    gesture: Option<(Pos2, f64, egui::Rect)>,
    focus_pending: bool,
    input_after: Option<Instant>,
}

#[derive(Clone)]
enum Input {
    Tap(Pos2),
    Swipe(Pos2, Pos2, u32),
    Key(&'static str),
    Install(PathBuf),
}

impl Desk {
    pub fn batch_targets(&self) -> Vec<Device> {
        if !self.control {
            return vec![];
        }
        self.ready()
            .into_iter()
            .filter(|device| self.can_control(&device.id))
            .collect()
    }

    pub(crate) fn can_control(&self, id: &str) -> bool {
        self.control
            && self
                .mirrors
                .get(id)
                .is_none_or(|mirror| mirror.controlled && !mirror.closing)
    }

    pub fn open_batch(&mut self, ctx: &Context) {
        if self.batch.busy {
            return;
        }
        let ids: Vec<_> = self
            .batch_targets()
            .into_iter()
            .map(|device| device.id)
            .collect();
        if ids.len() < 2 {
            return;
        }
        self.batch = Batch {
            open: true,
            leader: ids.first().cloned(),
            ids,
            focus_pending: true,
            input_after: Some(Instant::now() + Duration::from_millis(350)),
            ..Batch::default()
        };
        ctx.request_repaint();
    }

    fn eligible_batch(&self) -> Vec<Device> {
        self.batch
            .ids
            .iter()
            .filter(|id| self.can_control(id))
            .filter_map(|id| self.devices.get(id).filter(|device| device.ready()))
            .cloned()
            .collect()
    }

    pub fn render_batch(&mut self, ctx: &Context) {
        if !self.batch.open {
            return;
        }
        let targets = self.eligible_batch();
        if self
            .batch
            .leader
            .as_ref()
            .is_none_or(|id| !targets.iter().any(|device| &device.id == id))
        {
            self.batch.leader = targets.first().map(|device| device.id.clone());
            self.batch.gesture = None;
        }
        if let Some(leader) = self.batch.leader.clone() {
            self.queue_preview(ctx, &leader);
        }
        let mut close = false;
        let focus = std::mem::take(&mut self.batch.focus_pending);
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("batch-control"),
            egui::ViewportBuilder::default()
                .with_title(tr!("동시 조작", "Batch Control"))
                .with_inner_size(BATCH_SIZE)
                .with_min_inner_size([350., 465.])
                .with_active(true),
            |ctx, class| {
                if class == egui::ViewportClass::Embedded {
                    egui::Window::new(tr!("동시 조작", "Batch Control"))
                        .default_size(BATCH_SIZE)
                        .frame(
                            Frame::new()
                                .fill(WORKSPACE)
                                .stroke(Stroke::new(1_f32, BORDER)),
                        )
                        .show(ctx, |ui| self.batch_content(ui, &targets));
                    return;
                }
                close = ctx.input(|i| i.viewport().close_requested());
                if close {
                    return;
                }
                if focus {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                egui::CentralPanel::default()
                    .frame(Frame::new().fill(WORKSPACE))
                    .show(ctx, |ui| self.batch_content(ui, &targets));
            },
        );
        if close {
            self.batch.open = false;
            self.batch.gesture = None;
        }
    }

    fn batch_content(&mut self, ui: &mut Ui, targets: &[Device]) {
        desk_ui::style(ui);
        let selecting_leader = ui.memory(|memory| memory.any_popup_open());
        let permitted = targets.len() >= 2
            && !self.batch.busy
            && self
                .batch
                .input_after
                .is_none_or(|after| Instant::now() >= after);

        egui::TopBottomPanel::top(ui.id().with("batch-toolbar"))
            .exact_height(102.)
            .frame(
                Frame::new()
                    .fill(SURFACE)
                    .inner_margin(egui::Margin::symmetric(14, 11)),
            )
            .show_inside(ui, |ui| self.batch_toolbar(ui, targets));

        egui::TopBottomPanel::bottom(ui.id().with("batch-controls"))
            .exact_height(113.)
            .frame(
                Frame::new()
                    .fill(SURFACE)
                    .inner_margin(egui::Margin::symmetric(14, 11)),
            )
            .show_inside(ui, |ui| self.batch_controls(ui, targets, permitted));

        egui::CentralPanel::default()
            .frame(Frame::new().fill(STAGE))
            .show_inside(ui, |ui| {
                self.batch_preview(
                    ui,
                    targets,
                    permitted && !selecting_leader && !self.batch.busy,
                );
            });
    }

    fn batch_toolbar(&mut self, ui: &mut Ui, targets: &[Device]) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(tr!("동시 조작", "Batch control"))
                    .size(22.)
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(trf!("{}대 제어 가능", "{} available", targets.len()))
                        .color(MUTED)
                        .size(13.),
                );
            });
        });
        ui.add_space(3.);
        ui.horizontal(|ui| {
            ui.spacing_mut().interact_size.y = 36.;
            let previous = self.batch.leader.clone();
            let selected = previous
                .as_ref()
                .map(|id| self.label(id))
                .unwrap_or_else(|| tr!("연결된 기기 없음", "None connected").into());
            let preset = tr!("프리셋", "Presets");
            ui.label(RichText::new(tr!("대표 기기", "Reference device")).color(MUTED));
            let preset_width = 82.;
            let combo_width =
                (ui.available_width() - preset_width - ui.spacing().item_spacing.x).max(70.);
            ui.add_enabled_ui(!targets.is_empty(), |ui| {
                egui::ComboBox::from_id_salt("batch-leader")
                    .width(combo_width)
                    .selected_text(selected)
                    .truncate()
                    .show_ui(ui, |ui| {
                        for device in targets {
                            let label = self.label(&device.id);
                            ui.selectable_value(
                                &mut self.batch.leader,
                                Some(device.id.clone()),
                                label,
                            );
                        }
                    });
            });
            if self.batch.leader != previous {
                self.batch.gesture = None;
            }
            if ui
                .add(control(preset).min_size(Vec2::new(preset_width, 36.)))
                .clicked()
            {
                self.open_selected_presets();
                ui.ctx()
                    .send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
            }
        });
    }

    fn batch_preview(&mut self, ui: &mut Ui, targets: &[Device], permitted: bool) {
        let (area, _) = ui.allocate_exact_size(ui.available_size_before_wrap(), Sense::hover());
        ui.painter().rect_filled(area, 0., STAGE);
        let area = area.shrink(14.);
        if let Some(texture) = self
            .batch
            .leader
            .as_ref()
            .and_then(|id| self.images.get(id))
        {
            let dimensions = texture.size_vec2();
            let scale = (area.width() / dimensions.x).min(area.height() / dimensions.y);
            let rect = egui::Rect::from_center_size(area.center(), dimensions * scale);
            let bezel = rect.expand(5.);
            ui.painter().rect_filled(bezel, 16., Color32::from_gray(30));
            ui.painter().rect_stroke(
                bezel,
                16.,
                Stroke::new(1_f32, BORDER),
                egui::StrokeKind::Inside,
            );
            egui::Image::new((texture.id(), rect.size()))
                .corner_radius(11.)
                .paint_at(ui, rect);
            if permitted {
                ui.interact(rect, ui.id().with("batch-touch"), Sense::click_and_drag())
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                let (press, release, cursor, time) = ui.input(|input| {
                    (
                        input.events.iter().find_map(|event| match event {
                            egui::Event::PointerButton {
                                pos,
                                button: egui::PointerButton::Primary,
                                pressed: true,
                                ..
                            } => Some(*pos),
                            _ => None,
                        }),
                        input.events.iter().rev().find_map(|event| match event {
                            egui::Event::PointerButton {
                                pos,
                                button: egui::PointerButton::Primary,
                                pressed: false,
                                ..
                            } => Some(*pos),
                            _ => None,
                        }),
                        input.pointer.interact_pos(),
                        input.time,
                    )
                });
                // A release inherited from opening this viewport cannot arm a gesture.
                if let Some(at) = press {
                    let on_preview =
                        rect.contains(at) && ui.ctx().layer_id_at(at) == Some(ui.layer_id());
                    self.batch.gesture = on_preview.then_some((at, time, rect));
                }
                if release.is_none() {
                    if let (Some((origin, _, _)), Some(at)) = (self.batch.gesture, cursor) {
                        ui.painter()
                            .with_clip_rect(rect)
                            .line_segment([origin, at], Stroke::new(2.0_f32, Color32::WHITE));
                    }
                }
                if let Some(end) = release {
                    if let Some((start, begin, origin_rect)) = self.batch.gesture.take() {
                        if start.distance(end) >= 7. {
                            let duration = ((time - begin) * 1000.).clamp(120., 1600.) as u32;
                            self.dispatch_batch(
                                ui.ctx(),
                                targets,
                                tr!("스와이프", "Swipe"),
                                Input::Swipe(
                                    relative(origin_rect, start),
                                    relative(origin_rect, end),
                                    duration,
                                ),
                            );
                        } else if origin_rect.contains(end) {
                            self.dispatch_batch(
                                ui.ctx(),
                                targets,
                                tr!("탭", "Tap"),
                                Input::Tap(relative(origin_rect, end)),
                            );
                        }
                    }
                }
            } else {
                self.batch.gesture = None;
            }
        } else {
            self.batch.gesture = None;
            let message = if self.batch.leader.is_some() {
                tr!("대표 화면 불러오는 중", "Loading reference screen")
            } else {
                tr!("연결된 기기 없음", "No connected devices")
            };
            ui.painter().text(
                area.center(),
                egui::Align2::CENTER_CENTER,
                message,
                egui::TextStyle::Body.resolve(ui.style()),
                MUTED,
            );
        }
    }

    fn batch_controls(&mut self, ui: &mut Ui, targets: &[Device], permitted: bool) {
        ui.columns(4, |columns| {
            for (ui, action) in columns.iter_mut().zip([
                BatchAction::Recents,
                BatchAction::Home,
                BatchAction::Back,
                BatchAction::Install,
            ]) {
                if action.button(ui, permitted) {
                    let key = match action {
                        BatchAction::Recents => "KEYCODE_APP_SWITCH",
                        BatchAction::Home => "KEYCODE_HOME",
                        BatchAction::Back => "KEYCODE_BACK",
                        BatchAction::Install => {
                            self.choose_batch_apk(ui.ctx(), targets);
                            continue;
                        }
                    };
                    self.dispatch_batch(ui.ctx(), targets, action.label(), Input::Key(key));
                }
            }
        });
        ui.separator();
        let status = if !self.batch.status.is_empty() {
            self.batch.status.as_str()
        } else if targets.len() >= 2 {
            tr!("입력 대기 중", "Ready for input")
        } else {
            tr!(
                "제어 가능한 기기가 두 대 이상 필요합니다.",
                "Connect at least two controllable devices."
            )
        };
        ui.horizontal(|ui| {
            if self.batch.busy {
                ui.add(egui::Spinner::new().size(12.));
            } else {
                let (marker, _) = ui.allocate_exact_size(Vec2::splat(12.), Sense::hover());
                let color = if targets.len() >= 2 { MUTED } else { BORDER };
                ui.painter().circle_filled(marker.center(), 2.5, color);
            }
            ui.add(egui::Label::new(RichText::new(status).size(12.).color(MUTED)).truncate())
                .on_hover_text(status);
        });
    }

    fn choose_batch_apk(&mut self, ctx: &Context, targets: &[Device]) {
        let Some(file) = rfd::FileDialog::new()
            .add_filter("Android APK", &["apk"])
            .pick_file()
        else {
            return;
        };
        if rfd::MessageDialog::new()
            .set_title(tr!("APK 일괄 설치", "Batch APK installation"))
            .set_description(trf!(
                "선택한 {}대에 APK를 설치할까요?",
                "Install the APK on the {} selected devices?",
                targets.len()
            ))
            .set_buttons(rfd::MessageButtons::YesNo)
            .show()
            == rfd::MessageDialogResult::Yes
        {
            self.dispatch_batch(
                ctx,
                targets,
                tr!("APK 설치", "Install APK"),
                Input::Install(file),
            );
        }
    }

    fn dispatch_batch(&mut self, ctx: &Context, targets: &[Device], label: &str, input: Input) {
        if self.batch.busy || !self.control || targets.len() < 2 {
            return;
        }
        self.batch.busy = true;
        self.batch.status = trf!("{label} 전송 중…", "Sending {label}…");
        let reference = self
            .batch
            .leader
            .as_ref()
            .and_then(|id| self.screen_sizes.get(id))
            .map(|(size, _)| *size);
        let root = self.root.clone();
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let label = label.to_owned();
        let targets: Vec<_> = targets
            .iter()
            .cloned()
            .map(|device| {
                let size = self
                    .screen_sizes
                    .get(&device.id)
                    .filter(|(_, time)| time.elapsed() < Duration::from_secs(6))
                    .map(|(size, _)| *size);
                (device, size)
            })
            .collect();
        thread::spawn(move || {
            let mut delivered = 0;
            let mut failure = None;
            for chunk in targets.chunks(6) {
                thread::scope(|scope| {
                    let jobs: Vec<_> = chunk
                        .iter()
                        .map(|(device, size)| {
                            let root = &root;
                            let input = &input;
                            let size = *size;
                            scope.spawn(move || send_input(root, device, size, reference, input))
                        })
                        .collect();
                    for job in jobs {
                        match job.join().unwrap_or_else(|_| {
                            Err(tr!("입력 전송 실패", "Could not send input").into())
                        }) {
                            Ok(()) => delivered += 1,
                            Err(error) if failure.is_none() => failure = Some(error),
                            Err(_) => (),
                        }
                    }
                });
            }
            let _ = tx.send(Event::BatchDone(delivered, targets.len(), label, failure));
            ctx.request_repaint();
        });
    }
}

#[derive(Clone, Copy)]
enum BatchAction {
    Recents,
    Home,
    Back,
    Install,
}

impl BatchAction {
    fn label(self) -> &'static str {
        match self {
            Self::Recents => tr!("최근 앱", "Recents"),
            Self::Home => tr!("홈", "Home"),
            Self::Back => tr!("뒤로", "Back"),
            Self::Install => tr!("APK 설치", "Install APK"),
        }
    }

    fn button(self, ui: &mut Ui, enabled: bool) -> bool {
        let label = self.label();
        let button = control("").min_size(Vec2::new(ui.available_width(), 54.));
        let response = ui.add_enabled(enabled, button);
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
        let color = if enabled { TEXT } else { MUTED };
        let rect = response.rect;
        let painter = ui.painter_at(rect);
        self.paint_icon(
            &painter,
            Pos2::new(rect.center().x, rect.top() + 17.),
            color,
        );
        painter.text(
            Pos2::new(rect.center().x, rect.bottom() - 12.),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(13.),
            color,
        );
        response.clicked()
    }

    fn paint_icon(self, painter: &egui::Painter, center: Pos2, color: Color32) {
        let stroke = Stroke::new(1.4_f32, color);
        let point = |x, y| center + Vec2::new(x, y);
        let line = |points: &[(f32, f32)]| {
            painter.add(egui::Shape::line(
                points.iter().map(|&(x, y)| point(x, y)).collect(),
                stroke,
            ));
        };
        match self {
            Self::Recents => {
                let bounds = egui::Rect::from_min_max(point(-8., -2.), point(3., 7.));
                painter.rect_stroke(bounds, 2., stroke, egui::StrokeKind::Inside);
                line(&[(-4., -4.), (-4., -6.), (7., -6.), (7., 4.), (5., 4.)]);
            }
            Self::Home => line(&[
                (-8., 0.),
                (0., -7.),
                (8., 0.),
                (6., 0.),
                (6., 7.),
                (-6., 7.),
                (-6., 0.),
                (-8., 0.),
            ]),
            Self::Back => {
                line(&[(0., -6.), (-6., 0.), (0., 6.)]);
                line(&[(-6., 0.), (7., 0.)]);
            }
            Self::Install => {
                line(&[(0., -8.), (0., 2.)]);
                line(&[(-4., -2.), (0., 2.), (4., -2.)]);
                line(&[(-7., 2.), (-7., 7.), (7., 7.), (7., 2.)]);
            }
        }
    }
}

fn relative(rect: egui::Rect, point: Pos2) -> Pos2 {
    Pos2::new(
        ((point.x - rect.left()) / rect.width()).clamp(0., 1.),
        ((point.y - rect.top()) / rect.height()).clamp(0., 1.),
    )
}

fn pixels(point: Pos2, (width, height): engine::ScreenSize) -> [String; 2] {
    [
        (point.x * width.saturating_sub(1) as f32)
            .round()
            .to_string(),
        (point.y * height.saturating_sub(1) as f32)
            .round()
            .to_string(),
    ]
}

fn send_input(
    root: &std::path::Path,
    device: &Device,
    cached: Option<engine::ScreenSize>,
    reference: Option<engine::ScreenSize>,
    input: &Input,
) -> Result<(), String> {
    let mut command = engine::adb_command(root, device);
    match input {
        Input::Key(key) => {
            command.args(["shell", "input", "keyevent", key]);
        }
        Input::Install(path) => {
            command.args(["install", "-r"]).arg(path);
        }
        Input::Tap(_) | Input::Swipe(..) => {
            let size = match cached {
                Some(value) => value,
                None => engine::screen_size(root, device)?,
            };
            if reference.is_some_and(|(width, height)| (width > height) != (size.0 > size.1)) {
                return Err(tr!(
                    "대표 화면과 방향이 다른 기기는 건너뛰었습니다.",
                    "Skipped a device with a different screen orientation."
                )
                .into());
            }
            command.args(["shell", "input", "touchscreen"]);
            match input {
                Input::Tap(point) => {
                    command.arg("tap").args(pixels(*point, size));
                }
                Input::Swipe(start, end, duration) => {
                    command
                        .arg("swipe")
                        .args(pixels(*start, size))
                        .args(pixels(*end, size))
                        .arg(duration.to_string());
                }
                _ => unreachable!(),
            }
        }
    }
    let result = engine::run(
        &mut command,
        if matches!(input, Input::Install(_)) {
            120
        } else {
            6
        },
    )?;
    let response = String::from_utf8_lossy(&result.stdout);
    if response.contains("Failure") || response.starts_with("Error:") {
        return Err(response.trim().to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_resizes_between_fixed_batch_controls() {
        let ctx = Context::default();
        let mut desk = Desk::new(PathBuf::new(), &ctx);
        let mut draw = |height| {
            let size = Vec2::new(350., height);
            let screen = egui::Rect::from_min_size(Pos2::ZERO, size);
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .frame(Frame::new())
                        .show(ctx, |ui| {
                            desk.batch_content(ui, &[]);
                        });
                },
            );
            let preview = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect) if rect.fill == STAGE => Some(rect.rect),
                    _ => None,
                })
                .unwrap();
            for (label, inside) in [
                (tr!("대표 기기", "Reference device"), 0. ..preview.top()),
                (tr!("APK 설치", "Install APK"), preview.bottom()..height),
                (
                    tr!(
                        "제어 가능한 기기가 두 대 이상 필요합니다.",
                        "Connect at least two controllable devices."
                    ),
                    preview.bottom()..height,
                ),
            ] {
                assert!(
                    output.shapes.iter().any(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == label => {
                            inside.contains(&text.pos.y) && shape.clip_rect.contains(text.pos)
                        }
                        _ => false,
                    }),
                    "{label} should be visible at height {height}"
                );
            }
            (preview.top(), height - preview.bottom(), preview.height())
        };
        let compact = draw(465.);
        let expanded = draw(BATCH_SIZE[1]);
        assert_eq!(compact.0, expanded.0);
        assert_eq!(compact.1, expanded.1);
        assert_eq!(expanded.2 - compact.2, BATCH_SIZE[1] - 465.);
    }

    #[test]
    fn maps_gesture_from_preview_to_each_screen_size() {
        let preview = egui::Rect::from_min_size(Pos2::new(100., 200.), Vec2::new(200., 400.));
        let center = relative(preview, Pos2::new(200., 400.));
        assert_eq!(pixels(center, (1080, 2640)), ["540", "1320"]);
        assert_eq!(pixels(center, (720, 1440)), ["360", "720"]);
        let outside = relative(preview, Pos2::new(0., 900.));
        assert_eq!(pixels(outside, (720, 1440)), ["0", "1439"]);
    }
}
