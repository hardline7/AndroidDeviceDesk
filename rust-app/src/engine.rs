use std::collections::BTreeMap;
use std::env;
use std::io::{self, Cursor, Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};
use std::{fs, thread};

use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindowVisible, PostMessageW,
    SetForegroundWindow, ShowWindow, SW_RESTORE, WM_CLOSE,
};

const NO_CONSOLE: u32 = 0x08000000;
const ENGINE_ZIP: &[u8] = include_bytes!("../../vendor/scrcpy-win64-v5.0.1.zip");
const ENGINE_VERSION: &str = "scrcpy-win64-v5.0.1";
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

pub fn data_dir() -> PathBuf {
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir)
        .join("AndroidDeviceDesk")
}

pub fn install_engine() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let root = data_dir().join("engine");
    let destination = root.join(ENGINE_VERSION);
    fs::create_dir_all(&destination)?;
    let mut zip = zip::ZipArchive::new(Cursor::new(ENGINE_ZIP))?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        let Some(path) = entry.enclosed_name() else {
            continue;
        };
        let Some(name) = path.file_name() else {
            continue;
        };
        let file = destination.join(name);
        if file
            .metadata()
            .is_ok_and(|metadata| metadata.len() == entry.size())
        {
            continue;
        }
        let staging = file.with_extension(format!("{}.part", std::process::id()));
        let mut output = fs::File::create(&staging)?;
        io::copy(&mut entry, &mut output)?;
        drop(output);
        if let Err(error) = fs::rename(&staging, &file) {
            if !file
                .metadata()
                .is_ok_and(|metadata| metadata.len() == entry.size())
            {
                return Err(error.into());
            }
            let _ = fs::remove_file(staging);
        }
    }
    Ok(destination)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Device {
    pub id: String,
    pub state: String,
    pub model: String,
    pub transport: &'static str,
    pub secondary: bool,
}

impl Device {
    pub fn ready(&self) -> bool {
        self.state == "device"
    }
}

fn parse_devices(output: &str, secondary: bool) -> BTreeMap<String, Device> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let id = fields.next()?;
            let state = fields.next()?;
            if !matches!(
                state,
                "device" | "offline" | "unauthorized" | "recovery" | "sideload" | "bootloader"
            ) {
                return None;
            }
            let model = fields
                .find_map(|field| field.strip_prefix("model:"))
                .unwrap_or("Android")
                .replace('_', " ");
            let transport = if id.contains(':') || id.ends_with("._adb-tls-connect._tcp") {
                "Wi-Fi"
            } else {
                "USB"
            };
            Some((
                id.to_owned(),
                Device {
                    id: id.to_owned(),
                    state: state.to_owned(),
                    model,
                    transport,
                    secondary,
                },
            ))
        })
        .collect()
}

pub fn hidden(program: &Path) -> Command {
    let mut command = Command::new(program);
    command.creation_flags(NO_CONSOLE);
    command
}

pub fn adb_command(root: &Path, device: &Device) -> Command {
    let mut command = hidden(&root.join("adb.exe"));
    if device.secondary {
        command.args(["-H", "127.0.0.1", "-P", "5038"]);
    }
    command.args(["-s", &device.id]);
    command
}

pub fn scrcpy_command(root: &Path, device: &Device) -> Command {
    let mut command = hidden(&root.join("scrcpy.exe"));
    command.env("ADB", root.join("adb.exe"));
    if device.secondary {
        command
            .env_remove("ADB_SERVER_SOCKET")
            .env("ANDROID_ADB_SERVER_ADDRESS", "127.0.0.1")
            .env("ANDROID_ADB_SERVER_PORT", "5038");
    }
    command.current_dir(root).args(["--serial", &device.id]);
    command
}

pub fn run(command: &mut Command, seconds: u64) -> Result<Output, String> {
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut process = command.spawn().map_err(|error| error.to_string())?;
    let stdout = drain(process.stdout.take().unwrap());
    let stderr = drain(process.stderr.take().unwrap());
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        if let Some(status) = process.try_wait().map_err(|error| error.to_string())? {
            let output = Output {
                status,
                stdout: stdout.join().unwrap_or_default(),
                stderr: stderr.join().unwrap_or_default(),
            };
            if status.success() {
                return Ok(output);
            }
            let diagnostic = if output.stderr.is_empty() {
                &output.stdout
            } else {
                &output.stderr
            };
            return Err(String::from_utf8_lossy(diagnostic).trim().to_owned());
        }
        if Instant::now() > deadline {
            let _ = process.kill();
            let _ = process.wait();
            return Err(tr!(
                "기기 응답 시간이 초과됐습니다.",
                "The device did not respond in time."
            )
            .to_owned());
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn drain(mut pipe: impl Read + Send + 'static) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = pipe.read_to_end(&mut bytes);
        bytes
    })
}

fn secondary_devices() -> io::Result<BTreeMap<String, Device>> {
    let addr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 5038);
    let mut socket = TcpStream::connect_timeout(&addr.into(), Duration::from_millis(400))?;
    socket.set_read_timeout(Some(Duration::from_millis(500)))?;
    socket.write_all(b"000ehost:devices-l")?;
    let mut header = [0; 8];
    socket.read_exact(&mut header)?;
    if &header[..4] != b"OKAY" {
        return Err(io::Error::other("ADB server reply"));
    }
    let length = std::str::from_utf8(&header[4..])
        .ok()
        .and_then(|value| usize::from_str_radix(value, 16).ok())
        .ok_or_else(|| io::Error::other("ADB payload size"))?;
    let mut data = vec![0; length];
    socket.read_exact(&mut data)?;
    Ok(parse_devices(&String::from_utf8_lossy(&data), true))
}

pub fn discover(root: &Path) -> Result<BTreeMap<String, Device>, String> {
    let primary = run(hidden(&root.join("adb.exe")).args(["devices", "-l"]), 8);
    let secondary = secondary_devices().unwrap_or_default();
    let mut devices = match primary {
        Ok(output) => parse_devices(&String::from_utf8_lossy(&output.stdout), false),
        Err(error) if secondary.is_empty() => return Err(error),
        Err(_) => BTreeMap::new(),
    };
    for (id, device) in secondary {
        if devices
            .get(&id)
            .is_none_or(|current| !current.ready() && device.ready())
        {
            devices.insert(id, device);
        }
    }
    Ok(devices)
}

pub fn png_bytes(bytes: &[u8]) -> Result<&[u8], String> {
    let start = bytes[..bytes.len().min(8192)]
        .windows(PNG_SIGNATURE.len())
        .position(|window| window == PNG_SIGNATURE)
        .ok_or_else(|| {
            tr!(
                "기기에서 화면 이미지를 받지 못했습니다.",
                "The device did not return a screen image."
            )
            .to_owned()
        })?;
    Ok(&bytes[start..])
}

pub type ScreenSize = (u32, u32);

pub struct ScreenPreview {
    pub image: eframe::egui::ColorImage,
    pub size: ScreenSize,
}

fn screen_capture(root: &Path, device: &Device) -> Result<Output, String> {
    run(
        adb_command(root, device).args(["exec-out", "screencap", "-p"]),
        5,
    )
}

pub fn screen_size(root: &Path, device: &Device) -> Result<ScreenSize, String> {
    let output = screen_capture(root, device)?;
    image::ImageReader::with_format(
        Cursor::new(png_bytes(&output.stdout)?),
        image::ImageFormat::Png,
    )
    .into_dimensions()
    .map_err(|error| error.to_string())
}

pub fn thumbnail(root: &Path, device: &Device, detail: bool) -> Result<ScreenPreview, String> {
    let output = screen_capture(root, device)?;
    let image =
        image::load_from_memory_with_format(png_bytes(&output.stdout)?, image::ImageFormat::Png)
            .map_err(|error| error.to_string())?;
    let size = (image.width(), image.height());
    let (width, height) = if detail { (400, 720) } else { (160, 280) };
    let preview = image.thumbnail(width, height).into_rgba8();
    Ok(ScreenPreview {
        image: eframe::egui::ColorImage::from_rgba_unmultiplied(
            [preview.width() as usize, preview.height() as usize],
            preview.as_raw(),
        ),
        size,
    })
}

fn window_for_pid(pid: u32) -> HWND {
    unsafe extern "system" fn find(window: HWND, argument: LPARAM) -> BOOL {
        let result = &mut *(argument as *mut (u32, HWND));
        let mut owner = 0;
        GetWindowThreadProcessId(window, &mut owner);
        if owner == result.0 && IsWindowVisible(window) != 0 {
            result.1 = window;
            return 0;
        }
        1
    }
    let mut result = (pid, std::ptr::null_mut());
    unsafe { EnumWindows(Some(find), (&mut result as *mut (u32, HWND)) as LPARAM) };
    result.1
}

pub fn close_window(pid: u32) {
    let window = window_for_pid(pid);
    if !window.is_null() {
        unsafe { PostMessageW(window, WM_CLOSE, 0, 0) };
    }
}

pub fn focus_window(pid: u32) {
    let window = window_for_pid(pid);
    if !window.is_null() {
        unsafe {
            ShowWindow(window, SW_RESTORE);
            SetForegroundWindow(window);
        }
    }
}

pub fn window_rect(pid: u32) -> Option<RECT> {
    let window = window_for_pid(pid);
    if window.is_null() {
        return None;
    }
    let mut rect: RECT = unsafe { std::mem::zeroed() };
    if unsafe { IsIconic(window) == 0 && GetWindowRect(window, &mut rect) != 0 } {
        Some(rect)
    } else {
        None
    }
}
