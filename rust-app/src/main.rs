#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod batch;
mod engine;
mod panel;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, File};
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use eframe::egui::{
    self, Color32, Context, Frame, Pos2, RichText, Sense, Stroke, TextureHandle, Ui, Vec2,
};
use engine::Device;
use serde::{Deserialize, Serialize};

const BLACK: Color32 = Color32::from_gray(16);
const CARD: Color32 = Color32::from_gray(27);
const PAPER: Color32 = Color32::from_gray(248);
const INK: Color32 = Color32::from_gray(24);

fn monochrome(light: bool) -> egui::Visuals {
    let mut visuals = if light {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };
    visuals.selection.bg_fill = if light {
        Color32::from_gray(213)
    } else {
        Color32::from_gray(58)
    };
    visuals.selection.stroke = Stroke::new(1.0_f32, if light { INK } else { Color32::WHITE });
    visuals
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Note {
    number: u32,
    name: String,
    tag: String,
    model: String,
    transport: String,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct WindowSettings {
    large_window: u32,
    small_window: u32,
    small_quality: u32,
    small_fps: u32,
}

impl Default for WindowSettings {
    fn default() -> Self {
        Self {
            large_window: 500,
            small_window: 280,
            small_quality: 720,
            small_fps: 30,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Preferences {
    devices: BTreeMap<String, Note>,
    size_mode: String,
    windows: WindowSettings,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            devices: BTreeMap::new(),
            size_mode: "자동".into(),
            windows: WindowSettings::default(),
        }
    }
}

enum Event {
    Inventory(Result<BTreeMap<String, Device>, String>),
    Preview(Device, u64, Option<engine::ScreenPreview>),
    Action(String, String, Result<String, String>),
    BatchDone(usize, usize, String, Option<String>),
}

struct Mirror {
    child: Child,
    controlled: bool,
    closing: bool,
    log: PathBuf,
}

struct Desk {
    root: PathBuf,
    preferences: Preferences,
    devices: BTreeMap<String, Device>,
    selection: BTreeSet<String>,
    anchor: Option<String>,
    mirrors: BTreeMap<String, Mirror>,
    images: HashMap<String, TextureHandle>,
    preview_times: HashMap<String, Instant>,
    screen_sizes: HashMap<String, (engine::ScreenSize, Instant)>,
    pending: HashSet<String>,
    generation: u64,
    inventory_pending: bool,
    next_inventory: Instant,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    status: String,
    previews: bool,
    control: bool,
    audio: bool,
    record: bool,
    quality: usize,
    transport: String,
    tag: String,
    only_selected: bool,
    panel: panel::Panel,
    tag_edit: Option<String>,
    marquee: Option<(Pos2, bool)>,
    batch: batch::Batch,
}

fn install_fonts(ctx: &Context) {
    let system = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let Ok(bytes) = fs::read(system.join("Fonts").join("malgun.ttf")) else {
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    let name = "malgun".to_owned();
    fonts
        .font_data
        .insert(name.clone(), egui::FontData::from_owned(bytes).into());
    for family in fonts.families.values_mut() {
        family.push(name.clone());
    }
    ctx.set_fonts(fonts);
}

impl Desk {
    fn new(root: PathBuf, ctx: &Context) -> Self {
        install_fonts(ctx);
        ctx.set_visuals(monochrome(false));
        let preferences = fs::read(engine::data_dir().join("preferences.json"))
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default();
        let (tx, rx) = mpsc::channel();
        Self {
            root,
            preferences,
            tx,
            rx,
            devices: BTreeMap::new(),
            selection: BTreeSet::new(),
            anchor: None,
            mirrors: BTreeMap::new(),
            images: HashMap::new(),
            preview_times: HashMap::new(),
            screen_sizes: HashMap::new(),
            pending: HashSet::new(),
            generation: 0,
            inventory_pending: false,
            next_inventory: Instant::now(),
            status: "기기를 확인하는 중…".into(),
            previews: true,
            control: true,
            audio: false,
            record: false,
            quality: 1,
            transport: "전체".into(),
            tag: "모든 태그".into(),
            only_selected: false,
            panel: panel::Panel::default(),
            tag_edit: None,
            marquee: None,
            batch: batch::Batch::default(),
        }
    }

    fn save(&mut self) {
        if let Ok(data) = serde_json::to_vec_pretty(&self.preferences) {
            let folder = engine::data_dir();
            let staged = folder.join(format!("preferences-{}.tmp", std::process::id()));
            let result = fs::write(&staged, data)
                .and_then(|_| fs::rename(&staged, folder.join("preferences.json")));
            if let Err(error) = result {
                self.status = format!("설정 저장 실패 · {error}");
            }
        }
    }

    fn label(&self, id: &str) -> String {
        let note = &self.preferences.devices[id];
        let name = if !note.name.is_empty() {
            &note.name
        } else if !note.model.is_empty() {
            &note.model
        } else {
            "Android"
        };
        format!("{:02}  {name}", note.number)
    }

    fn remember_devices(&mut self) {
        let mut number = self
            .preferences
            .devices
            .values()
            .map(|note| note.number)
            .max()
            .unwrap_or(0);
        let mut changed = false;
        for device in self.devices.values() {
            let note = self
                .preferences
                .devices
                .entry(device.id.clone())
                .or_insert_with(|| {
                    number += 1;
                    changed = true;
                    Note {
                        number,
                        ..Note::default()
                    }
                });
            if device.model != "Android" && note.model != device.model {
                note.model = device.model.clone();
                changed = true;
            }
            if note.transport != device.transport {
                note.transport = device.transport.into();
                changed = true;
            }
        }
        if changed {
            self.save();
        }
    }

    fn refresh(&mut self, ctx: &Context) {
        if self.inventory_pending {
            return;
        }
        self.inventory_pending = true;
        self.next_inventory = Instant::now() + Duration::from_secs(3);
        let (root, tx, ctx) = (self.root.clone(), self.tx.clone(), ctx.clone());
        thread::spawn(move || {
            let _ = tx.send(Event::Inventory(engine::discover(&root)));
            ctx.request_repaint();
        });
    }

    fn events(&mut self, ctx: &Context) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Inventory(result) => {
                    self.inventory_pending = false;
                    match result {
                        Ok(devices) => {
                            self.devices = devices;
                            self.remember_devices();
                            self.images
                                .retain(|id, _| self.devices.get(id).is_some_and(Device::ready));
                            if self
                                .panel
                                .device
                                .as_ref()
                                .is_some_and(|id| !self.devices.get(id).is_some_and(Device::ready))
                            {
                                self.panel = panel::Panel::default();
                            }
                            if self.status.starts_with("기기를 확인")
                                || self.status.starts_with("ADB")
                            {
                                self.status = "드래그로 여러 기기를 선택하고, 동시 조작을 열어 한 번에 제어하세요.".into();
                            }
                        }
                        Err(error) => self.status = format!("ADB 연결 확인 필요 · {error}"),
                    }
                }
                Event::Preview(device, generation, image) => {
                    self.pending.remove(&device.id);
                    if generation == self.generation
                        && self.devices.get(&device.id) == Some(&device)
                    {
                        if let Some(preview) = image {
                            self.screen_sizes
                                .insert(device.id.clone(), (preview.size, Instant::now()));
                            let texture = ctx.load_texture(
                                &device.id,
                                preview.image,
                                egui::TextureOptions::LINEAR,
                            );
                            self.images.insert(device.id, texture);
                        }
                    }
                }
                Event::Action(id, label, result) => self.action_result(id, label, result),
                Event::BatchDone(delivered, attempted, label, failure) => {
                    self.batch.busy = false;
                    self.batch.status = format!("{label} · {delivered}/{attempted}대 완료");
                    if let Some(error) = failure {
                        self.batch.status.push_str(&format!(
                            " · {}",
                            error.chars().take(80).collect::<String>()
                        ));
                    }
                    if let Some(leader) = &self.batch.leader {
                        self.preview_times.remove(leader);
                    }
                    self.status = self.batch.status.clone();
                }
            }
        }
        self.mirrors.retain(|_, mirror| {
            if mirror.closing {
                engine::close_window(mirror.child.id());
            }
            match mirror.child.try_wait() {
                Ok(Some(exit)) => {
                    if !exit.success() {
                        let details = fs::read_to_string(&mirror.log).unwrap_or_default();
                        let line = details
                            .lines()
                            .rev()
                            .find(|line| line.contains("ERROR"))
                            .unwrap_or("");
                        self.status = format!("미러링 종료 · {line}");
                    }
                    let _ = fs::remove_file(&mirror.log);
                    false
                }
                _ => true,
            }
        });
    }

    fn filters(&self) -> Vec<String> {
        let mut list: Vec<_> = self
            .preferences
            .devices
            .iter()
            .filter(|(id, note)| {
                let transport_match = match self.transport.as_str() {
                    "전체" => true,
                    "접근 가능" => self.devices.get(*id).is_some_and(Device::ready),
                    filter => note.transport == filter,
                };
                let tag_match = match self.tag.as_str() {
                    "모든 태그" => true,
                    "태그 없음" => note.tag.is_empty(),
                    filter => note.tag == filter,
                };
                transport_match && tag_match
            })
            .collect();
        list.sort_by_key(|(_, note)| note.number);
        list.into_iter().map(|(id, _)| id.clone()).collect()
    }

    fn ready(&self) -> Vec<Device> {
        self.filters()
            .iter()
            .filter(|id| self.selection.contains(*id))
            .filter_map(|id| self.devices.get(id).filter(|device| device.ready()))
            .cloned()
            .collect()
    }

    fn select(&mut self, ctx: &Context, id: &str, additive: bool, range: bool) {
        let ids = self.filters();
        let endpoints = self.anchor.as_ref().and_then(|anchor| {
            Some((
                ids.iter().position(|item| item == anchor)?,
                ids.iter().position(|item| item == id)?,
            ))
        });
        if let (true, Some((start, end))) = (range, endpoints) {
            self.selection = ids[start.min(end)..=start.max(end)]
                .iter()
                .cloned()
                .collect();
        } else if additive {
            if !self.selection.remove(id) {
                self.selection.insert(id.to_owned());
            }
            self.anchor = Some(id.to_owned());
        } else {
            self.selection = BTreeSet::from([id.to_owned()]);
            self.anchor = Some(id.to_owned());
        }
        if self.selection.len() == 1 {
            if let Some(device) = self.ready().first() {
                self.open_panel(ctx, &device.id);
                return;
            }
        }
        self.panel = panel::Panel::default();
    }

    fn queue_preview(&mut self, ctx: &Context, id: &str) {
        let batch_preview = self.batch.leader.as_deref() == Some(id) && self.batch.open;
        if (!self.previews && !batch_preview)
            || self.pending.len() >= 3
            || self.pending.contains(id)
            || self.preview_times.get(id).is_some_and(|time| {
                time.elapsed()
                    < if batch_preview {
                        Duration::from_millis(500)
                    } else {
                        Duration::from_secs(4)
                    }
            })
        {
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
        self.pending.insert(id.to_owned());
        self.preview_times.insert(id.to_owned(), Instant::now());
        let (root, tx, ctx, generation) = (
            self.root.clone(),
            self.tx.clone(),
            ctx.clone(),
            self.generation,
        );
        thread::spawn(move || {
            let image = engine::thumbnail(&root, &device, batch_preview).ok();
            let _ = tx.send(Event::Preview(device, generation, image));
            ctx.request_repaint();
        });
    }

    fn wall(&mut self, ui: &mut Ui) {
        let devices = self.filters();
        let cards: Vec<_> = devices
            .iter()
            .filter(|id| !self.only_selected || self.selection.contains(*id))
            .collect();
        let columns = ((ui.available_width() + 12.) / 166.).floor().max(1.) as usize;
        let width =
            ((ui.available_width() - 12. * (columns - 1) as f32) / columns as f32).min(195.);
        let mut hit_boxes = Vec::new();
        let mut clip = ui.clip_rect();

        egui::ScrollArea::vertical()
            .drag_to_scroll(false)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                clip = ui.clip_rect();
                if cards.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(65.);
                        ui.label(
                            RichText::new("표시할 기기가 없어요")
                                .color(Color32::WHITE)
                                .size(19.),
                        );
                        ui.label(
                            RichText::new("USB 연결이나 오른쪽 기기 필터를 확인하세요.")
                                .color(Color32::from_gray(184)),
                        );
                    });
                }
                ui.spacing_mut().item_spacing = Vec2::splat(12.);
                for row in cards.chunks(columns) {
                    ui.horizontal(|ui| {
                        for &id in row {
                            let (rect, response) =
                                ui.allocate_exact_size(Vec2::new(width, 252.), Sense::click());
                            if !rect.intersects(clip) {
                                continue;
                            }
                            hit_boxes.push((id.clone(), rect.intersect(clip)));
                            self.draw_card(ui, rect, id);
                            self.queue_preview(ui.ctx(), id);
                            if response.clicked() {
                                let mods = ui.input(|input| input.modifiers);
                                self.select(ui.ctx(), id, mods.ctrl, mods.shift);
                            }
                            if response.double_clicked() {
                                self.open_devices(ui.ctx(), true);
                            }
                        }
                    });
                }
            });
        self.update_marquee(ui, clip, &hit_boxes);
    }

    fn draw_card(&self, ui: &Ui, rect: egui::Rect, id: &str) {
        let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
        let selected = self.selection.contains(id);
        painter.rect(
            rect,
            10.,
            if selected {
                Color32::from_gray(42)
            } else {
                CARD
            },
            Stroke::new(
                if selected { 2.0_f32 } else { 1.0_f32 },
                if selected {
                    Color32::WHITE
                } else {
                    Color32::from_gray(58)
                },
            ),
            egui::StrokeKind::Inside,
        );
        let note = &self.preferences.devices[id];
        let text = |offset: Vec2, align, value: &str, size, color| {
            painter.text(
                rect.min + offset,
                align,
                value,
                egui::FontId::proportional(size),
                color,
            );
        };
        text(
            Vec2::new(10., 17.),
            egui::Align2::LEFT_CENTER,
            &format!("{:02}", note.number),
            18.,
            Color32::WHITE,
        );
        text(
            Vec2::new(rect.width() - 10., 17.),
            egui::Align2::RIGHT_CENTER,
            &note.transport,
            12.,
            Color32::from_gray(192),
        );
        let preview =
            egui::Rect::from_min_max(rect.min + Vec2::new(8., 34.), rect.max - Vec2::new(8., 42.));
        painter.rect_filled(preview, 4., BLACK);
        if let Some(texture) = self.images.get(id).filter(|_| self.previews) {
            let size = texture.size_vec2();
            let scale = (preview.width() / size.x).min(preview.height() / size.y);
            painter.image(
                texture.id(),
                egui::Rect::from_center_size(preview.center(), size * scale),
                egui::Rect::from_min_max(Pos2::ZERO, Pos2::new(1., 1.)),
                Color32::WHITE,
            );
        } else {
            let label = match self.devices.get(id).map(|device| device.state.as_str()) {
                Some("device") if self.previews => "화면 불러오는 중",
                Some("device") => "미리보기 꺼짐",
                Some("unauthorized") => "폰에서 승인 필요",
                _ => "연결 끊김",
            };
            painter.text(
                preview.center(),
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(12.),
                Color32::LIGHT_GRAY,
            );
        }
        let name = if !note.name.is_empty() {
            &note.name
        } else if !note.model.is_empty() {
            &note.model
        } else {
            "Android"
        };
        let state = if self.mirrors.contains_key(id) {
            "화면 열림"
        } else if !note.tag.is_empty() {
            &note.tag
        } else if self.devices.get(id).is_some_and(Device::ready) {
            "연결됨"
        } else {
            "오프라인"
        };
        text(
            Vec2::new(10., 225.),
            egui::Align2::LEFT_CENTER,
            &name.chars().take(18).collect::<String>(),
            13.,
            Color32::WHITE,
        );
        text(
            Vec2::new(10., 241.),
            egui::Align2::LEFT_CENTER,
            &state.chars().take(18).collect::<String>(),
            11.,
            Color32::from_gray(192),
        );
    }

    fn update_marquee(&mut self, ui: &Ui, clip: egui::Rect, hit_boxes: &[(String, egui::Rect)]) {
        let (cursor, pressed, released, down, additive) = ui.input(|input| {
            (
                input.pointer.interact_pos(),
                input.pointer.button_pressed(egui::PointerButton::Primary),
                input.pointer.button_released(egui::PointerButton::Primary),
                input.pointer.primary_down(),
                input.modifiers.ctrl,
            )
        });
        if pressed
            && cursor.is_some_and(|pos| clip.contains(pos))
            && !ui.memory(|memory| memory.any_popup_open())
        {
            self.marquee = cursor.map(|pos| (pos, additive));
        }
        if let (Some((origin, additive)), Some(cursor)) = (self.marquee, cursor) {
            let region = egui::Rect::from_two_pos(origin, cursor).intersect(clip);
            if origin.distance(cursor) >= 7. && region.is_positive() {
                let painter = ui.ctx().layer_painter(egui::LayerId::new(
                    egui::Order::Foreground,
                    egui::Id::new("device-marquee"),
                ));
                painter.rect(
                    region,
                    0.,
                    Color32::from_white_alpha(28),
                    Stroke::new(1.0_f32, Color32::WHITE),
                    egui::StrokeKind::Inside,
                );
                for (_, rect) in hit_boxes
                    .iter()
                    .filter(|(_, rect)| region.intersects(*rect))
                {
                    painter.rect_stroke(
                        *rect,
                        8.,
                        Stroke::new(1.0_f32, Color32::WHITE),
                        egui::StrokeKind::Inside,
                    );
                }
                if released {
                    let picked = hit_boxes
                        .iter()
                        .filter(|(_, rect)| region.intersects(*rect))
                        .map(|(id, _)| id.clone());
                    if additive {
                        self.selection.extend(picked);
                    } else {
                        self.selection = picked.collect();
                    }
                    self.panel = panel::Panel::default();
                }
            }
        }
        if released || !down {
            self.marquee = None;
        }
    }

    fn rail(&mut self, ui: &mut Ui) {
        ui.set_min_width(268.);
        ui.set_max_width(268.);
        ui.horizontal(|ui| {
            ui.heading("컨트롤");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("{}대 선택", self.ready().len()))
                        .small()
                        .weak(),
                );
            });
        });
        ui.add_space(10.);
        let ready = self.ready();
        if primary(ui, "선택 화면 열기", !ready.is_empty()) {
            self.open_devices(ui.ctx(), false);
        }
        ui.horizontal(|ui| {
            let active = self
                .selection
                .iter()
                .any(|id| self.mirrors.contains_key(id));
            if ui
                .add_enabled(active, egui::Button::new("화면 닫기"))
                .clicked()
            {
                for id in &self.selection {
                    if let Some(mirror) = self.mirrors.get_mut(id) {
                        mirror.closing = true;
                    }
                }
            }
            if ui
                .add_enabled(ready.len() == 1, egui::Button::new("기기 조작"))
                .clicked()
            {
                self.open_panel(ui.ctx(), &ready[0].id);
            }
            if ui.button("녹화 폴더").clicked() {
                let path = recordings();
                let _ = fs::create_dir_all(&path);
                let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
            }
        });
        let batch_count = self.batch_targets().len();
        if primary(
            ui,
            &format!("동시 조작 · {batch_count}대"),
            batch_count >= 2,
        ) {
            self.open_batch(ui.ctx());
        }
        ui.separator();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.heading("미러링");
                ui.horizontal(|ui| {
                    let previous = self.preferences.size_mode.clone();
                    combo(
                        ui,
                        "창 모드",
                        &mut self.preferences.size_mode,
                        &["자동", "대형", "소형"],
                    );
                    if previous != self.preferences.size_mode {
                        self.save();
                    }
                    egui::ComboBox::from_id_salt("quality")
                        .selected_text(QUALITIES[self.quality].0)
                        .show_ui(ui, |ui| {
                            for (index, options) in QUALITIES.iter().enumerate() {
                                ui.selectable_value(&mut self.quality, index, options.0);
                            }
                        });
                });
                let settings = &mut self.preferences.windows;
                let mut changed = false;
                for (label, value, range, step) in [
                    ("대형 창 너비", &mut settings.large_window, 300..=850, 10.),
                    ("소형 창 너비", &mut settings.small_window, 160..=480, 10.),
                    (
                        "소형 영상 해상도",
                        &mut settings.small_quality,
                        360..=1440,
                        120.,
                    ),
                    ("소형 영상 최대 FPS", &mut settings.small_fps, 10..=60, 5.),
                ] {
                    ui.horizontal(|ui| {
                        ui.add_sized([124., 22.], egui::Label::new(label));
                        changed |= ui
                            .add(egui::Slider::new(value, range).step_by(step))
                            .changed();
                    });
                }
                if changed {
                    self.save();
                }
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut self.previews, "미리보기").changed() {
                        self.generation += 1;
                        self.images.clear();
                        self.preview_times.clear();
                    }
                    ui.checkbox(&mut self.control, "마우스·키보드");
                });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.audio, "기기 소리");
                    ui.checkbox(&mut self.record, "화면 녹화");
                });
                ui.separator();
                ui.heading("기기 및 태그");
                ui.horizontal(|ui| {
                    combo(
                        ui,
                        "연결 필터",
                        &mut self.transport,
                        &["전체", "USB", "Wi-Fi", "접근 가능"],
                    );
                    let tags: BTreeSet<String> = self
                        .preferences
                        .devices
                        .values()
                        .map(|note| note.tag.clone())
                        .filter(|tag| !tag.is_empty())
                        .collect();
                    let options: Vec<&str> = ["모든 태그", "태그 없음"]
                        .into_iter()
                        .chain(tags.iter().map(String::as_str))
                        .collect();
                    combo(ui, "태그 필터", &mut self.tag, &options);
                });
                let ids = self.filters();
                ui.horizontal(|ui| {
                    if ui.button("연결 기기 선택").clicked() {
                        self.selection = ids
                            .iter()
                            .filter(|id| self.devices.get(*id).is_some_and(Device::ready))
                            .cloned()
                            .collect();
                        self.panel = panel::Panel::default();
                    }
                    if ui
                        .add_enabled(!self.selection.is_empty(), egui::Button::new("태그 지정"))
                        .clicked()
                    {
                        self.tag_edit = Some(String::new());
                    }
                });
                ui.checkbox(&mut self.only_selected, "선택한 기기만 표시");
                for row in ids.chunks(6) {
                    ui.horizontal(|ui| {
                        for id in row {
                            let number = self.preferences.devices[id].number;
                            let selected = self.selection.contains(id);
                            if ui
                                .add_sized(
                                    [34., 26.],
                                    egui::Button::new(format!("{number:02}")).selected(selected),
                                )
                                .clicked()
                            {
                                let modifiers = ui.input(|input| input.modifiers);
                                self.select(ui.ctx(), id, modifiers.ctrl, modifiers.shift);
                            }
                        }
                    });
                }
                ui.add_space(6.);
                ui.horizontal(|ui| {
                    let connected: Vec<_> = self
                        .ready()
                        .into_iter()
                        .filter(|device| {
                            self.mirrors
                                .get(&device.id)
                                .is_some_and(|mirror| mirror.controlled)
                        })
                        .collect();
                    for (label, key) in [("홈", "KEYCODE_HOME"), ("뒤로", "KEYCODE_BACK")] {
                        if ui
                            .add_enabled(!connected.is_empty(), egui::Button::new(label))
                            .clicked()
                        {
                            for device in &connected {
                                self.run_action(
                                    ui.ctx(),
                                    device,
                                    label,
                                    &["shell", "input", "keyevent", key],
                                    None,
                                );
                            }
                        }
                    }
                });
            });
    }

    fn open_devices(&mut self, ctx: &Context, focus: bool) {
        let ready = self.ready();
        if focus && ready.len() == 1 {
            if let Some(mirror) = self.mirrors.get(&ready[0].id) {
                engine::focus_window(mirror.child.id());
                return;
            }
        }
        let targets: Vec<_> = ready
            .into_iter()
            .filter(|device| !self.mirrors.contains_key(&device.id))
            .collect();
        if targets.is_empty() {
            return;
        }
        let count = targets.len();
        let settings = &self.preferences.windows;
        let small = self.preferences.size_mode == "소형"
            || (self.preferences.size_mode == "자동" && count > 2);
        let (_, max_size, fps, bitrate) = QUALITIES[self.quality];
        let (width, size, fps, bitrate) = if small {
            (
                settings.small_window,
                settings.small_quality,
                settings.small_fps,
                "4M",
            )
        } else {
            (settings.large_window, max_size, fps, bitrate)
        };
        let screen = ctx
            .input(|input| input.viewport().monitor_size)
            .unwrap_or(Vec2::new(1440., 900.))
            * ctx.pixels_per_point();
        let cols = (count as f32).sqrt().ceil().min(4.) as usize;
        let rows = count.div_ceil(cols);
        let cell_width = (screen.x as usize / cols).max(100);
        let cell_height = ((screen.y as usize).saturating_sub(60) / rows).max(110);
        for (index, device) in targets.into_iter().enumerate() {
            let mut command = engine::scrcpy_command(&self.root, &device);
            command.args([
                format!("--window-title={}", self.label(&device.id)),
                format!("--window-x={}", 10 + (index % cols) * cell_width),
                format!("--window-y={}", 10 + (index / cols) * cell_height),
                format!(
                    "--window-width={}",
                    (width as usize).min(cell_width.saturating_sub(12))
                ),
                format!(
                    "--window-height={}",
                    cell_height.saturating_sub(32).min(820)
                ),
                format!("--max-size={size}"),
                format!("--max-fps={fps}"),
                format!("--video-bit-rate={bitrate}"),
            ]);
            if !self.control {
                command.arg("--no-control");
            }
            if !self.audio {
                command.arg("--no-audio");
            }
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            if self.record {
                let folder = recordings();
                if let Err(error) = fs::create_dir_all(&folder) {
                    self.status = format!("녹화 폴더 오류 · {error}");
                    continue;
                }
                command.arg(format!(
                    "--record={}",
                    folder
                        .join(format!("capture-{stamp}-{index}.mkv"))
                        .display()
                ));
            }
            let log = engine::data_dir()
                .join(format!("mirror-{}-{stamp}-{index}.log", std::process::id()));
            let process = File::create(&log).and_then(|out| {
                command
                    .stderr(Stdio::from(out.try_clone()?))
                    .stdout(Stdio::from(out))
                    .spawn()
            });
            match process {
                Ok(child) => {
                    self.mirrors.insert(
                        device.id,
                        Mirror {
                            child,
                            controlled: self.control,
                            closing: false,
                            log,
                        },
                    );
                }
                Err(error) => self.status = format!("미러링 실행 실패 · {error}"),
            }
        }
    }
}

const QUALITIES: [(&str, u32, u32, &str); 3] = [
    ("720p / 30fps", 720, 30, "3M"),
    ("1080p / 60fps", 1080, 60, "8M"),
    ("1440p / 60fps", 1440, 60, "16M"),
];

fn recordings() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(engine::data_dir)
        .join("Videos")
        .join("Android Device Desk")
}

fn combo(ui: &mut Ui, id: &str, selected: &mut String, options: &[&str]) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(&**selected)
        .show_ui(ui, |ui| {
            for option in options {
                ui.selectable_value(selected, (*option).to_owned(), *option);
            }
        });
}

fn primary(ui: &mut Ui, label: &str, enabled: bool) -> bool {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(label).color(Color32::WHITE).strong())
            .fill(INK)
            .min_size(Vec2::new(ui.available_width(), 32.)),
    )
    .clicked()
}

impl eframe::App for Desk {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.events(ctx);
        if Instant::now() >= self.next_inventory {
            self.refresh(ctx);
        }
        ctx.request_repaint_after(Duration::from_millis(250));
        egui::SidePanel::right("control")
            .resizable(false)
            .frame(Frame::new().fill(PAPER).inner_margin(15))
            .show(ctx, |ui| {
                ui.visuals_mut().clone_from(&monochrome(true));
                self.rail(ui);
            });
        egui::TopBottomPanel::top("heading")
            .frame(Frame::new().fill(BLACK).inner_margin(16))
            .show(ctx, |ui| {
                ui.visuals_mut().clone_from(&monochrome(false));
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("기기 데스크")
                            .size(27.)
                            .strong()
                            .color(Color32::WHITE),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("새로 고침").clicked() {
                            self.refresh(ctx);
                        }
                        ui.label(
                            RichText::new(format!(
                                "{}/{}대 연결  ·  {}개 조작 창",
                                self.devices
                                    .values()
                                    .filter(|device| device.ready())
                                    .count(),
                                self.preferences.devices.len(),
                                self.mirrors.len()
                            ))
                            .color(Color32::from_gray(210)),
                        );
                    });
                });
            });
        egui::TopBottomPanel::bottom("status")
            .frame(Frame::new().fill(BLACK).inner_margin(10))
            .show(ctx, |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(&self.status)
                            .small()
                            .color(Color32::from_gray(192)),
                    )
                    .truncate(),
                );
            });
        egui::CentralPanel::default()
            .frame(Frame::new().fill(BLACK).inner_margin(12))
            .show(ctx, |ui| self.wall(ui));
        if let Some(value) = self.tag_edit.as_mut() {
            let mut save = false;
            let mut cancel = false;
            egui::Window::new("기기 태그")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label("선택한 기기에 지정할 태그");
                    ui.text_edit_singleline(value);
                    ui.horizontal(|ui| {
                        save = ui.button("저장").clicked();
                        cancel = ui.button("취소").clicked();
                    });
                });
            if save {
                for id in &self.selection {
                    if let Some(note) = self.preferences.devices.get_mut(id) {
                        note.tag = value.trim().to_owned();
                    }
                }
                self.save();
            }
            if save || cancel {
                self.tag_edit = None;
            }
        }
        if self
            .panel
            .device
            .as_ref()
            .is_some_and(|id| !self.ready().iter().any(|device| &device.id == id))
        {
            self.panel = panel::Panel::default();
        }
        self.render_panel(ctx);
        self.render_batch(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        for mirror in self.mirrors.values() {
            engine::close_window(mirror.child.id());
        }
        let end = Instant::now() + Duration::from_secs(3);
        while !self.mirrors.is_empty() && Instant::now() < end {
            self.mirrors
                .retain(|_, mirror| !matches!(mirror.child.try_wait(), Ok(Some(_))));
            thread::sleep(Duration::from_millis(70));
        }
    }
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let root = match engine::install_engine() {
        Ok(path) => path,
        Err(error) => {
            rfd::MessageDialog::new()
                .set_title("기기 데스크")
                .set_description(format!("미러링 엔진을 준비할 수 없습니다. {error}"))
                .show();
            return;
        }
    };
    if args.get(1).is_some_and(|arg| arg == "--probe") {
        let _ = probe(&root, args.get(2).map(PathBuf::from));
        return;
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1370., 840.])
            .with_min_inner_size([940., 620.]),
        ..Default::default()
    };
    if let Err(error) = eframe::run_native(
        "기기 데스크",
        options,
        Box::new(move |cc| Ok(Box::new(Desk::new(root, &cc.egui_ctx)))),
    ) {
        rfd::MessageDialog::new()
            .set_title("기기 데스크")
            .set_description(error.to_string())
            .show();
    }
}

fn probe(root: &std::path::Path, output: Option<PathBuf>) -> std::io::Result<()> {
    if let Some(output) = output {
        let result = match engine::discover(root) {
            Ok(devices) => {
                let ready = devices.values().filter(|device| device.ready()).count();
                let secondary = devices.values().filter(|device| device.secondary).count();
                format!(
                    "total={} ready={ready} secondary={secondary}",
                    devices.len()
                )
            }
            Err(error) => format!("error: {error}"),
        };
        fs::write(output, result)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marquee_selects_cards_across_a_row() {
        let ctx = Context::default();
        let mut desk = Desk::new(PathBuf::new(), &ctx);
        desk.preferences = Preferences::default();
        desk.previews = false;
        for number in 1..=5 {
            desk.preferences.devices.insert(
                format!("phone-{number}"),
                Note {
                    number,
                    ..Note::default()
                },
            );
        }
        let origin = Pos2::new(22., 23.);
        let end = Pos2::new(365., 220.);
        let screen = egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(735., 650.));
        let frame = |desk: &mut Desk, events| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(screen),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| desk.wall(ui));
                },
            );
        };
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&mut desk, vec![]);
        frame(
            &mut desk,
            vec![egui::Event::PointerMoved(origin), button(origin, true)],
        );
        frame(&mut desk, vec![egui::Event::PointerMoved(end)]);
        frame(&mut desk, vec![button(end, false)]);
        assert_eq!(
            desk.selection,
            BTreeSet::from(["phone-1".into(), "phone-2".into()])
        );
    }
}
