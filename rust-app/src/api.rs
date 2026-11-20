use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use serde_json::{json, Value};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::desk_ui::{self, card, control, filled, Tool};
use crate::presets::{Catalog, Preset};

use super::*;

const PORT: u16 = 17653;
const MAX_BODY: usize = 32 * 1024;
pub const MAX_STEPS: usize = 32;
const HISTORY: usize = 32;
const URL: &str = "http://127.0.0.1:17653";

pub struct Api {
    pub core: Arc<Core>,
    server: Option<Running>,
    message: String,
}

struct Running {
    server: Arc<Server>,
    stopped: Arc<AtomicBool>,
    token: String,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        self.server.unblock();
    }
}

pub struct Core {
    root: PathBuf,
    devices: RwLock<BTreeMap<u32, Endpoint>>,
    jobs: Mutex<Jobs>,
    presets: RwLock<Catalog>,
    exiting: AtomicBool,
}

#[derive(Clone)]
pub struct Endpoint {
    pub number: u32,
    pub tag: String,
    name: String,
    device: Device,
    controllable: bool,
}

impl Endpoint {
    pub fn can_run(&self) -> bool {
        self.device.ready() && self.controllable
    }

    pub fn label(&self) -> &str {
        &self.name
    }

    fn public(&self) -> Value {
        json!({
            "number": self.number, "name": self.name, "model": self.device.model,
            "transport": self.device.transport, "state": self.device.state, "tag": self.tag,
            "controllable": self.can_run(),
        })
    }
}

#[derive(Default)]
struct Jobs {
    next: u64,
    busy: bool,
    history: BTreeMap<u64, Job>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    targets: Targets,
    steps: Vec<Step>,
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
pub enum Targets {
    Tagged(TagTarget),
    Group(String),
    One(u32),
    Many(Vec<u32>),
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TagTarget {
    pub tag: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    OpenUrl {
        url: String,
    },
    LaunchApp {
        package: String,
        component: Option<String>,
    },
    Tap {
        x: f32,
        y: f32,
    },
    Swipe {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        duration_ms: u32,
    },
    Key {
        key: Key,
    },
    Wait {
        ms: u32,
    },
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Key {
    Home,
    Back,
    Recents,
    Enter,
}

impl Key {
    fn android(&self) -> &'static str {
        match self {
            Self::Home => "KEYCODE_HOME",
            Self::Back => "KEYCODE_BACK",
            Self::Recents => "KEYCODE_APP_SWITCH",
            Self::Enter => "KEYCODE_ENTER",
        }
    }
}

impl Plan {
    fn validate(&self) -> Result<(), &'static str> {
        validate_steps(&self.steps)
    }
}

pub fn validate_steps(steps: &[Step]) -> Result<(), &'static str> {
    if steps.is_empty() || steps.len() > MAX_STEPS {
        return Err("Provide 1 to 32 steps.");
    }
    let mut wait = 0;
    for step in steps {
        match step {
            Step::OpenUrl { url } if parse_url(url).is_none() => {
                return Err("Use an HTTP or HTTPS URL without user credentials.");
            }
            Step::LaunchApp { package, component } => {
                if !valid_package(package)
                    || component.as_ref().is_some_and(|value| {
                        value.split_once('/').is_none_or(|(owner, activity)| {
                            owner != package || !valid_class(activity)
                        })
                    })
                {
                    return Err(
                        "Use an Android package name and an optional package/activity component.",
                    );
                }
            }
            Step::Tap { x, y } if !unit(*x) || !unit(*y) => {
                return Err("Tap coordinates must be between 0 and 1.")
            }
            Step::Swipe {
                x1,
                y1,
                x2,
                y2,
                duration_ms,
            } if ![*x1, *y1, *x2, *y2].into_iter().all(unit)
                || !(50..=3000).contains(duration_ms) =>
            {
                return Err("Swipe coordinates must be between 0 and 1 and duration_ms between 50 and 3000.");
            }
            Step::Wait { ms } => {
                if !(1..=15000).contains(ms) {
                    return Err("Wait ms must be between 1 and 15000.");
                }
                wait += ms;
            }
            _ => {}
        }
    }
    if wait > 60000 {
        return Err("Total wait time may not exceed 60000 ms.");
    }
    Ok(())
}

fn parse_url(value: &str) -> Option<url::Url> {
    let url = url::Url::parse(value).ok()?;
    (matches!(url.scheme(), "http" | "https")
        && url.host().is_some()
        && url.username().is_empty()
        && url.password().is_none())
    .then_some(url)
}

fn unit(point: f32) -> bool {
    (0. ..=1.).contains(&point)
}

fn valid_package(name: &str) -> bool {
    name.len() <= 200
        && name.contains('.')
        && name.split('.').all(|part| {
            part.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
}

fn valid_class(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 240
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.$".contains(&byte))
}

impl Desk {
    pub fn stop_api(&mut self) {
        self.api.server = None;
        self.api.core.exiting.store(true, Ordering::Release);
    }

    pub fn api_devices(&self) -> BTreeMap<u32, Endpoint> {
        self.devices
            .values()
            .filter_map(|device| {
                let note = self.preferences.devices.get(&device.id)?;
                let number = note.number;
                Some((
                    number,
                    Endpoint {
                        number,
                        tag: note.tag.clone(),
                        name: self.label(&device.id),
                        device: device.clone(),
                        controllable: self.can_control(&device.id),
                    },
                ))
            })
            .collect()
    }

    pub fn update_api(&self) {
        *self.api.core.devices.write().unwrap() = self.api_devices();
    }

    pub fn api_button(&mut self, ui: &mut Ui) {
        let title = if self.api.server.is_some() {
            tr!("로컬 API · 실행 중", "Local API · On")
        } else {
            tr!("로컬 API", "Local API")
        };
        if desk_ui::tool_button(ui, title) {
            self.tool = Some(Tool::Api);
        }
    }

    pub fn api_window(&mut self, ctx: &Context) {
        if self.tool != Some(Tool::Api) {
            return;
        }
        let closed = desk_ui::window(
            ctx,
            Tool::Api,
            tr!("로컬 API", "Local API"),
            [530., 312.],
            |ui| {
                let running = self.api.server.is_some();
                card().show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    desk_ui::heading(
                        ui,
                        if running {
                            tr!("서버 실행 중", "Server running")
                        } else {
                            tr!("서버 꺼짐", "Server off")
                        },
                    );
                    desk_ui::note(
                        ui,
                        tr!(
                            "이 PC의 스크립트에서 기기를 선택하고 작업을 실행할 수 있어요.",
                            "Run device actions from scripts on this PC."
                        ),
                    );
                    if running {
                        ui.separator();
                        ui.label(RichText::new(URL).monospace().size(17.));
                    }
                    ui.add_space(3.);
                    ui.horizontal_wrapped(|ui| {
                        if let Some(api) = &self.api.server {
                            if ui
                                .add(filled(tr!("API 토큰 복사", "Copy API token")))
                                .clicked()
                            {
                                ctx.copy_text(api.token.clone());
                                self.api.message =
                                    tr!("API 토큰을 복사했습니다.", "API token copied.").into();
                            }
                            if ui.add(control(tr!("서버 끄기", "Stop server"))).clicked() {
                                self.api.server = None;
                                self.api.message.clear();
                            }
                        } else if ui.add(filled(tr!("서버 켜기", "Start server"))).clicked() {
                            match Running::listen(self.api.core.clone(), PORT) {
                                Ok(server) => {
                                    self.api.server = Some(server);
                                    self.api.message.clear();
                                }
                                Err(error) => self.api.message = error,
                            }
                        }
                    });
                });
                if running {
                    card().show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        desk_ui::heading(ui, tr!("엔드포인트", "Endpoints"));
                        for (method, path) in [
                            ("GET", "/v1/devices"),
                            ("GET", "/v1/devices/{number}/apps"),
                            ("POST", "/v1/actions"),
                            ("GET", "/v1/presets"),
                            ("POST", "/v1/presets/{id}/run"),
                            ("GET", "/v1/jobs/{job_id}"),
                            ("POST", "/v1/jobs/{job_id}/retry"),
                        ] {
                            ui.horizontal(|ui| {
                                ui.add_sized(
                                    [38., 20.],
                                    egui::Label::new(
                                        RichText::new(method).monospace().color(desk_ui::MUTED),
                                    ),
                                );
                                ui.add(
                                    egui::Label::new(RichText::new(path).monospace()).truncate(),
                                )
                                .on_hover_text(path);
                            });
                        }
                    });
                }
                desk_ui::message(ui, &self.api.message);
            },
        );
        if closed {
            self.tool = None;
        }
    }
}

impl Api {
    pub fn new(root: PathBuf, presets: Catalog) -> Self {
        Self {
            core: Arc::new(Core::new(root, BTreeMap::new(), presets)),
            server: None,
            message: String::new(),
        }
    }
}

impl Running {
    fn listen(core: Arc<Core>, port: u16) -> Result<Self, String> {
        let token = token()?;
        let server = Arc::new(
            Server::http((std::net::Ipv4Addr::LOCALHOST, port))
                .map_err(|error| error.to_string())?,
        );
        let stopped = Arc::new(AtomicBool::new(false));
        let (listener, canceled, secret) = (server.clone(), stopped.clone(), token.clone());
        thread::spawn(move || {
            while !canceled.load(Ordering::Acquire) {
                match listener.recv_timeout(Duration::from_millis(250)) {
                    Ok(Some(request)) if !canceled.load(Ordering::Acquire) => {
                        handle(request, &core, &secret, &canceled)
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            server,
            stopped,
            token,
        })
    }
}

fn token() -> Result<String, String> {
    use windows_sys::Win32::Security::Cryptography::{
        BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
    };
    let mut bytes = [0_u8; 32];
    let status = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            bytes.as_mut_ptr(),
            bytes.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status < 0 {
        return Err(tr!(
            "API 토큰을 생성하지 못했습니다.",
            "Could not generate an API token."
        )
        .into());
    }
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn authenticated(request: &Request, secret: &str) -> bool {
    request
        .headers()
        .iter()
        .find(|header| header.field.equiv("Authorization"))
        .and_then(|header| header.value.as_str().strip_prefix("Bearer "))
        .is_some_and(|value| {
            value.len() == secret.len()
                && value
                    .bytes()
                    .zip(secret.bytes())
                    .fold(0, |difference, (left, right)| difference | (left ^ right))
                    == 0
        })
}

fn handle(mut request: Request, state: &Arc<Core>, secret: &str, canceled: &Arc<AtomicBool>) {
    let (code, value) = if !authenticated(&request, secret) {
        failure(401, "A valid Bearer token is required.")
    } else {
        route(&mut request, state, canceled)
    };
    let response = Response::from_string(value.to_string())
        .with_status_code(code)
        .with_header(Header::from_bytes("Content-Type", "application/json; charset=utf-8").unwrap())
        .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap());
    let _ = request.respond(response);
}

fn failure(code: u16, error: impl Into<String>) -> (u16, Value) {
    (code, json!({ "error": error.into() }))
}

fn route(request: &mut Request, state: &Arc<Core>, canceled: &Arc<AtomicBool>) -> (u16, Value) {
    match (request.method(), request.url()) {
        (&Method::Get, "/v1/devices") => {
            let devices: Vec<_> = state
                .devices
                .read()
                .unwrap()
                .values()
                .map(Endpoint::public)
                .collect();
            (200, json!({ "devices": devices }))
        }
        (&Method::Get, "/v1/presets") => {
            let presets = &state.presets.read().unwrap().items;
            let items: Vec<_> = presets.iter().map(|(id, preset)| {
                json!({ "id": id, "name": preset.name, "step_count": preset.steps.len() })
            }).collect();
            (200, json!({ "presets": items }))
        }
        (&Method::Post, "/v1/actions") => {
            let plan: Plan = match read_json(request) {
                Ok(plan) => plan,
                Err(error) => return error,
            };
            if let Err(error) = plan.validate() {
                return failure(400, error);
            }
            let targets = match get_targets(&state.devices.read().unwrap(), plan.targets) {
                Ok(targets) => targets,
                Err(error) => return error,
            };
            accepted(state.start_job(None, targets, plan.steps.into(), Some(canceled.clone())))
        }
        (&Method::Post, path) => {
            if let Some(id) = route_id(path, "/v1/presets/", "/run") {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct RunPreset {
                    targets: Targets,
                }
                let run: RunPreset = match read_json(request) {
                    Ok(run) => run,
                    Err(error) => return error,
                };
                let preset = state.presets.read().unwrap().items.get(&id).cloned();
                let Some(preset) = preset else {
                    return failure(404, "Preset not found.");
                };
                let targets = match get_targets(&state.devices.read().unwrap(), run.targets) {
                    Ok(targets) => targets,
                    Err(error) => return error,
                };
                return accepted(state.run_preset(preset, targets, Some(canceled.clone())));
            }
            if let Some(id) = route_id(path, "/v1/jobs/", "/retry") {
                return accepted(state.retry_job(id, Some(canceled.clone())));
            }
            failure(404, "Unknown endpoint.")
        }
        (&Method::Get, path) => {
            if let Some(id) = path
                .strip_prefix("/v1/jobs/")
                .and_then(|id| id.parse::<u64>().ok())
            {
                return match state.jobs.lock().unwrap().history.get(&id) {
                    Some(job) => (200, job.public()),
                    None => failure(404, "Job not found."),
                };
            }
            if let Some(number) = path
                .strip_prefix("/v1/devices/")
                .and_then(|tail| tail.strip_suffix("/apps"))
                .and_then(|id| id.parse::<u32>().ok())
            {
                return apps(state, number);
            }
            failure(404, "Unknown endpoint.")
        }
        _ => failure(404, "Unknown endpoint."),
    }
}

fn route_id(path: &str, prefix: &str, suffix: &str) -> Option<u64> {
    path.strip_prefix(prefix)?
        .strip_suffix(suffix)?
        .parse()
        .ok()
}

fn apps(state: &Core, number: u32) -> (u16, Value) {
    let device = state.devices.read().unwrap().get(&number).cloned();
    let Some(endpoint) = device.filter(|entry| entry.device.ready()) else {
        return failure(404, "Connected device not found.");
    };
    match engine::run(
        engine::adb_command(&state.root, &endpoint.device).args(crate::panel::APP_QUERY),
        8,
    ) {
        Ok(result) => {
            let listed: BTreeSet<_> = String::from_utf8_lossy(&result.stdout)
                .lines()
                .filter_map(|line| {
                    let component = line.trim();
                    let (package, class) = component.split_once('/')?;
                    (valid_package(package) && valid_class(class))
                        .then(|| (package.to_owned(), component.to_owned()))
                })
                .collect();
            let apps: Vec<_> = listed
                .into_iter()
                .map(|(package, component)| json!({ "package": package, "component": component }))
                .collect();
            (200, json!({ "number": number, "apps": apps }))
        }
        Err(error) => failure(502, error),
    }
}

fn get_targets(
    devices: &BTreeMap<u32, Endpoint>,
    requested: Targets,
) -> Result<Vec<Endpoint>, (u16, Value)> {
    let numbers: BTreeSet<u32> = match requested {
        Targets::Tagged(target) if !target.tag.trim().is_empty() => devices
            .values()
            .filter(|device| device.can_run() && device.tag == target.tag)
            .map(|device| device.number)
            .collect(),
        Targets::Tagged(_) => return Err(failure(400, "Provide a nonempty device tag.")),
        Targets::Group(name) if name == "all" => devices
            .values()
            .filter(|device| device.can_run())
            .map(|device| device.number)
            .collect(),
        Targets::Group(_) => {
            return Err(failure(
                400,
                "Use \"all\", a device number, an array of numbers, or a tag object.",
            ))
        }
        Targets::One(number) => BTreeSet::from([number]),
        Targets::Many(numbers) => numbers.into_iter().collect(),
    };
    if numbers.is_empty() {
        return Err(failure(409, "No controllable devices were selected."));
    }
    numbers
        .into_iter()
        .map(|number| {
            let device = devices
                .get(&number)
                .ok_or_else(|| failure(404, format!("Device {number} not found.")))?;
            if !device.can_run() {
                return Err(failure(
                    409,
                    format!("Device {number} is offline or view-only."),
                ));
            }
            Ok(device.clone())
        })
        .collect()
}

fn read_json<T: serde::de::DeserializeOwned>(request: &mut Request) -> Result<T, (u16, Value)> {
    let json_type = request
        .headers()
        .iter()
        .find(|item| item.field.equiv("Content-Type"))
        .is_some_and(|item| {
            item.value
                .as_str()
                .split(';')
                .next()
                .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
        });
    if !json_type {
        return Err(failure(415, "Use Content-Type: application/json."));
    }
    let Some(length) = request.body_length() else {
        return Err(failure(411, "Content-Length is required."));
    };
    if length > MAX_BODY {
        return Err(failure(413, "Request body is too large."));
    }
    let mut body = Vec::new();
    if request
        .as_reader()
        .take(length as u64)
        .read_to_end(&mut body)
        .is_err()
        || body.len() != length
    {
        return Err(failure(400, "Could not read the request body."));
    }
    serde_json::from_slice(&body).map_err(|error| failure(400, error.to_string()))
}

#[derive(Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Running,
    Succeeded,
    Failed,
    PartialFailure,
    Canceled,
}

#[derive(Clone, Serialize)]
pub struct DeviceResult {
    pub number: u32,
    pub status: Status,
    pub completed_steps: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed_step: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone)]
pub struct Job {
    pub id: u64,
    pub name: Option<String>,
    pub status: Status,
    targets: Vec<Endpoint>,
    steps: Arc<[Step]>,
    pub results: Vec<DeviceResult>,
}

impl Job {
    pub fn count(&self) -> usize {
        self.targets.len()
    }

    pub fn step_count(&self) -> usize {
        self.steps.len()
    }

    pub fn failed_count(&self) -> usize {
        self.results
            .iter()
            .filter(|result| result.status == Status::Failed)
            .count()
    }

    fn public(&self) -> Value {
        if self.status == Status::Running {
            let targets: Vec<_> = self.targets.iter().map(|target| target.number).collect();
            json!({ "job_id": self.id, "status": self.status, "targets": targets })
        } else {
            json!({ "job_id": self.id, "status": self.status, "devices": self.results })
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum JobError {
    Busy,
    NoTargets,
    JobNotFound,
    NoFailures,
    Stopped,
    InvalidSteps(&'static str),
}

impl JobError {
    pub fn label(self) -> String {
        match self {
            Self::Busy => tr!("다른 작업이 실행 중입니다.", "Another job is running."),
            Self::NoTargets => tr!(
                "제어 가능한 기기가 없습니다.",
                "No controllable devices were selected."
            ),
            Self::JobNotFound => tr!("작업을 찾을 수 없습니다.", "Job not found."),
            Self::NoFailures => tr!(
                "재실행할 실패 기기가 없습니다.",
                "No failed devices to retry."
            ),
            Self::Stopped => tr!(
                "작업 실행을 시작할 수 없습니다.",
                "The job could not be started."
            ),
            Self::InvalidSteps(message) => return crate::presets::validation_label(message).into(),
        }
        .into()
    }
}

fn accepted(result: Result<Job, JobError>) -> (u16, Value) {
    match result {
        Ok(job) => (202, job.public()),
        Err(error) => match error {
            JobError::Busy => failure(409, "Another job is running."),
            JobError::NoTargets => failure(409, "No controllable devices were selected."),
            JobError::JobNotFound => failure(404, "Job not found."),
            JobError::NoFailures => failure(409, "No failed devices to retry."),
            JobError::Stopped => failure(503, "The job could not be started."),
            JobError::InvalidSteps(message) => failure(400, message),
        },
    }
}

impl Core {
    fn new(root: PathBuf, devices: BTreeMap<u32, Endpoint>, presets: Catalog) -> Self {
        Self {
            root,
            devices: RwLock::new(devices),
            jobs: Mutex::new(Jobs::default()),
            presets: RwLock::new(presets),
            exiting: AtomicBool::new(false),
        }
    }

    pub fn set_presets(&self, presets: Catalog) {
        *self.presets.write().unwrap() = presets;
    }

    pub fn history(&self) -> Vec<Job> {
        self.jobs
            .lock()
            .unwrap()
            .history
            .values()
            .rev()
            .cloned()
            .collect()
    }

    pub fn busy(&self) -> bool {
        self.jobs.lock().unwrap().busy
    }

    pub fn run_preset(
        self: &Arc<Self>,
        preset: Preset,
        targets: Vec<Endpoint>,
        canceled: Option<Arc<AtomicBool>>,
    ) -> Result<Job, JobError> {
        validate_steps(&preset.steps).map_err(JobError::InvalidSteps)?;
        self.start_job(Some(preset.name), targets, preset.steps.into(), canceled)
    }

    pub fn retry_job(
        self: &Arc<Self>,
        id: u64,
        canceled: Option<Arc<AtomicBool>>,
    ) -> Result<Job, JobError> {
        let job = self
            .jobs
            .lock()
            .unwrap()
            .history
            .get(&id)
            .cloned()
            .ok_or(JobError::JobNotFound)?;
        if job.status == Status::Running {
            return Err(JobError::Busy);
        }
        let devices = self.devices.read().unwrap();
        let retries: Vec<_> = job
            .targets
            .iter()
            .zip(&job.results)
            .filter(|(_, result)| result.status == Status::Failed)
            .map(|(target, _)| {
                devices
                    .get(&target.number)
                    .filter(|current| current.device.id == target.device.id)
                    .unwrap_or(target)
                    .clone()
            })
            .collect();
        if retries.is_empty() {
            return Err(JobError::NoFailures);
        }
        if !retries.iter().any(|target| is_current(&devices, target)) {
            return Err(JobError::NoTargets);
        }
        drop(devices);
        self.start_job(job.name, retries, job.steps, canceled)
    }

    fn start_job(
        self: &Arc<Self>,
        name: Option<String>,
        targets: Vec<Endpoint>,
        steps: Arc<[Step]>,
        canceled: Option<Arc<AtomicBool>>,
    ) -> Result<Job, JobError> {
        if targets.is_empty() {
            return Err(JobError::NoTargets);
        }
        if self.stopped(canceled.as_deref()) {
            return Err(JobError::Stopped);
        }
        let mut jobs = self.jobs.lock().unwrap();
        if jobs.busy {
            return Err(JobError::Busy);
        }
        jobs.next += 1;
        let job = Job {
            id: jobs.next,
            name,
            status: Status::Running,
            targets,
            steps,
            results: Vec::new(),
        };
        let running = job.clone();
        let state = self.clone();
        // Keep the lock until the worker is registered so even a zero-time job has an entry.
        thread::Builder::new()
            .spawn(move || run_job(&state, running, canceled.as_deref()))
            .map_err(|_| JobError::Stopped)?;
        if jobs.history.len() >= HISTORY {
            jobs.history.pop_first();
        }
        jobs.history.insert(job.id, job.clone());
        jobs.busy = true;
        Ok(job)
    }

    fn stopped(&self, canceled: Option<&AtomicBool>) -> bool {
        self.exiting.load(Ordering::Acquire)
            || canceled.is_some_and(|flag| flag.load(Ordering::Acquire))
    }
}

fn is_current(devices: &BTreeMap<u32, Endpoint>, target: &Endpoint) -> bool {
    devices
        .get(&target.number)
        .is_some_and(|current| current.can_run() && current.device == target.device)
}

fn run_job(state: &Core, mut job: Job, canceled: Option<&AtomicBool>) {
    let mut results = Vec::new();
    for group in job.targets.chunks(6) {
        thread::scope(|scope| {
            let workers: Vec<_> = group
                .iter()
                .map(|target| scope.spawn(|| run_steps(state, target, &job.steps, canceled)))
                .collect();
            for (worker, target) in workers.into_iter().zip(group) {
                results.push(worker.join().unwrap_or_else(|_| DeviceResult {
                    number: target.number,
                    status: Status::Failed,
                    completed_steps: 0,
                    failed_step: None,
                    error: Some("Worker failed.".into()),
                }));
            }
        });
    }
    let done = results
        .iter()
        .filter(|result| result.status == Status::Succeeded)
        .count();
    job.status = if done == results.len() {
        Status::Succeeded
    } else if results
        .iter()
        .any(|result| result.status == Status::Canceled)
    {
        Status::Canceled
    } else if done > 0 {
        Status::PartialFailure
    } else {
        Status::Failed
    };
    job.results = results;
    let mut jobs = state.jobs.lock().unwrap();
    jobs.history.insert(job.id, job);
    jobs.busy = false;
}

fn run_steps(
    state: &Core,
    target: &Endpoint,
    steps: &[Step],
    canceled: Option<&AtomicBool>,
) -> DeviceResult {
    let mut result = DeviceResult {
        number: target.number,
        status: Status::Succeeded,
        completed_steps: 0,
        failed_step: None,
        error: None,
    };
    for (index, step) in steps.iter().enumerate() {
        if state.stopped(canceled) {
            result.status = Status::Canceled;
            return result;
        }
        let step_result = if is_current(&state.devices.read().unwrap(), target) {
            execute(state, &target.device, step, canceled)
        } else {
            Err("Device disconnected or became view-only.".into())
        };
        if let Err(error) = step_result {
            result.status = if state.stopped(canceled) {
                Status::Canceled
            } else {
                Status::Failed
            };
            result.failed_step = Some(index + 1);
            result.error = Some(error.chars().take(600).collect());
            return result;
        }
        result.completed_steps += 1;
    }
    result
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn pixels(x: f32, y: f32, size: engine::ScreenSize) -> [String; 2] {
    [
        (x * size.0.saturating_sub(1) as f32).round().to_string(),
        (y * size.1.saturating_sub(1) as f32).round().to_string(),
    ]
}

fn execute(
    state: &Core,
    device: &Device,
    step: &Step,
    canceled: Option<&AtomicBool>,
) -> Result<(), String> {
    let mut cmd = engine::adb_command(&state.root, device);
    match step {
        Step::OpenUrl { url } => {
            let url = parse_url(url).ok_or("Use an HTTP or HTTPS URL without user credentials.")?;
            cmd.args([
                "shell",
                "am",
                "start",
                "-W",
                "-a",
                "android.intent.action.VIEW",
                "-c",
                "android.intent.category.BROWSABLE",
                "-d",
                &quote(url.as_str()),
            ]);
        }
        Step::LaunchApp { package, component } => {
            cmd.args([
                "shell",
                "am",
                "start",
                "-W",
                "-a",
                "android.intent.action.MAIN",
                "-c",
                "android.intent.category.LAUNCHER",
            ]);
            if let Some(component) = component {
                cmd.args(["-n", &quote(component)]);
            } else {
                cmd.args(["-p", &quote(package)]);
            }
        }
        Step::Tap { x, y } => {
            let size = engine::screen_size(&state.root, device)?;
            cmd.args(["shell", "input", "touchscreen", "tap"])
                .args(pixels(*x, *y, size));
        }
        Step::Swipe {
            x1,
            y1,
            x2,
            y2,
            duration_ms,
        } => {
            let size = engine::screen_size(&state.root, device)?;
            cmd.args(["shell", "input", "touchscreen", "swipe"])
                .args(pixels(*x1, *y1, size))
                .args(pixels(*x2, *y2, size))
                .arg(duration_ms.to_string());
        }
        Step::Key { key } => {
            cmd.args(["shell", "input", "keyevent", key.android()]);
        }
        Step::Wait { ms } => {
            let end = Instant::now() + Duration::from_millis(*ms as u64);
            while Instant::now() < end {
                if state.stopped(canceled) {
                    return Err("Server stopped.".into());
                }
                thread::sleep(
                    end.saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(50)),
                );
            }
            return Ok(());
        }
    }
    if state.stopped(canceled) {
        return Err("Server stopped.".into());
    }
    let result = engine::run(&mut cmd, 12)?;
    for output in [&result.stdout, &result.stderr] {
        let text = String::from_utf8_lossy(output);
        if text.lines().any(|line| {
            line.starts_with("Error")
                || line.starts_with("SecurityException")
                || line.starts_with("Status: timeout")
        }) {
            return Err(text.trim().to_owned());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::{SocketAddr, TcpStream};

    fn fixture(number: u32, controllable: bool) -> (u32, Endpoint) {
        (
            number,
            Endpoint {
                number,
                controllable,
                tag: if number == 2 {
                    "QA".into()
                } else {
                    String::new()
                },
                name: format!("Demo {number}"),
                device: Device {
                    id: format!("demo-{number}"),
                    state: "device".into(),
                    model: "Android".into(),
                    transport: "USB",
                    secondary: number == 2,
                },
            },
        )
    }

    fn request(
        addr: SocketAddr,
        secret: &str,
        verb: &str,
        route: &str,
        body: &str,
    ) -> (u16, Value) {
        let mut socket = TcpStream::connect(addr).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        write!(socket, "{verb} {route} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nAuthorization: Bearer {secret}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        let mut data = String::new();
        socket.read_to_string(&mut data).unwrap();
        let (head, body) = data.split_once("\r\n\r\n").unwrap();
        let code = head.split_whitespace().nth(1).unwrap().parse().unwrap();
        (code, serde_json::from_str(body).unwrap())
    }

    #[test]
    fn local_http_requires_token_and_runs_a_plan_on_eligible_devices() {
        let entries = BTreeMap::from([fixture(2, true), fixture(3, true), fixture(4, false)]);
        let server = Running::listen(
            Arc::new(Core::new(PathBuf::new(), entries, Catalog::default())),
            0,
        )
        .unwrap();
        let addr = server.server.server_addr().to_ip().unwrap();
        assert!(addr.ip().is_loopback());
        assert_eq!(server.token.len(), 64);
        assert_eq!(request(addr, "invalid", "GET", "/v1/devices", "").0, 401);
        let (code, devices) = request(addr, &server.token, "GET", "/v1/devices", "");
        assert_eq!(code, 200);
        assert_eq!(devices["devices"].as_array().unwrap().len(), 3);
        let denied = r#"{"targets":[2,4],"steps":[{"action":"wait","ms":1}]}"#;
        assert_eq!(
            request(addr, &server.token, "POST", "/v1/actions", denied).0,
            409
        );

        let body =
            r#"{"targets":"all","steps":[{"action":"wait","ms":100},{"action":"wait","ms":10}]}"#;
        let (code, accepted) = request(addr, &server.token, "POST", "/v1/actions", body);
        assert_eq!(code, 202);
        assert_eq!(accepted["targets"], json!([2, 3]));
        let path = format!("/v1/jobs/{}", accepted["job_id"]);
        let job = wait_for_job(addr, &server.token, &path);
        assert_eq!(job["status"], "succeeded");
        assert_eq!(job["devices"][0]["completed_steps"], 2);
        assert_eq!(job["devices"][1]["completed_steps"], 2);
    }

    #[test]
    fn preset_jobs_retry_only_original_failed_devices_with_original_steps() {
        let (number, mut disconnected) = fixture(5, true);
        disconnected.tag = "QA".into();
        let mut entries = BTreeMap::from([fixture(2, true), (number, disconnected.clone())]);
        let preset = Preset {
            name: "QA smoke test".into(),
            steps: vec![Step::Wait { ms: 1 }, Step::Wait { ms: 1 }],
        };
        let mut catalog = Catalog::default();
        catalog.items.insert(1, preset.clone());
        let core = Arc::new(Core::new(PathBuf::new(), entries.clone(), catalog));
        let server = Running::listen(core.clone(), 0).unwrap();
        let addr = server.server.server_addr().to_ip().unwrap();

        let send = |verb, path, body| request(addr, &server.token, verb, path, body);
        let list = send("GET", "/v1/presets", "").1;
        assert_eq!(
            list["presets"][0],
            json!({"id": 1, "name": "QA smoke test", "step_count": 2})
        );
        let (code, started) = send("POST", "/v1/presets/1/run", r#"{"targets":{"tag":"QA"}}"#);
        assert_eq!(code, 202);
        assert_eq!(started["targets"], json!([2, 5]));
        let path = format!("/v1/jobs/{}", started["job_id"]);
        assert_eq!(
            wait_for_job(addr, &server.token, &path)["status"],
            "succeeded"
        );

        // Simulate a disconnect between choosing targets and dispatching the job.
        let targets = entries.values().cloned().collect();
        core.devices.write().unwrap().remove(&5);
        let started = core.run_preset(preset.clone(), targets, None).unwrap();
        let path = format!("/v1/jobs/{}", started.id);
        let final_job = wait_for_job(addr, &server.token, &path);
        assert_eq!(final_job["status"], "partial_failure");
        assert_eq!(final_job["devices"][0]["status"], "succeeded");
        assert_eq!(final_job["devices"][1]["status"], "failed");
        assert!(final_job["devices"][1]["failed_step"].as_u64().is_some());

        let retry_path = format!("{path}/retry");
        entries.get_mut(&5).unwrap().device.id = "replacement".into();
        *core.devices.write().unwrap() = entries.clone();
        assert_eq!(send("POST", &retry_path, "").0, 409);

        entries.insert(5, disconnected);
        *core.devices.write().unwrap() = entries;
        let mut changed = Catalog::default();
        changed.items.insert(
            1,
            Preset {
                steps: vec![Step::Wait { ms: 1 }],
                ..preset
            },
        );
        core.set_presets(changed);
        let (code, retry) = send("POST", &retry_path, "");
        assert_eq!(code, 202);
        assert_eq!(retry["targets"], json!([5]));
        let retry_job = wait_for_job(
            addr,
            &server.token,
            &format!("/v1/jobs/{}", retry["job_id"]),
        );
        assert_eq!(retry_job["status"], "succeeded");
        assert_eq!(retry_job["devices"][0]["completed_steps"], 2);
        assert_eq!(retry_job["devices"].as_array().unwrap().len(), 1);
    }

    fn wait_for_job(addr: SocketAddr, token: &str, path: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let (code, job) = request(addr, token, "GET", path, "");
            assert_eq!(code, 200);
            if job["status"] != "running" {
                return job;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn validates_allowed_actions_and_quotes_android_shell_arguments() {
        let valid = r#"{"targets":2,"steps":[{"action":"open_url","url":"https://example.com/?a=1&b=2"},{"action":"launch_app","package":"com.example.app","component":"com.example.app/.Main$Inner"},{"action":"tap","x":0.5,"y":0.9}]}"#;
        assert!(serde_json::from_str::<Plan>(valid)
            .unwrap()
            .validate()
            .is_ok());
        let invalid_url =
            r#"{"targets":2,"steps":[{"action":"open_url","url":"file:///sdcard/test"}]}"#;
        assert!(serde_json::from_str::<Plan>(invalid_url)
            .unwrap()
            .validate()
            .is_err());
        let invalid_app = r#"{"targets":2,"steps":[{"action":"launch_app","package":"x.y","component":"other.app/.Main"}]}"#;
        assert!(serde_json::from_str::<Plan>(invalid_app)
            .unwrap()
            .validate()
            .is_err());
        let too_far = r#"{"targets":2,"steps":[{"action":"tap","x":1.1,"y":0.5}]}"#;
        assert!(serde_json::from_str::<Plan>(too_far)
            .unwrap()
            .validate()
            .is_err());
        assert_eq!(quote("a'b&c"), "'a'\\''b&c'");
        assert_eq!(pixels(1., 1., (1080, 2640)), ["1079", "2639"]);
    }
}
