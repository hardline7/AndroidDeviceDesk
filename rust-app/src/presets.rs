use crate::api::{self, Endpoint, Job, Key, Status, Step};

use super::*;
use crate::desk_ui::{self, card, control, filled, Tool, MUTED, TEXT};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Catalog {
    next: u64,
    pub items: BTreeMap<u64, Preset>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub steps: Vec<Step>,
}

#[derive(Default)]
pub struct Panel {
    history: bool,
    selected: Option<u64>,
    editor: Option<(Option<u64>, Preset)>,
    tag: Option<String>,
    focused_job: Option<u64>,
    message: String,
}

impl Desk {
    fn open_presets(&mut self) {
        self.tool = Some(Tool::Presets);
    }

    pub fn open_selected_presets(&mut self) {
        self.presets.tag = None;
        self.presets.history = false;
        self.open_presets();
    }

    pub fn preset_button(&mut self, ui: &mut Ui) {
        if desk_ui::tool_button(ui, tr!("작업 프리셋", "Job presets")) {
            self.open_presets();
        }
    }

    pub fn preset_window(&mut self, ctx: &Context) {
        if self.tool != Some(Tool::Presets) {
            return;
        }
        let closed = desk_ui::window(
            ctx,
            Tool::Presets,
            tr!("작업 프리셋", "Job presets"),
            [690., 450.],
            |ui| {
                ui.horizontal(|ui| {
                    for (history, label) in [
                        (false, tr!("프리셋", "Presets")),
                        (true, tr!("실행 결과", "Run history")),
                    ] {
                        let selected = self.presets.history == history;
                        let button = if selected {
                            filled(label)
                        } else {
                            control(label)
                        };
                        if ui.add(button.min_size(Vec2::new(104., 34.))).clicked() {
                            self.presets.history = history;
                            self.presets.message.clear();
                        }
                    }
                });
                ui.add_space(8.);
                ui.separator();
                ui.add_space(8.);
                if self.presets.history {
                    self.preset_history(ui);
                } else {
                    self.preset_content(ui);
                }
                desk_ui::message(ui, &self.presets.message);
            },
        );
        if closed {
            self.tool = None;
        }
    }

    fn new_preset(&mut self) {
        self.presets.editor = Some((None, Preset::default()));
        self.presets.message.clear();
    }

    fn preset_content(&mut self, ui: &mut Ui) {
        if let Some((id, mut draft)) = self.presets.editor.take() {
            let (save, cancel) = edit(ui, &mut draft, id.is_some());
            if save {
                match self.save_preset(id, draft.clone()) {
                    Ok(saved) => {
                        self.presets.selected = Some(saved);
                        self.presets.message =
                            tr!("프리셋을 저장했습니다.", "Preset saved.").into();
                        return;
                    }
                    Err(error) => self.presets.message = error,
                }
            }
            if cancel {
                self.presets.message.clear();
            } else {
                self.presets.editor = Some((id, draft));
            }
            return;
        }

        if self.preferences.presets.items.is_empty() {
            if empty_state(
                ui,
                tr!("저장된 프리셋이 없어요", "No saved presets yet"),
                tr!(
                    "앱 실행, 사이트 열기, 탭을 묶어 여러 기기에서 반복 실행하세요.",
                    "Save a sequence of app launches, websites, and taps to run across devices."
                ),
                Some(tr!("＋  새 프리셋", "＋  New preset")),
                false,
            ) {
                self.new_preset();
            }
            return;
        }

        let items = &self.preferences.presets.items;
        let default_id = *items.keys().next().unwrap();
        if self
            .presets
            .selected
            .is_none_or(|id| !items.contains_key(&id))
        {
            self.presets.selected = Some(default_id);
        }

        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tr!("저장된 작업", "Saved presets")).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(filled(tr!("＋  새 프리셋", "＋  New preset")))
                    .clicked()
                {
                    self.presets.editor = Some((None, Preset::default()));
                    self.presets.message.clear();
                }
            });
        });
        egui::ScrollArea::vertical()
            .id_salt("preset-list")
            .max_height(138.)
            .show(ui, |ui| {
                for (&id, preset) in items {
                    let selected = self.presets.selected == Some(id);
                    let label = trf!(
                        "{}  ·  {}단계",
                        "{}  ·  {} steps",
                        preset.name,
                        preset.steps.len()
                    );
                    let width = ui.available_width();
                    if ui
                        .add(
                            (if selected {
                                filled(label)
                            } else {
                                control(label)
                            })
                            .min_size(Vec2::new(width, 34.))
                            .truncate(),
                        )
                        .clicked()
                    {
                        self.presets.selected = Some(id);
                        self.presets.message.clear();
                    }
                }
            });

        let id = self.presets.selected.unwrap_or(default_id);
        let preset = items[&id].clone();
        ui.add_space(4.);
        card().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr!("실행 순서", "Steps")).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(control(tr!("삭제", "Delete"))).clicked()
                        && rfd::MessageDialog::new()
                            .set_title(tr!("프리셋 삭제", "Delete preset"))
                            .set_description(trf!(
                                "‘{}’ 프리셋을 삭제할까요?",
                                "Delete the “{}” preset?",
                                preset.name
                            ))
                            .set_buttons(rfd::MessageButtons::YesNo)
                            .show()
                            == rfd::MessageDialogResult::Yes
                    {
                        let mut catalog = self.preferences.presets.clone();
                        catalog.items.remove(&id);
                        self.presets.message = match self.store_presets(catalog) {
                            Ok(()) => tr!("프리셋을 삭제했습니다.", "Preset deleted.").into(),
                            Err(error) => error,
                        };
                    }
                    if ui.add(control(tr!("편집", "Edit"))).clicked() {
                        self.presets.editor = Some((Some(id), preset.clone()));
                        self.presets.message.clear();
                    }
                });
            });
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("preset-steps")
                .max_height(138.)
                .show(ui, |ui| {
                    for (index, step) in preset.steps.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("{:02}", index + 1))
                                    .monospace()
                                    .color(MUTED),
                            );
                            let summary = step_summary(step);
                            ui.add(egui::Label::new(&summary).truncate())
                                .on_hover_text(summary);
                        });
                    }
                });
        });
        card().show(ui, |ui| {
            let targets = self.preset_targets(ui);
            if ui
                .add_enabled(
                    !targets.is_empty() && !self.api.core.busy() && !self.batch.busy,
                    filled(trf!("{}대에서 실행", "Run on {} devices", targets.len()))
                        .min_size(Vec2::new(ui.available_width(), 38.)),
                )
                .clicked()
            {
                self.update_api();
                let result = self.api.core.run_preset(preset, targets, None);
                self.show_job(result);
            }
        });
    }

    fn preset_targets(&mut self, ui: &mut Ui) -> Vec<Endpoint> {
        let devices = self.api_devices();
        let tags: BTreeSet<_> = self
            .preferences
            .devices
            .values()
            .map(|note| note.tag.as_str())
            .filter(|tag| !tag.is_empty())
            .collect();
        if self
            .presets
            .tag
            .as_ref()
            .is_some_and(|tag| !tags.contains(tag.as_str()))
        {
            self.presets.tag = None;
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr!("실행 대상", "Run targets")).strong());
            let current = self.presets.tag.as_ref().map_or_else(
                || tr!("선택한 기기", "Selected devices").into(),
                |tag| trf!("태그 · {tag}", "Tag · {tag}"),
            );
            egui::ComboBox::from_id_salt("preset-targets")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.presets.tag,
                        None,
                        tr!("선택한 기기", "Selected devices"),
                    );
                    for tag in tags {
                        ui.selectable_value(
                            &mut self.presets.tag,
                            Some(tag.to_owned()),
                            trf!("태그 · {tag}", "Tag · {tag}"),
                        );
                    }
                });
        });

        let selected: BTreeSet<_> = self
            .batch_targets()
            .iter()
            .map(|device| self.preferences.devices[&device.id].number)
            .collect();
        let targets: Vec<_> = devices
            .into_values()
            .filter(|device| device.can_run())
            .filter(|device| match &self.presets.tag {
                Some(tag) => &device.tag == tag,
                None => selected.contains(&device.number),
            })
            .collect();
        if targets.is_empty() {
            ui.label(
                RichText::new(tr!(
                    "기기 격자에서 기기를 선택하거나 태그를 고르세요.",
                    "Select devices in the grid or choose a tag."
                ))
                .color(MUTED),
            );
        } else {
            let labels: Vec<_> = targets.iter().map(Endpoint::label).collect();
            ui.add(egui::Label::new(labels.join(" · ")).truncate())
                .on_hover_text(labels.join("\n"));
        }
        targets
    }

    fn save_preset(&mut self, id: Option<u64>, mut preset: Preset) -> Result<u64, String> {
        preset.name = preset.name.trim().to_owned();
        if preset.name.is_empty() || preset.name.chars().count() > 80 {
            return Err(tr!(
                "이름은 1~80자로 입력하세요.",
                "Enter a name of 1 to 80 characters."
            )
            .into());
        }
        api::validate_steps(&preset.steps).map_err(validation_label)?;
        let mut catalog = self.preferences.presets.clone();
        let id = id.unwrap_or_else(|| {
            catalog.next += 1;
            catalog.next
        });
        catalog.items.insert(id, preset);
        self.store_presets(catalog)?;
        Ok(id)
    }

    fn store_presets(&mut self, catalog: Catalog) -> Result<(), String> {
        let previous = std::mem::replace(&mut self.preferences.presets, catalog);
        if let Err(error) = self.write_preferences() {
            self.preferences.presets = previous;
            return Err(trf!(
                "프리셋 저장 실패 · {error}",
                "Could not save presets · {error}"
            ));
        }
        self.api.core.set_presets(self.preferences.presets.clone());
        Ok(())
    }

    fn show_job(&mut self, result: Result<Job, api::JobError>) {
        match result {
            Ok(job) => {
                self.presets.history = true;
                self.presets.focused_job = Some(job.id);
                self.presets.message.clear();
            }
            Err(error) => self.presets.message = error.label(),
        }
    }

    fn preset_history(&mut self, ui: &mut Ui) {
        let jobs = self.api.core.history();
        let Some(latest) = jobs.first() else {
            empty_state(
                ui,
                tr!("아직 실행한 작업이 없어요", "No runs yet"),
                tr!(
                    "작업을 실행하면 기기별 완료 단계와 실패 이유를 여기에서 볼 수 있어요.",
                    "Once a job runs, its steps and any errors will appear here for each device."
                ),
                None,
                true,
            );
            return;
        };
        let selected = jobs
            .iter()
            .find(|job| Some(job.id) == self.presets.focused_job)
            .unwrap_or(latest);
        self.presets.focused_job = Some(selected.id);
        egui::ComboBox::from_id_salt("history-choice")
            .width(310.)
            .selected_text(job_title(selected))
            .show_ui(ui, |ui| {
                for job in &jobs {
                    ui.selectable_value(
                        &mut self.presets.focused_job,
                        Some(job.id),
                        job_title(job),
                    );
                }
            });
        let job = jobs
            .iter()
            .find(|job| Some(job.id) == self.presets.focused_job)
            .unwrap_or(latest);
        let succeeded = job
            .results
            .iter()
            .filter(|item| item.status == Status::Succeeded)
            .count();
        card().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            if job.status == Status::Running {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(trf!(
                        "{}대에서 작업 중",
                        "Running on {} devices",
                        job.count()
                    ));
                });
            } else {
                ui.label(
                    RichText::new(trf!(
                        "{}대 중 {succeeded}대 성공",
                        "{succeeded} of {} devices succeeded",
                        job.count()
                    ))
                    .size(21.)
                    .strong(),
                );
            }
        });

        egui::ScrollArea::vertical()
            .id_salt("history-results")
            .max_height(330.)
            .show(ui, |ui| {
                for entry in &job.results {
                    card().show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.horizontal_wrapped(|ui| {
                            ui.strong(format!(
                                "{:02} · {}",
                                entry.number,
                                status_label(entry.status)
                            ));
                            ui.label(trf!(
                                "{} / {} 단계 완료",
                                "{} of {} steps completed",
                                entry.completed_steps,
                                job.step_count()
                            ));
                            if let Some(step) = entry.failed_step {
                                ui.label(trf!("{step}단계에서 중단", "Stopped at step {step}"));
                            }
                        });
                        if let Some(error) = &entry.error {
                            let display = match error.as_str() {
                                "Device disconnected or became view-only." => tr!(
                                    "기기 연결이 끊겼거나 보기 전용으로 바뀌었습니다.",
                                    "Device disconnected or became view-only."
                                ),
                                "Server stopped." => {
                                    tr!("API 서버가 중지되었습니다.", "Server stopped.")
                                }
                                "Worker failed." => {
                                    tr!("기기 작업을 완료하지 못했습니다.", "Worker failed.")
                                }
                                _ => error.as_str(),
                            };
                            ui.add(egui::Label::new(RichText::new(display).weak()).wrap());
                        }
                    });
                }
            });
        let failed = job.failed_count();
        if failed > 0 {
            ui.label(tr!(
                "실패한 기기에서 이 작업의 첫 단계부터 다시 실행합니다.",
                "Replay this job from the first step on failed devices."
            ));
            if ui
                .add_enabled(
                    !self.api.core.busy() && !self.batch.busy,
                    filled(trf!(
                        "실패한 {failed}대만 재실행",
                        "Retry {failed} failed devices"
                    )),
                )
                .clicked()
            {
                self.update_api();
                let result = self.api.core.retry_job(job.id, None);
                self.show_job(result);
            }
        }
    }
}

fn empty_state(
    ui: &mut Ui,
    title: &str,
    message: &str,
    action: Option<&str>,
    history: bool,
) -> bool {
    card()
        .show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(16.);
                let (icon, _) = ui.allocate_exact_size(Vec2::splat(46.), Sense::hover());
                let icon = egui::Rect::from_center_size(
                    Pos2::new(ui.max_rect().center().x, icon.center().y),
                    icon.size(),
                );
                let painter = ui.painter();
                painter.rect_filled(icon, 13., Color32::from_gray(42));
                let mark = icon.shrink(13.);
                let stroke = Stroke::new(1.6_f32, TEXT);
                if history {
                    painter.circle_stroke(mark.center(), 9.5, stroke);
                    painter
                        .line_segment([mark.center() - Vec2::new(0., 5.), mark.center()], stroke);
                    painter
                        .line_segment([mark.center(), mark.center() + Vec2::new(4., 2.)], stroke);
                } else {
                    painter.rect_stroke(mark, 4., stroke, egui::StrokeKind::Inside);
                    for offset in [-4., 1., 6.] {
                        let y = mark.center().y + offset;
                        painter.line_segment(
                            [
                                Pos2::new(mark.left() + 5., y),
                                Pos2::new(mark.right() - 5., y),
                            ],
                            stroke,
                        );
                    }
                }
                ui.add_space(8.);
                ui.label(RichText::new(title).size(21.).strong());
                ui.add(egui::Label::new(RichText::new(message).color(MUTED)).wrap());
                let clicked = action.is_some_and(|label| {
                    ui.add_space(9.);
                    ui.add(filled(label).min_size(Vec2::new(146., 38.)))
                        .clicked()
                });
                ui.add_space(16.);
                clicked
            })
            .inner
        })
        .inner
}

fn job_title(job: &Job) -> String {
    let name = job.name.as_deref().unwrap_or(tr!("API 작업", "API job"));
    format!("#{} · {} · {}", job.id, name, status_label(job.status))
}

fn status_label(status: Status) -> &'static str {
    match status {
        Status::Running => tr!("실행 중", "Running"),
        Status::Succeeded => tr!("성공", "Succeeded"),
        Status::Failed => tr!("실패", "Failed"),
        Status::PartialFailure => tr!("일부 실패", "Partially failed"),
        Status::Canceled => tr!("취소", "Canceled"),
    }
}

pub fn validation_label(message: &str) -> &str {
    match message {
        "Provide 1 to 32 steps." => tr!("단계를 1~32개 추가하세요.", "Provide 1 to 32 steps."),
        "Use an HTTP or HTTPS URL without user credentials." => tr!(
            "로그인 정보가 없는 HTTP 또는 HTTPS 주소를 입력하세요.",
            "Use an HTTP or HTTPS URL without user credentials."
        ),
        "Use an Android package name and an optional package/activity component." => tr!(
            "올바른 Android 패키지 이름과 선택적으로 패키지/액티비티를 입력하세요.",
            "Use an Android package name and an optional package/activity component."
        ),
        "Total wait time may not exceed 60000 ms." => tr!(
            "대기 시간 합계는 60초까지 설정할 수 있습니다.",
            "Total wait time may not exceed 60000 ms."
        ),
        _ => message,
    }
}

fn action_label(step: &Step) -> &'static str {
    match step {
        Step::OpenUrl { .. } => tr!("사이트 열기", "Open website"),
        Step::LaunchApp { .. } => tr!("앱 실행", "Launch app"),
        Step::Tap { .. } => tr!("탭", "Tap"),
        Step::Swipe { .. } => tr!("스와이프", "Swipe"),
        Step::Key { .. } => tr!("키 입력", "Key press"),
        Step::Wait { .. } => tr!("대기", "Wait"),
    }
}

fn key_label(key: Key) -> &'static str {
    match key {
        Key::Home => tr!("홈", "Home"),
        Key::Back => tr!("뒤로", "Back"),
        Key::Recents => tr!("최근 앱", "Recents"),
        Key::Enter => tr!("엔터", "Enter"),
    }
}

fn step_summary(step: &Step) -> String {
    let detail = match step {
        Step::OpenUrl { url } => url.clone(),
        Step::LaunchApp { package, component } => component.as_ref().unwrap_or(package).clone(),
        Step::Tap { x, y } => format!("({x:.2}, {y:.2})"),
        Step::Swipe { x1, y1, x2, y2, .. } => format!("({x1:.2}, {y1:.2}) → ({x2:.2}, {y2:.2})"),
        Step::Key { key } => key_label(*key).to_owned(),
        Step::Wait { ms } => format!("{ms} ms"),
    };
    format!("{} · {detail}", action_label(step))
}

fn edit(ui: &mut Ui, draft: &mut Preset, editing: bool) -> (bool, bool) {
    ui.label(
        RichText::new(if editing {
            tr!("프리셋 편집", "Edit preset")
        } else {
            tr!("새 프리셋", "New preset")
        })
        .size(21.)
        .strong(),
    );
    ui.label(tr!("프리셋 이름", "Preset name"));
    ui.add(
        egui::TextEdit::singleline(&mut draft.name)
            .desired_width(ui.available_width())
            .margin(egui::Vec2::new(11., 9.))
            .char_limit(80),
    );
    ui.label(RichText::new(tr!("실행 순서", "Steps")).strong());
    if draft.steps.is_empty() {
        card().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(
                RichText::new(tr!(
                    "아래에서 첫 동작을 추가하세요.",
                    "Add the first action below."
                ))
                .color(MUTED),
            );
        });
    }
    egui::ScrollArea::vertical()
        .id_salt("preset-editor")
        .max_height(340.)
        .show(ui, |ui| {
            let count = draft.steps.len();
            let mut swap = None;
            let mut remove = None;
            for (index, step) in draft.steps.iter_mut().enumerate() {
                ui.push_id(index, |ui| {
                    card().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.strong(format!("{:02}  {}", index + 1, action_label(step)));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.add(control(tr!("삭제", "Remove"))).clicked() {
                                        remove = Some(index);
                                    }
                                    if ui
                                        .add_enabled(index + 1 < count, control("↓"))
                                        .on_hover_text(tr!("아래로 이동", "Move down"))
                                        .clicked()
                                    {
                                        swap = Some((index, index + 1));
                                    }
                                    if ui
                                        .add_enabled(index > 0, control("↑"))
                                        .on_hover_text(tr!("위로 이동", "Move up"))
                                        .clicked()
                                    {
                                        swap = Some((index, index - 1));
                                    }
                                },
                            );
                        });
                        step_fields(ui, step);
                    });
                });
            }
            if let Some((from, to)) = swap {
                draft.steps.swap(from, to);
            }
            if let Some(index) = remove {
                draft.steps.remove(index);
            }
        });

    ui.add_enabled_ui(draft.steps.len() < api::MAX_STEPS, |ui| {
        ui.menu_button(tr!("＋  단계 추가", "＋  Add step"), |ui| {
            for step in [
                Step::OpenUrl { url: String::new() },
                Step::LaunchApp {
                    package: String::new(),
                    component: None,
                },
                Step::Tap { x: 0.5, y: 0.5 },
                Step::Swipe {
                    x1: 0.5,
                    y1: 0.75,
                    x2: 0.5,
                    y2: 0.25,
                    duration_ms: 450,
                },
                Step::Key { key: Key::Home },
                Step::Wait { ms: 1000 },
            ] {
                if ui
                    .add(control(action_label(&step)).min_size(Vec2::new(136., 30.)))
                    .clicked()
                {
                    draft.steps.push(step);
                    ui.close_menu();
                    break;
                }
            }
        });
    });
    ui.separator();
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let save = ui
            .add(filled(tr!("저장", "Save")).min_size(Vec2::new(88., 36.)))
            .clicked();
        let cancel = ui
            .add(control(tr!("취소", "Cancel")).min_size(Vec2::new(68., 36.)))
            .clicked();
        (save, cancel)
    })
    .inner
}

fn step_fields(ui: &mut Ui, step: &mut Step) {
    match step {
        Step::OpenUrl { url } => {
            ui.add(
                egui::TextEdit::singleline(url)
                    .desired_width(ui.available_width())
                    .margin(egui::Vec2::new(10., 7.))
                    .hint_text("https://example.com/")
                    .char_limit(2048),
            );
        }
        Step::LaunchApp { package, component } => {
            ui.add(
                egui::TextEdit::singleline(package)
                    .desired_width(ui.available_width())
                    .margin(egui::Vec2::new(10., 7.))
                    .hint_text(tr!(
                        "패키지 이름 · com.example.app",
                        "Package name · com.example.app"
                    ))
                    .char_limit(200),
            );
            let mut value = component.clone().unwrap_or_default();
            if ui
                .add(
                    egui::TextEdit::singleline(&mut value)
                        .desired_width(ui.available_width())
                        .margin(egui::Vec2::new(10., 7.))
                        .hint_text(tr!(
                            "선택 사항 · com.example.app/.MainActivity",
                            "Optional · com.example.app/.MainActivity"
                        ))
                        .char_limit(441),
                )
                .changed()
            {
                *component = (!value.trim().is_empty()).then_some(value);
            }
        }
        Step::Tap { x, y } => {
            ui.horizontal(|ui| {
                coordinate(ui, "X", x);
                coordinate(ui, "Y", y);
                ui.label(tr!(
                    "화면 상대 좌표 (0~1)",
                    "Relative screen coordinates (0–1)"
                ));
            });
        }
        Step::Swipe {
            x1,
            y1,
            x2,
            y2,
            duration_ms,
        } => {
            ui.horizontal_wrapped(|ui| {
                coordinate(ui, "X₁", x1);
                coordinate(ui, "Y₁", y1);
                coordinate(ui, "X₂", x2);
                coordinate(ui, "Y₂", y2);
                ui.add(
                    egui::DragValue::new(duration_ms)
                        .range(50..=3000)
                        .suffix(" ms"),
                );
            });
        }
        Step::Key { key } => {
            egui::ComboBox::from_id_salt("step-key")
                .selected_text(key_label(*key))
                .show_ui(ui, |ui| {
                    for value in [Key::Home, Key::Back, Key::Recents, Key::Enter] {
                        ui.selectable_value(key, value, key_label(value));
                    }
                });
        }
        Step::Wait { ms } => {
            ui.add(egui::DragValue::new(ms).range(1..=15000).suffix(" ms"));
        }
    }
}

fn coordinate(ui: &mut Ui, axis: &str, value: &mut f32) {
    ui.label(axis);
    ui.add(
        egui::DragValue::new(value)
            .range(0. ..=1.)
            .speed(0.005)
            .fixed_decimals(3),
    );
}
