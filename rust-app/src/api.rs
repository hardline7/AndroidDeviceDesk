use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use serde_json::{json, Value};
use tiny_http::{Header, Method, Request, Response, Server};

use super::*;

const PORT: u16 = 17653;
const MAX_BODY: usize = 32 * 1024;
const MAX_STEPS: usize = 32;
const HISTORY: usize = 32;
const URL: &str = "http://127.0.0.1:17653";

#[derive(Default)]
pub struct Api {
    open: bool,
    server: Option<Running>,
    message: String,
}

struct Running {
    server: Arc<Server>,
    core: Arc<Core>,
    token: String,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.core.stopped.store(true, Ordering::Release);
        self.server.unblock();
    }
}

struct Core {
    root: PathBuf,
    devices: RwLock<BTreeMap<u32, Endpoint>>,
    jobs: Mutex<Jobs>,
    stopped: AtomicBool,
}

#[derive(Clone)]
struct Endpoint {
    number: u32,
    name: String,
    device: Device,
    controllable: bool,
}

impl Endpoint {
    fn can_run(&self) -> bool {
        self.device.ready() && self.controllable
    }

    fn public(&self) -> Value {
        json!({
            "number": self.number, "name": self.name, "model": self.device.model,
            "transport": self.device.transport, "state": self.device.state,
            "controllable": self.can_run(),
        })
    }
}

#[derive(Default)]
struct Jobs {
    next: u64,
    busy: bool,
    history: BTreeMap<u64, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    targets: Targets,
    steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Targets {
    Group(String),
    One(u32),
    Many(Vec<u32>),
}

#[derive(Clone, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    OpenUrl {
        url: url::Url,
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

#[derive(Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Key {
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
        if self.steps.is_empty() || self.steps.len() > MAX_STEPS {
            return Err("Provide 1 to 32 steps.");
        }
        let mut wait = 0;
        for step in &self.steps {
            match step {
                Step::OpenUrl { url }
                    if !matches!(url.scheme(), "http" | "https")
                        || url.host().is_none()
                        || !url.username().is_empty()
                        || url.password().is_some() =>
                {
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
                        return Err("Use an Android package name and an optional package/activity component.");
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
    }

    fn api_devices(&self) -> BTreeMap<u32, Endpoint> {
        self.devices
            .values()
            .filter_map(|device| {
                let number = self.preferences.devices.get(&device.id)?.number;
                Some((
                    number,
                    Endpoint {
                        number,
                        name: self.label(&device.id),
                        device: device.clone(),
                        controllable: self.can_control(&device.id),
                    },
                ))
            })
            .collect()
    }

    pub fn update_api(&self) {
        if let Some(api) = &self.api.server {
            *api.core.devices.write().unwrap() = self.api_devices();
        }
    }

    pub fn api_button(&mut self, ui: &mut Ui) {
        let title = if self.api.server.is_some() {
            tr!("로컬 API · 실행 중", "Local API · On")
        } else {
            tr!("로컬 API", "Local API")
        };
        if ui
            .add_sized([ui.available_width(), 24.], egui::Button::new(title))
            .clicked()
        {
            self.api.open = true;
        }
    }

    pub fn api_window(&mut self, ctx: &Context) {
        if !self.api.open {
            return;
        }
        let mut open = true;
        egui::Window::new(tr!("로컬 API", "Local API"))
            .open(&mut open)
            .collapsible(false)
            .default_width(360.)
            .show(ctx, |ui| {
                ui.label(tr!(
                    "이 PC의 스크립트에서 기기를 선택하고 동작을 실행할 수 있습니다.",
                    "Use scripts on this PC to select devices and run actions."
                ));
                if self.api.server.is_some() {
                    ui.monospace(URL);
                    ui.horizontal(|ui| {
                        if ui.button(tr!("API 토큰 복사", "Copy API token")).clicked() {
                            if let Some(api) = &self.api.server {
                                ctx.copy_text(api.token.clone());
                            }
                            self.api.message =
                                tr!("API 토큰을 복사했습니다.", "API token copied.").into();
                        }
                        if ui.button(tr!("서버 끄기", "Stop server")).clicked() {
                            self.api.server = None;
                            self.api.message.clear();
                        }
                    });
                    ui.monospace("GET  /v1/devices");
                    ui.monospace("POST /v1/actions");
                    ui.monospace("GET  /v1/jobs/{job_id}");
                } else if ui.button(tr!("서버 켜기", "Start server")).clicked() {
                    match Running::start(self.root.clone(), self.api_devices()) {
                        Ok(server) => {
                            self.api.server = Some(server);
                            self.api.message.clear();
                        }
                        Err(error) => self.api.message = error,
                    }
                }
                if !self.api.message.is_empty() {
                    ui.label(&self.api.message);
                }
            });
        self.api.open = open;
    }
}

impl Running {
    fn start(root: PathBuf, devices: BTreeMap<u32, Endpoint>) -> Result<Self, String> {
        Self::listen(root, devices, PORT)
    }

    fn listen(root: PathBuf, devices: BTreeMap<u32, Endpoint>, port: u16) -> Result<Self, String> {
        let token = token()?;
        let server = Arc::new(
            Server::http((std::net::Ipv4Addr::LOCALHOST, port))
                .map_err(|error| error.to_string())?,
        );
        let core = Arc::new(Core {
            root,
            devices: RwLock::new(devices),
            jobs: Mutex::new(Jobs::default()),
            stopped: AtomicBool::new(false),
        });
        let (listener, state, secret) = (server.clone(), core.clone(), token.clone());
        thread::spawn(move || {
            while !state.stopped.load(Ordering::Acquire) {
                match listener.recv_timeout(Duration::from_millis(250)) {
                    Ok(Some(request)) if !state.stopped.load(Ordering::Acquire) => {
                        handle(request, &state, &secret)
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            server,
            core,
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

fn handle(mut request: Request, state: &Arc<Core>, secret: &str) {
    let (code, value) = if !authenticated(&request, secret) {
        failure(401, "A valid Bearer token is required.")
    } else {
        route(&mut request, state)
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

fn route(request: &mut Request, state: &Arc<Core>) -> (u16, Value) {
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
        (&Method::Post, "/v1/actions") => submit(request, state),
        (&Method::Get, path) => {
            if let Some(id) = path
                .strip_prefix("/v1/jobs/")
                .and_then(|id| id.parse::<u64>().ok())
            {
                return match state.jobs.lock().unwrap().history.get(&id) {
                    Some(job) => (200, job.clone()),
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
        Targets::Group(name) if name == "all" => devices
            .values()
            .filter(|device| device.can_run())
            .map(|device| device.number)
            .collect(),
        Targets::Group(_) => {
            return Err(failure(
                400,
                "Use \"all\", a device number, or an array of numbers.",
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

fn submit(request: &mut Request, state: &Arc<Core>) -> (u16, Value) {
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
        return failure(415, "Use Content-Type: application/json.");
    }
    let Some(length) = request.body_length() else {
        return failure(411, "Content-Length is required.");
    };
    if length > MAX_BODY {
        return failure(413, "Request body is too large.");
    }
    let mut body = Vec::new();
    if request
        .as_reader()
        .take(length as u64)
        .read_to_end(&mut body)
        .is_err()
        || body.len() != length
    {
        return failure(400, "Could not read the request body.");
    }
    let plan = match serde_json::from_slice::<Plan>(&body) {
        Ok(plan) => plan,
        Err(error) => return failure(400, error.to_string()),
    };
    if let Err(error) = plan.validate() {
        return failure(400, error);
    }
    let targets = match get_targets(&state.devices.read().unwrap(), plan.targets) {
        Ok(targets) => targets,
        Err(error) => return error,
    };
    let numbers: Vec<_> = targets.iter().map(|target| target.number).collect();
    let id = {
        let mut jobs = state.jobs.lock().unwrap();
        if jobs.busy {
            return failure(409, "Another job is running.");
        }
        jobs.next += 1;
        let id = jobs.next;
        if jobs.history.len() >= HISTORY {
            jobs.history.pop_first();
        }
        jobs.busy = true;
        jobs.history.insert(
            id,
            json!({ "job_id": id, "status": "running", "targets": numbers }),
        );
        id
    };
    let state = state.clone();
    thread::spawn(move || run_job(&state, id, targets, &plan.steps));
    (
        202,
        json!({ "job_id": id, "status": "running", "targets": numbers }),
    )
}

fn run_job(state: &Core, id: u64, targets: Vec<Endpoint>, steps: &[Step]) {
    let mut results = Vec::new();
    for group in targets.chunks(6) {
        thread::scope(|scope| {
            let workers: Vec<_> = group
                .iter()
                .map(|target| scope.spawn(move || run_steps(state, target, steps)))
                .collect();
            for (worker, target) in workers.into_iter().zip(group) {
                results.push(worker.join().unwrap_or_else(|_| json!({ "number": target.number, "status": "failed", "error": "Worker failed." })));
            }
        });
    }
    let done = results
        .iter()
        .filter(|result| result["status"] == "succeeded")
        .count();
    let status = if done == results.len() {
        "succeeded"
    } else if state.stopped.load(Ordering::Acquire) {
        "canceled"
    } else if done > 0 {
        "partial_failure"
    } else {
        "failed"
    };
    let mut jobs = state.jobs.lock().unwrap();
    jobs.history.insert(
        id,
        json!({ "job_id": id, "status": status, "devices": results }),
    );
    jobs.busy = false;
}

fn run_steps(state: &Core, target: &Endpoint, steps: &[Step]) -> Value {
    for (index, step) in steps.iter().enumerate() {
        if state.stopped.load(Ordering::Acquire) {
            return json!({ "number": target.number, "status": "canceled", "completed_steps": index });
        }
        let still_connected = state
            .devices
            .read()
            .unwrap()
            .get(&target.number)
            .is_some_and(|current| current.can_run() && current.device == target.device);
        let result = if still_connected {
            execute(state, &target.device, step)
        } else {
            Err("Device disconnected or became view-only.".into())
        };
        if let Err(error) = result {
            let status = if state.stopped.load(Ordering::Acquire) {
                "canceled"
            } else {
                "failed"
            };
            return json!({
                "number": target.number, "status": status, "completed_steps": index,
                "failed_step": index + 1, "error": error.chars().take(600).collect::<String>(),
            });
        }
    }
    json!({ "number": target.number, "status": "succeeded", "completed_steps": steps.len() })
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

fn execute(state: &Core, device: &Device, step: &Step) -> Result<(), String> {
    let mut cmd = engine::adb_command(&state.root, device);
    match step {
        Step::OpenUrl { url } => {
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
                if state.stopped.load(Ordering::Acquire) {
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
    if state.stopped.load(Ordering::Acquire) {
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
        let server = Running::listen(PathBuf::new(), entries, 0).unwrap();
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
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let (code, job) = request(addr, &server.token, "GET", &path, "");
            assert_eq!(code, 200);
            if job["status"] != "running" {
                assert_eq!(job["status"], "succeeded");
                assert_eq!(job["devices"][0]["completed_steps"], 2);
                assert_eq!(job["devices"][1]["completed_steps"], 2);
                break;
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
