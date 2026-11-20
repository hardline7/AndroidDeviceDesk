use super::*;

#[derive(Default)]
pub struct Panel {
    pub device: Option<String>,
    app_tab: bool,
    search: String,
    apps: Vec<Launcher>,
    selected: Option<String>,
    status: String,
    editor: Option<(String, String)>,
    download: Option<String>,
    position: Option<Pos2>,
}

struct Launcher {
    name: String,
    target: String,
    details: String,
}

#[derive(Clone, Copy)]
enum Tool {
    Install,
    Capture,
    Send,
    Receive,
    Rename,
    Restart,
}

impl Tool {
    fn label(self) -> &'static str {
        match self {
            Self::Install => tr!("APK 설치", "Install APK"),
            Self::Capture => tr!("화면 캡처", "Screenshot"),
            Self::Send => tr!("파일 보내기", "Send file"),
            Self::Receive => tr!("파일 가져오기", "Get file"),
            Self::Rename => tr!("이름 · 태그", "Name and tag"),
            Self::Restart => tr!("다시 시작", "Restart"),
        }
    }
}

const APP_QUERY: &[&str] = &[
    "shell",
    "cmd",
    "package",
    "query-activities",
    "--brief",
    "--components",
    "-a",
    "android.intent.action.MAIN",
    "-c",
    "android.intent.category.LAUNCHER",
];

impl Desk {
    pub fn run_action(
        &mut self,
        ctx: &Context,
        device: &Device,
        label: &str,
        args: &[&str],
        output_file: Option<PathBuf>,
    ) {
        let (root, tx, ctx, device, label, args) = (
            self.root.clone(),
            self.tx.clone(),
            ctx.clone(),
            device.clone(),
            label.to_owned(),
            args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>(),
        );
        if self.panel.device.as_ref() == Some(&device.id) {
            self.panel.status = trf!("{label} 중…", "{label} in progress…");
        }
        thread::spawn(move || {
            let timeout = if matches!(
                args.first().map(String::as_str),
                Some("install" | "push" | "pull")
            ) {
                120
            } else {
                12
            };
            let result = engine::run(engine::adb_command(&root, &device).args(args), timeout)
                .and_then(|response| {
                    if let Some(file) = output_file {
                        return fs::write(file, engine::png_bytes(&response.stdout)?)
                            .map(|_| String::new())
                            .map_err(|error| error.to_string());
                    }
                    let output = String::from_utf8_lossy(&response.stdout).trim().to_owned();
                    if output.starts_with("Error:") || output.starts_with("Failure") {
                        Err(output)
                    } else {
                        Ok(output)
                    }
                });
            let _ = tx.send(Event::Action(device.id, label, result));
            ctx.request_repaint();
        });
    }

    pub fn action_result(&mut self, id: String, label: String, result: Result<String, String>) {
        if self.panel.device.as_deref() != Some(&id) {
            return;
        }
        match result {
            Ok(output) => {
                self.panel.status = trf!("{label} 완료", "{label} completed");
                if label == tr!("앱 목록 확인", "Load apps") {
                    self.panel.apps = launchers(&output);
                }
            }
            Err(error) => {
                self.panel.status = trf!(
                    "{label} 실패 · {}",
                    "{label} failed · {}",
                    error.chars().take(120).collect::<String>()
                )
            }
        }
    }

    pub fn open_panel(&mut self, ctx: &Context, id: &str) {
        if self.panel.device.as_deref() == Some(id) {
            return;
        }
        let Some(device) = self
            .devices
            .get(id)
            .filter(|device| device.ready())
            .cloned()
        else {
            return;
        };
        self.panel = Panel {
            device: Some(id.to_owned()),
            ..Panel::default()
        };
        self.run_action(
            ctx,
            &device,
            tr!("앱 목록 확인", "Load apps"),
            APP_QUERY,
            None,
        );
    }

    pub fn render_panel(&mut self, ctx: &Context) {
        let Some(device) = self
            .panel
            .device
            .as_ref()
            .and_then(|id| self.devices.get(id))
            .cloned()
        else {
            return;
        };
        let viewport = egui::ViewportId::from_hash_of("device-controls");
        let (main, monitor) =
            ctx.input(|input| (input.viewport().outer_rect, input.viewport().monitor_size));
        let width = 304.;
        let (position, height) = if let Some(rect) = self
            .mirrors
            .get(&device.id)
            .and_then(|mirror| engine::window_rect(mirror.child.id()))
        {
            let scale = ctx.pixels_per_point();
            let screen = monitor.unwrap_or(Vec2::new(1600., 900.));
            let left = rect.left as f32 / scale;
            let right = rect.right as f32 / scale;
            let x = if right + width + 8. <= screen.x {
                right + 8.
            } else {
                left - width - 8.
            };
            let height =
                ((rect.bottom - rect.top) as f32 / scale).clamp(440., (screen.y - 85.).max(440.));
            (
                Some(Pos2::new(
                    x.max(0.),
                    (rect.top as f32 / scale).clamp(0., (screen.y - height - 50.).max(0.)),
                )),
                height,
            )
        } else {
            (
                main.map(|rect| {
                    Pos2::new((rect.right() - 630.).max(0.), (rect.top() + 62.).max(0.))
                }),
                565.,
            )
        };
        let mut builder = egui::ViewportBuilder::default()
            .with_title(tr!("기기 조작", "Device Controls"))
            .with_inner_size([width, height])
            .with_min_inner_size([282., 440.])
            .with_taskbar(false);
        if let Some(position) = position {
            builder = builder.with_position(position);
        }
        if self.panel.position != position {
            if let Some(position) = position {
                ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::OuterPosition(position));
            }
            self.panel.position = position;
        }
        let mut close = false;
        ctx.show_viewport_immediate(viewport, builder, |ctx, class| {
            if class == egui::ViewportClass::Embedded {
                egui::Window::new(self.label(&device.id))
                    .default_width(width)
                    .show(ctx, |ui| self.device_controls(ui, &device));
                return;
            }
            close = ctx.input(|input| input.viewport().close_requested());
            if close {
                return;
            }
            egui::TopBottomPanel::top("device-title")
                .frame(Frame::new().fill(BLACK).inner_margin(12))
                .show(ctx, |ui| {
                    ui.label(
                        RichText::new(self.label(&device.id))
                            .color(Color32::WHITE)
                            .size(19.)
                            .strong(),
                    );
                });
            egui::CentralPanel::default()
                .frame(Frame::new().fill(PAPER).inner_margin(11))
                .show(ctx, |ui| {
                    ui.visuals_mut().clone_from(&monochrome(true));
                    self.device_controls(ui, &device);
                });
        });
        if close {
            self.panel = Panel::default();
        }
    }

    fn device_controls(&mut self, ui: &mut Ui, device: &Device) {
        let controlled = self
            .mirrors
            .get(&device.id)
            .is_none_or(|mirror| mirror.controlled && !mirror.closing);
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 8.) / 2.;
            for (index, text) in [tr!("기기 도구", "Device tools"), tr!("앱 실행", "Apps")]
                .into_iter()
                .enumerate()
            {
                if ui
                    .add_sized(
                        [width, 31.],
                        egui::Button::new(text).selected(self.panel.app_tab == (index == 1)),
                    )
                    .clicked()
                {
                    self.panel.app_tab = index == 1;
                }
            }
        });
        ui.add_space(8.);
        if self.panel.app_tab {
            self.apps_tab(ui, device, controlled);
        } else {
            self.tools_tab(ui, device, controlled);
        }

        ui.separator();
        ui.add(egui::Label::new(RichText::new(&self.panel.status).small().weak()).truncate());
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 16.) / 3.;
            for (label, key) in [
                (tr!("최근 앱", "Recents"), "KEYCODE_APP_SWITCH"),
                (tr!("홈", "Home"), "KEYCODE_HOME"),
                (tr!("뒤로", "Back"), "KEYCODE_BACK"),
            ] {
                let clicked = ui
                    .add_enabled(
                        controlled,
                        egui::Button::new(label).min_size(Vec2::new(width, 28.)),
                    )
                    .clicked();
                if clicked {
                    self.run_action(
                        ui.ctx(),
                        device,
                        label,
                        &["shell", "input", "keyevent", key],
                        None,
                    );
                }
            }
        });
    }

    fn apps_tab(&mut self, ui: &mut Ui, device: &Device, controlled: bool) {
        ui.horizontal(|ui| {
            ui.label(tr!("앱 검색", "Search apps"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(tr!("새로 고침", "Refresh")).clicked() {
                    self.run_action(
                        ui.ctx(),
                        device,
                        tr!("앱 목록 확인", "Load apps"),
                        APP_QUERY,
                        None,
                    );
                }
            });
        });
        ui.add(
            egui::TextEdit::singleline(&mut self.panel.search).desired_width(ui.available_width()),
        );
        let search = self.panel.search.trim().to_lowercase();
        let mut launch = false;
        egui::ScrollArea::vertical()
            .max_height((ui.available_height() - 105.).max(100.))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for launcher in self.panel.apps.iter().filter(|app| {
                    app.name.to_lowercase().contains(&search)
                        || app.details.to_lowercase().contains(&search)
                }) {
                    let selected = self.panel.selected.as_deref() == Some(launcher.target.as_str());
                    let response = ui.add_sized(
                        [ui.available_width(), 26.],
                        egui::Button::new(&launcher.name).selected(selected),
                    );
                    if response.clicked() {
                        self.panel.selected = Some(launcher.target.clone());
                    }
                    launch |= response.double_clicked();
                }
            });
        let selected = self
            .panel
            .selected
            .as_ref()
            .and_then(|target| self.panel.apps.iter().find(|app| &app.target == target));
        ui.add(
            egui::Label::new(
                RichText::new(
                    selected.map_or(tr!("앱을 선택하세요", "Select an app"), |app| {
                        app.details.as_str()
                    }),
                )
                .small()
                .weak(),
            )
            .truncate(),
        );
        launch |= primary(
            ui,
            tr!("선택한 앱 열기", "Open selected app"),
            controlled && selected.is_some(),
        );
        if launch && controlled {
            if let Some(app) = selected {
                let target = app.target.clone();
                let option = if target.contains('/') { "-n" } else { "-a" };
                let quoted = format!("'{target}'");
                self.run_action(
                    ui.ctx(),
                    device,
                    tr!("앱 열기", "Open app"),
                    &["shell", "am", "start", option, &quoted],
                    None,
                );
            }
        }
    }

    fn tools_tab(&mut self, ui: &mut Ui, device: &Device, controlled: bool) {
        let open = self.mirrors.contains_key(&device.id);
        if primary(
            ui,
            if open {
                tr!("미러링 화면으로 이동", "Switch to mirror")
            } else {
                tr!("미러링 화면 열기", "Open mirror")
            },
            true,
        ) {
            self.open_devices(ui.ctx(), true);
        }
        ui.add_space(5.);
        ui.label(
            RichText::new(tr!("파일 및 기기", "Files and device"))
                .small()
                .weak(),
        );
        for actions in [
            [(Tool::Install, controlled), (Tool::Capture, true)],
            [(Tool::Send, controlled), (Tool::Receive, true)],
            [(Tool::Rename, true), (Tool::Restart, controlled)],
        ] {
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 8.) / 2.;
                for (tool, allowed) in actions {
                    if ui
                        .add_enabled(
                            allowed,
                            egui::Button::new(tool.label()).min_size(Vec2::new(width, 29.)),
                        )
                        .clicked()
                    {
                        self.tool_action(ui.ctx(), device, tool);
                    }
                }
            });
        }
        ui.add_enabled_ui(controlled, |ui| {
            ui.menu_button(tr!("화면 방향", "Orientation"), |ui| {
                for (label, args) in [
                    (tr!("세로로 고정", "Lock portrait"), &["lock", "0"][..]),
                    (tr!("가로로 고정", "Lock landscape"), &["lock", "1"][..]),
                    (tr!("기기 자동 회전", "Auto-rotate"), &["free"][..]),
                ] {
                    if ui.button(label).clicked() {
                        let mut command = vec!["shell", "cmd", "window", "user-rotation"];
                        command.extend_from_slice(args);
                        self.run_action(
                            ui.ctx(),
                            device,
                            tr!("화면 방향 변경", "Set orientation"),
                            &command,
                            None,
                        );
                        ui.close_menu();
                    }
                }
            });
        });
        self.edit_tools(ui, device);
    }

    fn tool_action(&mut self, ctx: &Context, device: &Device, tool: Tool) {
        let label = tool.label();
        match tool {
            Tool::Install => {
                if let Some(file) = rfd::FileDialog::new()
                    .add_filter("Android APK", &["apk"])
                    .pick_file()
                {
                    self.run_action(
                        ctx,
                        device,
                        label,
                        &["install", "-r", &file.to_string_lossy()],
                        None,
                    );
                }
            }
            Tool::Capture => {
                if let Some(file) = rfd::FileDialog::new()
                    .add_filter("PNG", &["png"])
                    .set_file_name("android.png")
                    .save_file()
                {
                    self.run_action(
                        ctx,
                        device,
                        label,
                        &["exec-out", "screencap", "-p"],
                        Some(file),
                    );
                }
            }
            Tool::Send => {
                if let Some(file) = rfd::FileDialog::new().pick_file() {
                    self.run_action(
                        ctx,
                        device,
                        label,
                        &["push", &file.to_string_lossy(), "/sdcard/Download/"],
                        None,
                    );
                }
            }
            Tool::Receive => self.panel.download = Some(String::new()),
            Tool::Rename => {
                let note = &self.preferences.devices[&device.id];
                self.panel.editor = Some((note.name.clone(), note.tag.clone()));
            }
            Tool::Restart => {
                if rfd::MessageDialog::new()
                    .set_title(tr!("기기 다시 시작", "Restart device"))
                    .set_description(trf!(
                        "{} 기기를 다시 시작할까요?",
                        "Restart {}?",
                        self.label(&device.id)
                    ))
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .show()
                    == rfd::MessageDialogResult::Yes
                {
                    self.run_action(ctx, device, label, &["reboot"], None);
                }
            }
        }
    }

    fn edit_tools(&mut self, ui: &mut Ui, device: &Device) {
        if let Some((name, tag)) = self.panel.editor.as_mut() {
            let mut submit = false;
            let mut cancel = false;
            egui::Window::new(tr!("이름 · 태그", "Name and tag"))
                .collapsible(false)
                .show(ui.ctx(), |ui| {
                    ui.label(tr!("이름", "Name"));
                    ui.text_edit_singleline(name);
                    ui.label(tr!("태그", "Tag"));
                    ui.text_edit_singleline(tag);
                    submit = ui.button(tr!("저장", "Save")).clicked();
                    cancel = ui.button(tr!("취소", "Cancel")).clicked();
                });
            if submit {
                if let Some(note) = self.preferences.devices.get_mut(&device.id) {
                    note.name = name.trim().to_owned();
                    note.tag = tag.trim().to_owned();
                }
                self.save();
            }
            if submit || cancel {
                self.panel.editor = None;
            }
        }
        if let Some(name) = self.panel.download.as_mut() {
            let mut submit = false;
            let mut cancel = false;
            egui::Window::new(tr!("파일 가져오기", "Get file"))
                .collapsible(false)
                .show(ui.ctx(), |ui| {
                    ui.label(tr!(
                        "폰의 다운로드 폴더에 있는 파일 이름",
                        "File name in the device’s Download folder"
                    ));
                    ui.text_edit_singleline(name);
                    let valid = !name.trim().is_empty()
                        && !name.contains(['/', '\\', ':'])
                        && !matches!(name.trim(), "." | "..");
                    submit = ui
                        .add_enabled(
                            valid,
                            egui::Button::new(tr!("저장 위치 선택", "Choose save location")),
                        )
                        .clicked();
                    cancel = ui.button(tr!("취소", "Cancel")).clicked();
                });
            if submit {
                let name = name.trim().to_owned();
                if let Some(file) = rfd::FileDialog::new().set_file_name(&name).save_file() {
                    self.run_action(
                        ui.ctx(),
                        device,
                        tr!("파일 가져오기", "Get file"),
                        &[
                            "pull",
                            &format!("/sdcard/Download/{name}"),
                            &file.to_string_lossy(),
                        ],
                        None,
                    );
                }
            }
            if submit || cancel {
                self.panel.download = None;
            }
        }
    }
}

fn launchers(output: &str) -> Vec<Launcher> {
    let mut list = vec![];
    for target in output
        .lines()
        .map(str::trim)
        .filter(|line| {
            line.split_once('/').is_some_and(|(package, class)| {
                !package.is_empty()
                    && !class.is_empty()
                    && package != "com.android.settings"
                    && package
                        .bytes()
                        .chain(class.bytes())
                        .all(|byte| byte.is_ascii_alphanumeric() || b"_.$".contains(&byte))
            })
        })
        .collect::<BTreeSet<_>>()
    {
        let package = target.split('/').next().unwrap_or_default();
        let name = match package {
            "com.android.chrome" => "Chrome",
            "com.android.vending" => tr!("Play 스토어", "Play Store"),
            "com.facebook.katana" => "Facebook",
            "com.google.android.apps.bard" => "Gemini",
            "com.google.android.apps.docs" => tr!("Google 드라이브", "Google Drive"),
            "com.google.android.apps.maps" => tr!("Google 지도", "Google Maps"),
            "com.google.android.apps.messaging" => tr!("Google 메시지", "Google Messages"),
            "com.google.android.apps.photos" => tr!("Google 포토", "Google Photos"),
            "com.google.android.apps.tachyon" => "Google Meet",
            "com.google.android.apps.youtube.music" => "YouTube Music",
            "com.google.android.gm" => "Gmail",
            "com.google.android.googlequicksearchbox" => "Google",
            "com.google.android.youtube" => "YouTube",
            "com.linkedin.android" => "LinkedIn",
            "com.microsoft.office.outlook" => "Outlook",
            "com.netflix.mediaclient" => "Netflix",
            "com.samsung.android.app.notes" => tr!("삼성 노트", "Samsung Notes"),
            "com.samsung.android.app.contacts" => tr!("연락처", "Contacts"),
            "com.samsung.android.dialer" => tr!("전화", "Phone"),
            "com.samsung.android.gallery3d" => tr!("갤러리", "Gallery"),
            "com.sec.android.app.camera" => tr!("카메라", "Camera"),
            "com.sec.android.app.myfiles" => tr!("내 파일", "My Files"),
            "com.sec.android.app.sbrowser" => tr!("삼성 인터넷", "Samsung Internet"),
            _ => package.rsplit('.').next().unwrap_or(package),
        };
        list.push(Launcher {
            name: name.into(),
            target: target.into(),
            details: package.into(),
        });
    }
    list.sort_by_key(|app| app.name.to_lowercase());
    let mut settings: Vec<_> = [
        (tr!("설정", "Settings"), "SETTINGS"),
        (tr!("Wi-Fi 설정", "Wi-Fi settings"), "WIFI_SETTINGS"),
        (
            tr!("언어 및 입력", "Language and input"),
            "INPUT_METHOD_SETTINGS",
        ),
        (
            tr!("접근성 설정", "Accessibility settings"),
            "ACCESSIBILITY_SETTINGS",
        ),
    ]
    .into_iter()
    .map(|(name, action)| Launcher {
        name: name.into(),
        target: format!("android.settings.{action}"),
        details: tr!("시스템 설정", "System settings").into(),
    })
    .collect();
    settings.append(&mut list);
    settings
}
