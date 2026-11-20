use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::Path;

use super::*;

const ARCHIVE: &[u8] = include_bytes!("../../vendor/gnirehtet-rust-win64-v2.5.1.zip");
const LICENSE: &str = include_str!("../../third-party/gnirehtet-LICENSE.txt");
const VERSION: &str = "gnirehtet-rust-win64-v2.5.1";
const PACKAGE: &str = "com.genymobile.gnirehtet";
const ACTIVITY: &str = "com.genymobile.gnirehtet/.GnirehtetActivity";
const SOCKET: &str = "localabstract:gnirehtet";

pub struct Relay {
    child: Child,
    port: u16,
}

impl Drop for Relay {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Default)]
pub struct Sharing {
    relay: Option<Relay>,
    devices: BTreeMap<String, (Device, bool)>,
    busy: bool,
    checking: bool,
    check_at: Option<Instant>,
    open: bool,
    quit_requested: bool,
    message: String,
}

pub enum ResultEvent {
    Changed {
        enabling: bool,
        relay: Option<Relay>,
        results: Vec<(Device, Result<(), String>)>,
    },
    Failed(String),
    Status(Vec<(Device, bool)>),
}

fn usb_device(device: &Device) -> bool {
    device.ready() && device.transport == "USB" && !device.id.starts_with("emulator-")
}

impl Desk {
    pub fn internet_button(&mut self, ui: &mut Ui) {
        let count = self
            .sharing
            .devices
            .values()
            .filter(|(_, running)| *running)
            .count();
        let label = if count == 0 {
            tr!("USB 인터넷 공유", "USB Internet sharing").into()
        } else {
            trf!(
                "USB 인터넷 공유 · {count}대",
                "USB Internet sharing · {count}"
            )
        };
        if ui
            .add_sized([ui.available_width(), 24.], egui::Button::new(label))
            .clicked()
        {
            self.sharing.open = true;
        }
    }

    pub fn internet_window(&mut self, ctx: &Context) {
        if !self.sharing.open {
            return;
        }
        let mut open = true;
        let targets: Vec<_> = self.ready().into_iter().filter(usb_device).collect();
        egui::Window::new(tr!("USB 인터넷 공유", "USB Internet sharing"))
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .default_width(345.)
            .show(ctx, |ui| {
                ui.label(tr!(
                    "USB로 연결된 Android 기기가 이 PC의 인터넷을 사용합니다.",
                    "Share this PC’s Internet connection with Android devices connected over USB."
                ));
                ui.label(tr!(
                    "처음에는 각 폰에서 Gnirehtet VPN 연결 요청을 승인하세요.",
                    "On first use, approve the Gnirehtet VPN request on each device."
                ));
                ui.separator();
                ui.label(trf!(
                    "선택한 USB 기기 {}대",
                    "{} USB devices selected",
                    targets.len()
                ));
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !self.sharing.busy && !targets.is_empty(),
                            egui::Button::new(tr!("선택 기기에 켜기", "Enable for selected")),
                        )
                        .clicked()
                    {
                        self.change_sharing(ctx, targets.clone(), true);
                    }
                    let to_stop: Vec<_> = targets
                        .iter()
                        .filter(|device| self.sharing.devices.contains_key(&device.id))
                        .cloned()
                        .collect();
                    if ui
                        .add_enabled(
                            !self.sharing.busy && !to_stop.is_empty(),
                            egui::Button::new(tr!("선택 기기에 끄기", "Disable for selected")),
                        )
                        .clicked()
                    {
                        self.change_sharing(ctx, to_stop, false);
                    }
                });
                if ui
                    .add_enabled(
                        !self.sharing.busy && !self.sharing.devices.is_empty(),
                        egui::Button::new(tr!("모두 끄기", "Disable all")),
                    )
                    .clicked()
                {
                    let devices = self
                        .sharing
                        .devices
                        .values()
                        .map(|(device, _)| {
                            self.devices
                                .get(&device.id)
                                .filter(|current| usb_device(current))
                                .cloned()
                                .unwrap_or_else(|| Device {
                                    state: "offline".into(),
                                    ..device.clone()
                                })
                        })
                        .collect();
                    self.change_sharing(ctx, devices, false);
                }
                if !self.sharing.message.is_empty() {
                    ui.label(&self.sharing.message);
                }
                egui::ScrollArea::vertical()
                    .max_height(230.)
                    .show(ui, |ui| {
                        let mut sessions: Vec<_> = self.sharing.devices.values().collect();
                        sessions.sort_by_key(|(device, _)| {
                            self.preferences
                                .devices
                                .get(&device.id)
                                .map(|note| note.number)
                        });
                        for (device, running) in sessions {
                            let label = if self.preferences.devices.contains_key(&device.id) {
                                self.label(&device.id)
                            } else {
                                device.model.clone()
                            };
                            let status = if !self.devices.get(&device.id).is_some_and(usb_device) {
                                tr!("USB 연결 끊김", "USB disconnected")
                            } else if *running {
                                tr!("VPN 실행 중", "VPN running")
                            } else {
                                tr!(
                                    "VPN 승인 또는 재연결 대기",
                                    "Waiting for VPN approval or reconnection"
                                )
                            };
                            ui.label(format!("{label} · {status}"));
                        }
                    });
            });
        self.sharing.open = open;
    }

    fn change_sharing(&mut self, ctx: &Context, targets: Vec<Device>, enabling: bool) {
        if self.sharing.busy || targets.is_empty() {
            return;
        }
        let port = self.sharing.relay.as_ref().map(|relay| relay.port);
        let running = self
            .sharing
            .relay
            .as_mut()
            .is_some_and(|relay| matches!(relay.child.try_wait(), Ok(None)));
        if !enabling && port.is_none() {
            return;
        }
        self.sharing.busy = true;
        self.sharing.message = if enabling {
            tr!(
                "USB 인터넷 공유를 시작하는 중…",
                "Starting USB Internet sharing…"
            )
        } else {
            tr!(
                "USB 인터넷 공유를 종료하는 중…",
                "Stopping USB Internet sharing…"
            )
        }
        .into();
        let root = self.root.clone();
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let mut relay = None;
            let started = (|| -> ResultEvent {
                let (port, client) = if enabling {
                    match prepare(port, running) {
                        Ok((new, client, port)) => {
                            relay = new;
                            (port, Some(client))
                        }
                        Err(error) => return ResultEvent::Failed(error),
                    }
                } else {
                    (port.expect("stop requires relay port"), None)
                };
                let results = parallel(&targets, |device| {
                    if let Some(client) = client.as_ref() {
                        start(&root, device, client, port)
                    } else if !usb_device(device) {
                        Ok(())
                    } else {
                        stop(&root, device, port)
                    }
                });
                ResultEvent::Changed {
                    enabling,
                    relay,
                    results,
                }
            })();
            let _ = tx.send(Event::Sharing(started));
            ctx.request_repaint();
        });
    }

    pub fn sharing_event(&mut self, event: ResultEvent) {
        match event {
            ResultEvent::Changed {
                enabling,
                relay,
                results,
            } => {
                if relay.is_some() {
                    self.sharing.relay = relay;
                }
                let total = results.len();
                let mut success = 0;
                let mut error_text = None;
                for (device, result) in results {
                    match result {
                        Ok(()) => {
                            success += 1;
                            if enabling {
                                self.sharing
                                    .devices
                                    .insert(device.id.clone(), (device, false));
                            } else {
                                self.sharing.devices.remove(&device.id);
                            }
                        }
                        Err(error) => {
                            error_text.get_or_insert(error);
                        }
                    }
                }
                self.sharing.message = if enabling {
                    trf!("{success}/{total}대에 시작 요청. 폰의 VPN 승인 창을 확인하세요.", "Start requested on {success}/{total} devices. Check the phones for a VPN prompt.")
                } else {
                    trf!(
                        "{success}/{total}대 종료",
                        "Stopped on {success}/{total} devices"
                    )
                };
                if let Some(error) = error_text {
                    self.sharing.message.push_str(&format!(
                        " · {}",
                        error.chars().take(180).collect::<String>()
                    ));
                }
                if self.sharing.devices.is_empty() {
                    self.sharing.relay = None;
                }
                self.sharing.busy = false;
                self.sharing.check_at = None;
            }
            ResultEvent::Failed(error) => {
                self.sharing.busy = false;
                self.sharing.message = error;
            }
            ResultEvent::Status(statuses) => {
                self.sharing.checking = false;
                for (checked, running) in statuses {
                    if let Some((current, flag)) = self.sharing.devices.get_mut(&checked.id) {
                        if *current == checked {
                            *flag = running;
                        }
                    }
                }
            }
        }
    }

    pub fn update_sharing(&mut self, ctx: &Context) {
        let state = &mut self.sharing;
        if ctx.input(|input| input.viewport().close_requested()) && state.busy {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            state.quit_requested = true;
        }
        if state.busy {
            return;
        }
        if state.quit_requested {
            state.quit_requested = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        let Some(relay) = state.relay.as_mut() else {
            return;
        };
        if let Ok(Some(_)) = relay.child.try_wait() {
            state.message = tr!(
                "PC 중계가 종료됐습니다. 다시 켜거나 모두 끄세요.",
                "The PC relay stopped. Enable sharing again or disable all sessions."
            )
            .into();
            for (_, running) in state.devices.values_mut() {
                *running = false;
            }
            return;
        }
        if state.checking || state.check_at.is_some_and(|at| at > Instant::now()) {
            return;
        }
        let port = relay.port;
        let devices: Vec<_> = state
            .devices
            .values()
            .map(|(device, _)| device.clone())
            .collect();
        let root = self.root.clone();
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        state.checking = true;
        state.check_at = Some(Instant::now() + Duration::from_secs(5));
        thread::spawn(move || {
            let statuses = parallel(&devices, |device| vpn_running(&root, device, port));
            let _ = tx.send(Event::Sharing(ResultEvent::Status(statuses)));
            ctx.request_repaint();
        });
    }

    pub fn stop_sharing_on_exit(&mut self) {
        let Some(relay) = self.sharing.relay.take() else {
            return;
        };
        let devices: Vec<_> = self
            .sharing
            .devices
            .values()
            .map(|(device, _)| device.clone())
            .collect();
        let _ = parallel(&devices, |device| stop(&self.root, device, relay.port));
    }
}

fn parallel<T: Send>(devices: &[Device], f: impl Fn(&Device) -> T + Sync) -> Vec<(Device, T)> {
    thread::scope(|scope| {
        devices
            .iter()
            .map(|device| {
                let f = &f;
                (device, scope.spawn(move || f(device)))
            })
            .collect::<Vec<_>>()
            .into_iter()
            .filter_map(|(device, work)| work.join().ok().map(|result| (device.clone(), result)))
            .collect()
    })
}

fn prepare(existing: Option<u16>, running: bool) -> Result<(Option<Relay>, PathBuf, u16), String> {
    let folder = engine::unpack_runtime(VERSION, ARCHIVE).map_err(|error| error.to_string())?;
    fs::write(folder.join("LICENSE.txt"), LICENSE).map_err(|error| error.to_string())?;
    let apk = folder.join("gnirehtet.apk");
    let port = match existing {
        Some(port) if running => return Ok((None, apk, port)),
        Some(port) => port,
        None => TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .and_then(|socket| socket.local_addr())
            .map_err(|error| error.to_string())?
            .port(),
    };
    let child = engine::hidden(&folder.join("gnirehtet.exe"))
        .args(["relay", "-p", &port.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    let mut relay = Relay { child, port };
    for _ in 0..40 {
        if relay
            .child
            .try_wait()
            .map_err(|error| error.to_string())?
            .is_some()
        {
            break;
        }
        if TcpStream::connect_timeout(
            &(Ipv4Addr::LOCALHOST, port).into(),
            Duration::from_millis(60),
        )
        .is_ok()
        {
            return Ok((Some(relay), apk, port));
        }
        thread::sleep(Duration::from_millis(45));
    }
    Err(tr!(
        "PC 중계를 시작하지 못했습니다.",
        "Could not start the PC relay."
    )
    .into())
}

fn addresses(output: &str) -> Option<&str> {
    output.lines().find_map(|line| {
        let mut fields = line.split_whitespace().rev();
        let address = fields.next()?;
        (fields.next()? == SOCKET).then_some(address)
    })
}

fn mapping(root: &Path, device: &Device) -> Result<Option<String>, String> {
    let output = engine::run(
        engine::adb_command(root, device).args(["reverse", "--list"]),
        4,
    )?;
    Ok(addresses(&String::from_utf8_lossy(&output.stdout)).map(str::to_owned))
}

fn launch(root: &Path, device: &Device, action: &str) -> Result<(), String> {
    let result = engine::run(
        engine::adb_command(root, device)
            .args(["shell", "am", "start", "-a", action, "-n", ACTIVITY]),
        7,
    )?;
    let text = String::from_utf8_lossy(&result.stdout);
    if text.lines().any(|line| line.starts_with("Error")) {
        return Err(text.trim().to_owned());
    }
    Ok(())
}

fn start(root: &Path, device: &Device, client: &Path, port: u16) -> Result<(), String> {
    let address = format!("tcp:{port}");
    let current = mapping(root, device)?;
    if current.as_ref().is_some_and(|current| current != &address) {
        return Err(tr!(
            "이 기기는 다른 Gnirehtet 공유를 사용 중입니다.",
            "This device already has a different Gnirehtet session."
        )
        .into());
    }
    let installed = engine::run(
        engine::adb_command(root, device).args(["shell", "pm", "path", PACKAGE]),
        5,
    )
    .is_ok_and(|result| result.stdout.starts_with(b"package:"));
    if !installed {
        engine::run(
            engine::adb_command(root, device).arg("install").arg(client),
            45,
        )?;
    }
    if current.is_none() {
        engine::run(
            engine::adb_command(root, device).args(["reverse", "--no-rebind", SOCKET, &address]),
            6,
        )?;
    }
    if let Err(error) = launch(root, device, "com.genymobile.gnirehtet.START") {
        if current.is_none() {
            let _ = engine::run(
                engine::adb_command(root, device).args(["reverse", "--remove", SOCKET]),
                4,
            );
        }
        return Err(error);
    }
    Ok(())
}

fn stop(root: &Path, device: &Device, port: u16) -> Result<(), String> {
    let current = mapping(root, device)?;
    if current
        .as_ref()
        .is_some_and(|current| current != &format!("tcp:{port}"))
    {
        return Err(tr!(
            "이 기기의 공유는 다른 중계에서 관리하고 있습니다.",
            "The current session is owned by a different relay."
        )
        .into());
    }
    launch(root, device, "com.genymobile.gnirehtet.STOP")?;
    if current.is_some() {
        engine::run(
            engine::adb_command(root, device).args(["reverse", "--remove", SOCKET]),
            4,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_only_the_gnirehtet_reverse_mapping() {
        assert_eq!(
            addresses("UsbFfs tcp:8200 tcp:8200\nUsbFfs localabstract:gnirehtet tcp:42510  \r\n"),
            Some("tcp:42510")
        );
        assert_eq!(addresses("UsbFfs localabstract:scrcpy tcp:33851\n"), None);
        assert_eq!(addresses(""), None);
    }
}

fn vpn_running(root: &Path, device: &Device, port: u16) -> bool {
    mapping(root, device).is_ok_and(|address| address.as_deref() == Some(&format!("tcp:{port}")))
        && engine::run(
            engine::adb_command(root, device)
                .args(["shell", "dumpsys", "activity", "services", PACKAGE]),
            4,
        )
        .is_ok_and(|output| {
            let text = String::from_utf8_lossy(&output.stdout);
            text.contains("GnirehtetService") && text.contains("isForeground=true")
        })
}
