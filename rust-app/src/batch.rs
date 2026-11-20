use super::*;

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
            .is_some_and(|id| !targets.iter().any(|device| &device.id == id))
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
                .with_inner_size([400., 675.])
                .with_min_inner_size([335., 465.])
                .with_active(true),
            |ctx, class| {
                if class == egui::ViewportClass::Embedded {
                    egui::Window::new(tr!("동시 조작", "Batch Control"))
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
                egui::TopBottomPanel::top("batch-title")
                    .frame(Frame::new().fill(BLACK).inner_margin(12))
                    .show(ctx, |ui| {
                        ui.label(
                            RichText::new(trf!(
                                "동시 조작 · {}대",
                                "Batch Control · {} devices",
                                targets.len()
                            ))
                            .color(Color32::WHITE)
                            .size(18.)
                            .strong(),
                        );
                    });
                egui::CentralPanel::default()
                    .frame(Frame::new().fill(PAPER).inner_margin(12))
                    .show(ctx, |ui| {
                        ui.visuals_mut().clone_from(&monochrome(true));
                        self.batch_content(ui, &targets);
                    });
            },
        );
        if close {
            self.batch.open = false;
            self.batch.gesture = None;
        }
    }

    fn batch_content(&mut self, ui: &mut Ui, targets: &[Device]) {
        let permitted = targets.len() >= 2
            && !self.batch.busy
            && self
                .batch
                .input_after
                .is_none_or(|after| Instant::now() >= after);
        ui.horizontal(|ui| {
            ui.label(tr!("대표 화면", "Reference screen"));
            let previous = self.batch.leader.clone();
            let selected = previous
                .as_ref()
                .map(|id| self.label(id))
                .unwrap_or_else(|| tr!("연결된 기기 없음", "No connected devices").to_owned());
            egui::ComboBox::from_id_salt("batch-leader")
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    for device in targets {
                        let label = self.label(&device.id);
                        ui.selectable_value(&mut self.batch.leader, Some(device.id.clone()), label);
                    }
                });
            if self.batch.leader != previous {
                self.batch.gesture = None;
            }
        });
        ui.label(tr!(
            "대표 화면을 클릭하거나 스와이프하면 선택한 기기에 전달됩니다.",
            "Click or swipe the reference screen to control the selected devices."
        ));
        ui.add_space(4.);
        let height = (ui.available_height() - 107.).max(140.);
        let (area, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
        ui.painter().rect_filled(area, 8., BLACK);
        if let Some(texture) = self
            .batch
            .leader
            .as_ref()
            .and_then(|id| self.images.get(id))
        {
            let dimensions = texture.size_vec2();
            let scale = (area.width() / dimensions.x).min(area.height() / dimensions.y);
            let rect = egui::Rect::from_center_size(area.center(), dimensions * scale);
            ui.painter().image(
                texture.id(),
                rect,
                egui::Rect::from_min_max(Pos2::ZERO, Pos2::new(1., 1.)),
                Color32::WHITE,
            );
            if permitted {
                ui.interact(rect, ui.id().with("batch-touch"), Sense::click_and_drag());
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
                    self.batch.gesture = rect.contains(at).then_some((at, time, rect));
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
            ui.painter().text(
                area.center(),
                egui::Align2::CENTER_CENTER,
                tr!("대표 화면 불러오는 중", "Loading reference screen"),
                egui::FontId::proportional(14.),
                Color32::LIGHT_GRAY,
            );
        }
        ui.add_space(5.);
        ui.horizontal(|ui| {
            for (label, key) in [
                (tr!("최근 앱", "Recents"), "KEYCODE_APP_SWITCH"),
                (tr!("홈", "Home"), "KEYCODE_HOME"),
                (tr!("뒤로", "Back"), "KEYCODE_BACK"),
            ] {
                if ui
                    .add_enabled(
                        permitted,
                        egui::Button::new(label).min_size(Vec2::new(62., 26.)),
                    )
                    .clicked()
                {
                    self.dispatch_batch(ui.ctx(), targets, label, Input::Key(key));
                }
            }
            if ui
                .add_enabled(
                    permitted,
                    egui::Button::new(tr!("APK 설치", "Install APK")).min_size(Vec2::new(62., 26.)),
                )
                .clicked()
            {
                if let Some(file) = rfd::FileDialog::new()
                    .add_filter("Android APK", &["apk"])
                    .pick_file()
                {
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
                            ui.ctx(),
                            targets,
                            tr!("APK 설치", "Install APK"),
                            Input::Install(file),
                        );
                    }
                }
            }
        });
        if !self.batch.status.is_empty() {
            ui.add(egui::Label::new(RichText::new(&self.batch.status).small()).truncate());
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
    fn maps_gesture_from_preview_to_each_screen_size() {
        let preview = egui::Rect::from_min_size(Pos2::new(100., 200.), Vec2::new(200., 400.));
        let center = relative(preview, Pos2::new(200., 400.));
        assert_eq!(pixels(center, (1080, 2640)), ["540", "1320"]);
        assert_eq!(pixels(center, (720, 1440)), ["360", "720"]);
        let outside = relative(preview, Pos2::new(0., 900.));
        assert_eq!(pixels(outside, (720, 1440)), ["0", "1439"]);
    }
}
