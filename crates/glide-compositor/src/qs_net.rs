use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::power::PowerAction;
use crate::state::GlideState;
use crate::topbar::{Detail, PanelAction, PromptOutcome, RowTarget};

pub const RESCAN_INTERVAL: Duration = Duration::from_secs(8);
pub const PAGE_TICK: Duration = Duration::from_secs(1);
const WIFI_SCAN_SETTLE: Duration = Duration::from_secs(4);
const SCAN_TIMEOUT: Duration = Duration::from_secs(20);
const WIFI_CONNECT_TIMEOUT: Duration = Duration::from_secs(45);
const BLUETOOTH_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

pub use glide_system::spawn::{run_with_input, run_with_timeout};
pub use glide_topbar::qs_net::{
    AudioOutput, BtDevice, BtKind, ConnectError, NetEvent, Security, VpnConnection, VpnKind, WifiInfo, WifiNetwork,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BtInfo {
    pub name: Option<String>,
    pub icon: Option<String>,
    pub connected: bool,
    pub battery: Option<u8>,
}

#[must_use]
pub fn split_terse(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                if let Some(next) = chars.next()
                    && let Some(field) = fields.last_mut()
                {
                    field.push(next);
                }
            }
            ':' => fields.push(String::new()),
            other => {
                if let Some(field) = fields.last_mut() {
                    field.push(other);
                }
            }
        }
    }
    fields
}

#[must_use]
pub fn parse_wifi_list(text: &str) -> Vec<WifiNetwork> {
    let mut networks: Vec<WifiNetwork> = Vec::new();
    for line in text.lines() {
        let fields = split_terse(line);
        let [in_use, ssid, security, signal, ..] = fields.as_slice() else {
            continue;
        };
        if ssid.trim().is_empty() {
            continue;
        }
        let network = WifiNetwork {
            ssid: ssid.clone(),
            security: Security::parse(security),
            signal: signal.trim().parse::<u8>().unwrap_or(0).min(100),
            connected: in_use.trim() == "*",
            saved: false,
        };
        match networks.iter_mut().find(|known| known.ssid == network.ssid) {
            Some(known) => {
                known.connected |= network.connected;
                known.security = known.security.max(network.security);
                known.signal = known.signal.max(network.signal);
            }
            None => networks.push(network),
        }
    }
    networks
}

pub fn mark_saved(networks: &mut [WifiNetwork], saved: &[String]) {
    for network in networks {
        network.saved = saved.contains(&network.ssid);
    }
}

pub fn sort_networks(networks: &mut [WifiNetwork]) {
    networks.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then(b.level().cmp(&a.level()))
            .then_with(|| a.ssid.to_lowercase().cmp(&b.ssid.to_lowercase()))
    });
}

#[must_use]
pub fn parse_bt_devices(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("Device ")?;
            let (mac, name) = rest.split_once(' ').unwrap_or((rest, ""));
            let valid = mac.len() == 17 && mac.split(':').count() == 6;
            valid.then(|| {
                let name = name.trim();
                let name = if name.is_empty() { mac } else { name };
                (mac.to_string(), name.to_string())
            })
        })
        .collect()
}

#[must_use]
pub fn parse_bt_info(text: &str) -> BtInfo {
    let mut info = BtInfo::default();
    for line in text.lines() {
        let Some((key, value)) = line.trim().split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "Alias" => info.name = Some(value.to_string()),
            "Name" if info.name.is_none() => info.name = Some(value.to_string()),
            "Icon" => info.icon = Some(value.to_string()),
            "Connected" => info.connected = value == "yes",
            "Battery Percentage" => {
                info.battery = value
                    .split_once('(')
                    .and_then(|(_, tail)| tail.trim_end_matches(')').trim().parse::<u8>().ok())
                    .map(|pct| pct.min(100));
            }
            _ => {}
        }
    }
    info
}

#[must_use]
pub fn device_from(mac: String, listed_name: String, info: &BtInfo) -> BtDevice {
    BtDevice {
        name: info.name.clone().filter(|name| !name.is_empty()).unwrap_or(listed_name),
        kind: info.icon.as_deref().map_or(BtKind::Other, BtKind::from_icon),
        connected: info.connected,
        battery: info.battery,
        mac,
    }
}

pub fn sort_devices(devices: &mut [BtDevice]) {
    devices
        .sort_by(|a, b| b.connected.cmp(&a.connected).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
}

#[must_use]
pub fn classify_wifi_failure(stderr: &str, had_password: bool) -> ConnectError {
    let text = stderr.to_ascii_lowercase();
    let secrets = text.contains("secrets were required")
        || text.contains("802-11-wireless-security.psk")
        || text.contains("no secrets");
    if secrets || (had_password && text.contains("password")) {
        ConnectError::WrongPassword
    } else {
        ConnectError::Failed
    }
}

#[must_use]
pub fn bluetooth_succeeded(status_ok: bool, stdout: &str) -> bool {
    status_ok && !stdout.contains("Failed") && !stdout.contains("not available")
}

#[must_use]
pub fn output_icon(form_factor: Option<&str>, bus: Option<&str>, port: &str) -> &'static str {
    let port = port.to_ascii_lowercase();
    match form_factor {
        Some("headset" | "hands-free") => return "audio-headset",
        Some("headphone") => return "audio-headphones",
        Some("speaker") => return "audio-speakers",
        _ => {}
    }
    if port.contains("headset") {
        "audio-headset"
    } else if port.contains("headphone") || bus == Some("bluetooth") || port.contains("bluez") {
        "audio-headphones"
    } else if port.contains("hdmi") || port.contains("displayport") {
        "video-display"
    } else {
        "audio-speakers"
    }
}

#[must_use]
pub fn parse_sinks_json(text: &str, default: &str) -> Vec<AudioOutput> {
    use serde_json::Value;
    let Ok(Value::Array(sinks)) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    sinks
        .iter()
        .filter_map(|sink| {
            let name = sink.get("name")?.as_str()?.to_string();
            let property = |key: &str| sink.get("properties").and_then(|map| map.get(key)).and_then(Value::as_str);
            let description =
                sink.get("description").and_then(Value::as_str).filter(|text| !text.is_empty()).unwrap_or(&name);
            let port = sink.get("active_port").and_then(Value::as_str).unwrap_or(&name);
            Some(AudioOutput {
                description: description.to_string(),
                icon: output_icon(property("device.form_factor"), property("device.bus"), port),
                active: name == default,
                name,
            })
        })
        .collect()
}

#[must_use]
pub fn parse_sinks_short(text: &str, default: &str) -> Vec<AudioOutput> {
    text.lines()
        .filter_map(|line| {
            let name = line.split('\t').nth(1)?.trim();
            (!name.is_empty()).then(|| AudioOutput {
                name: name.to_string(),
                description: name.to_string(),
                icon: output_icon(None, None, name),
                active: name == default,
            })
        })
        .collect()
}

#[must_use]
pub fn input_icon(form_factor: Option<&str>, bus: Option<&str>, port: &str) -> &'static str {
    let port = port.to_ascii_lowercase();
    match form_factor {
        Some("headset" | "hands-free") => return "audio-headset",
        Some("webcam") => return "camera-web",
        _ => {}
    }
    if port.contains("headset") || bus == Some("bluetooth") || port.contains("bluez") {
        "audio-headset"
    } else {
        "audio-input-microphone"
    }
}

fn is_monitor(source: &serde_json::Value, name: &str) -> bool {
    let monitor_of = source.get("monitor_source").and_then(serde_json::Value::as_str).unwrap_or_default();
    let class = source.get("properties").and_then(|map| map.get("device.class")).and_then(serde_json::Value::as_str);
    !monitor_of.is_empty() || class == Some("monitor") || name.ends_with(".monitor")
}

#[must_use]
pub fn parse_sources_json(text: &str, default: &str) -> Vec<AudioOutput> {
    use serde_json::Value;
    let Ok(Value::Array(sources)) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    sources
        .iter()
        .filter_map(|source| {
            let name = source.get("name")?.as_str()?.to_string();
            if is_monitor(source, &name) {
                return None;
            }
            let property = |key: &str| source.get("properties").and_then(|map| map.get(key)).and_then(Value::as_str);
            let description =
                source.get("description").and_then(Value::as_str).filter(|text| !text.is_empty()).unwrap_or(&name);
            let port = source.get("active_port").and_then(Value::as_str).unwrap_or(&name);
            Some(AudioOutput {
                description: description.to_string(),
                icon: input_icon(property("device.form_factor"), property("device.bus"), port),
                active: name == default,
                name,
            })
        })
        .collect()
}

#[must_use]
pub fn parse_sources_short(text: &str, default: &str) -> Vec<AudioOutput> {
    text.lines()
        .filter_map(|line| {
            let name = line.split('\t').nth(1)?.trim();
            (!name.is_empty() && !name.ends_with(".monitor")).then(|| AudioOutput {
                name: name.to_string(),
                description: name.to_string(),
                icon: input_icon(None, None, name),
                active: name == default,
            })
        })
        .collect()
}

pub fn sort_outputs(outputs: &mut [AudioOutput]) {
    outputs.sort_by(|a, b| {
        b.active.cmp(&a.active).then_with(|| a.description.to_lowercase().cmp(&b.description.to_lowercase()))
    });
}

pub enum NetRequest {
    ScanWifi,
    RescanWifi,
    RefreshWifi,
    ScanBluetooth,
    ScanOutputs,
    ScanInputs,
    ScanVpn,
    Vpn { name: String, up: bool },
    ConnectWifi { ssid: String, password: Option<String>, saved: bool },
    DisconnectWifi { ssid: String },
    ForgetWifi { ssid: String },
    Bluetooth { mac: String, connect: bool },
    UseOutput { name: String },
    UseInput { name: String },
}

impl std::fmt::Debug for NetRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ScanWifi => f.write_str("ScanWifi"),
            Self::RescanWifi => f.write_str("RescanWifi"),
            Self::RefreshWifi => f.write_str("RefreshWifi"),
            Self::ScanBluetooth => f.write_str("ScanBluetooth"),
            Self::ScanOutputs => f.write_str("ScanOutputs"),
            Self::ScanInputs => f.write_str("ScanInputs"),
            Self::ScanVpn => f.write_str("ScanVpn"),
            Self::Vpn { name, up } => f.debug_struct("Vpn").field("name", name).field("up", up).finish(),
            Self::UseOutput { name } => f.debug_struct("UseOutput").field("name", name).finish(),
            Self::UseInput { name } => f.debug_struct("UseInput").field("name", name).finish(),
            Self::ConnectWifi { ssid, password, saved } => f
                .debug_struct("ConnectWifi")
                .field("ssid", ssid)
                .field("password", &password.as_ref().map(|_| "<redacted>"))
                .field("saved", saved)
                .finish(),
            Self::DisconnectWifi { ssid } => f.debug_struct("DisconnectWifi").field("ssid", ssid).finish(),
            Self::ForgetWifi { ssid } => f.debug_struct("ForgetWifi").field("ssid", ssid).finish(),
            Self::Bluetooth { mac, connect } => {
                f.debug_struct("Bluetooth").field("mac", mac).field("connect", connect).finish()
            }
        }
    }
}

type Sink = Box<dyn Fn(NetEvent) + Send + Sync>;

static SINK: OnceLock<Sink> = OnceLock::new();
static WIFI_SCANNING: AtomicBool = AtomicBool::new(false);
static WIFI_LISTING: AtomicBool = AtomicBool::new(false);
static WIFI_CONNECTING: AtomicU64 = AtomicU64::new(0);
static SAVED_SSIDS: Mutex<Option<Vec<String>>> = Mutex::new(None);
static BLUETOOTH_SCANNING: AtomicBool = AtomicBool::new(false);
static OUTPUTS_SCANNING: AtomicBool = AtomicBool::new(false);
static INPUTS_SCANNING: AtomicBool = AtomicBool::new(false);
static OUTPUTS_LOCK: Mutex<()> = Mutex::new(());
static INPUTS_LOCK: Mutex<()> = Mutex::new(());
static OUTPUTS_REQUEST: AtomicU64 = AtomicU64::new(0);
static INPUTS_REQUEST: AtomicU64 = AtomicU64::new(0);

pub fn install(sink: impl Fn(NetEvent) + Send + Sync + 'static) {
    if SINK.set(Box::new(sink)).is_err() {
        tracing::warn!("the quick settings network worker was installed twice");
    }
}

fn emit(event: NetEvent) {
    if let Some(sink) = SINK.get() {
        sink(event);
    }
}

pub fn request(request: NetRequest) {
    if SINK.get().is_none() {
        return;
    }
    let flag = match request {
        NetRequest::ScanWifi => Some(&WIFI_LISTING),
        NetRequest::RescanWifi | NetRequest::RefreshWifi if WIFI_CONNECTING.load(Ordering::Acquire) > 0 => return,
        NetRequest::RescanWifi | NetRequest::RefreshWifi => Some(&WIFI_SCANNING),
        NetRequest::ScanBluetooth => Some(&BLUETOOTH_SCANNING),
        NetRequest::ScanOutputs => Some(&OUTPUTS_SCANNING),
        NetRequest::ScanInputs => Some(&INPUTS_SCANNING),
        _ => None,
    };
    if let Some(flag) = flag
        && flag.swap(true, Ordering::AcqRel)
    {
        return;
    }
    let audio = match &request {
        NetRequest::ScanOutputs => Some((&OUTPUTS_LOCK, None)),
        NetRequest::ScanInputs => Some((&INPUTS_LOCK, None)),
        NetRequest::UseOutput { .. } => {
            Some((&OUTPUTS_LOCK, Some((&OUTPUTS_REQUEST, OUTPUTS_REQUEST.fetch_add(1, Ordering::SeqCst) + 1))))
        }
        NetRequest::UseInput { .. } => {
            Some((&INPUTS_LOCK, Some((&INPUTS_REQUEST, INPUTS_REQUEST.fetch_add(1, Ordering::SeqCst) + 1))))
        }
        _ => None,
    };
    tracing::debug!(?request, "quick settings network request");
    let spawned = std::thread::Builder::new().name("glide-qs-net".to_string()).spawn(move || {
        let _guard = audio.map(|(lock, _)| lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
        if audio.and_then(|(_, generation)| generation).is_none_or(|(latest, id)| latest.load(Ordering::SeqCst) == id) {
            run_request(request);
        }
        if let Some(flag) = flag {
            flag.store(false, Ordering::Release);
        }
    });
    if let Err(err) = spawned {
        tracing::warn!("quick settings network worker did not start: {err}");
        if let Some(flag) = flag {
            flag.store(false, Ordering::Release);
        }
    }
}

fn emit_wifi() {
    forget_saved_ssids();
    emit_scanned(list_wifi());
}

pub(crate) fn request_wifi_scan() -> bool {
    let requested = run_with_timeout("nmcli", &["dev", "wifi", "rescan"], SCAN_TIMEOUT)
        .is_some_and(|output| output.status.success());
    if !requested {
        tracing::debug!("NetworkManager refused a wi-fi scan request");
    }
    requested
}

fn run_request(request: NetRequest) {
    match request {
        NetRequest::ScanWifi => {
            if let Some(networks) = list_wifi() {
                emit(NetEvent::Wifi(networks));
            }
        }
        NetRequest::RescanWifi => {
            emit(NetEvent::WifiScanning(true));
            forget_saved_ssids();
            request_wifi_scan();
            emit_scanned(list_wifi());
            std::thread::sleep(WIFI_SCAN_SETTLE);
            emit_scanned(list_wifi());
            emit(NetEvent::WifiScanning(false));
        }
        NetRequest::RefreshWifi => {
            if request_wifi_scan() {
                std::thread::sleep(WIFI_SCAN_SETTLE);
                if let Some(networks) = list_wifi() {
                    emit(NetEvent::Wifi(networks));
                }
                emit(NetEvent::WifiScanned);
            }
        }
        NetRequest::ForgetWifi { ssid } => {
            let result = forget_wifi(&ssid);
            emit(NetEvent::WifiResult { ssid, result });
            emit_wifi();
        }
        NetRequest::ScanBluetooth => emit(NetEvent::Bluetooth(scan_bluetooth())),
        NetRequest::ConnectWifi { ssid, password, saved } => {
            WIFI_CONNECTING.fetch_add(1, Ordering::AcqRel);
            let result = connect_wifi(&ssid, password.as_deref(), saved);
            WIFI_CONNECTING.fetch_sub(1, Ordering::AcqRel);
            if let Err(error) = result {
                tracing::info!(ssid, ?error, "wi-fi connection failed");
            }
            emit(NetEvent::WifiResult { ssid, result });
            emit_wifi();
        }
        NetRequest::DisconnectWifi { ssid } => {
            let result = disconnect_wifi(&ssid);
            emit(NetEvent::WifiResult { ssid, result });
            emit_wifi();
        }
        NetRequest::Bluetooth { mac, connect } => {
            let verb = if connect { "connect" } else { "disconnect" };
            let result = run_with_timeout("bluetoothctl", &[verb, &mac], BLUETOOTH_CONNECT_TIMEOUT)
                .filter(|output| bluetooth_succeeded(output.status.success(), &String::from_utf8_lossy(&output.stdout)))
                .map(|_| ())
                .ok_or(ConnectError::Failed);
            emit(NetEvent::BluetoothResult { mac, result });
            emit(NetEvent::Bluetooth(scan_bluetooth()));
        }
        NetRequest::ScanOutputs => emit(NetEvent::Outputs(scan_outputs())),
        NetRequest::ScanInputs => emit(NetEvent::Inputs(scan_inputs())),
        NetRequest::ScanVpn => emit(NetEvent::Vpn(scan_vpn())),
        NetRequest::Vpn { name, up } => {
            let verb = if up { "up" } else { "down" };
            let result = run_with_timeout("nmcli", &["con", verb, "id", &name], WIFI_CONNECT_TIMEOUT)
                .filter(|output| output.status.success())
                .map(|_| ())
                .ok_or(ConnectError::Failed);
            if result.is_err() {
                tracing::info!(name, up, "switching the vpn connection failed");
            }
            emit(NetEvent::VpnResult { name, result });
            emit(NetEvent::Vpn(scan_vpn()));
        }
        NetRequest::UseOutput { name } => {
            let result = set_default("set-default-sink", &name);
            emit(NetEvent::OutputResult { name, result });
            emit(NetEvent::Outputs(scan_outputs()));
        }
        NetRequest::UseInput { name } => {
            let result = set_default("set-default-source", &name);
            emit(NetEvent::InputResult { name, result });
            emit(NetEvent::Inputs(scan_inputs()));
        }
    }
}

fn set_default(verb: &str, name: &str) -> Result<(), ConnectError> {
    let result = run_with_timeout("pactl", &[verb, name], SCAN_TIMEOUT)
        .filter(|output| output.status.success())
        .map(|_| ())
        .ok_or(ConnectError::Failed);
    if result.is_err() {
        tracing::info!(name, verb, "switching the audio device failed");
    }
    crate::system_info::poke(crate::system_info::Topics::AUDIO);
    result
}

fn scan_inputs() -> Vec<AudioOutput> {
    let default = stdout_of("pactl", &["get-default-source"]).map(|text| text.trim().to_string()).unwrap_or_default();
    let mut inputs = stdout_of("pactl", &["--format=json", "list", "sources"])
        .map(|text| parse_sources_json(&text, &default))
        .filter(|inputs| !inputs.is_empty())
        .or_else(|| stdout_of("pactl", &["list", "short", "sources"]).map(|text| parse_sources_short(&text, &default)))
        .unwrap_or_default();
    sort_outputs(&mut inputs);
    inputs
}

fn scan_outputs() -> Vec<AudioOutput> {
    let default = stdout_of("pactl", &["get-default-sink"]).map(|text| text.trim().to_string()).unwrap_or_default();
    let mut outputs = stdout_of("pactl", &["--format=json", "list", "sinks"])
        .map(|text| parse_sinks_json(&text, &default))
        .filter(|outputs| !outputs.is_empty())
        .or_else(|| stdout_of("pactl", &["list", "short", "sinks"]).map(|text| parse_sinks_short(&text, &default)))
        .unwrap_or_default();
    sort_outputs(&mut outputs);
    outputs
}

fn wifi_connect_args(ssid: &str, has_password: bool, saved: bool) -> Vec<&str> {
    match (has_password, saved) {
        (true, _) => vec!["--ask", "dev", "wifi", "connect", ssid],
        (false, true) => vec!["con", "up", "id", ssid],
        (false, false) => vec!["dev", "wifi", "connect", ssid],
    }
}

fn stdout_of(program: &str, args: &[&str]) -> Option<String> {
    run_with_timeout(program, args, SCAN_TIMEOUT)
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn list_wifi() -> Option<Vec<WifiNetwork>> {
    let list =
        stdout_of("nmcli", &["-t", "-f", "IN-USE,SSID,SECURITY,SIGNAL,BARS", "dev", "wifi", "list", "--rescan", "no"])?;
    let mut networks = parse_wifi_list(&list);
    let mut saved = SAVED_SSIDS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let saved = saved.get_or_insert_with(|| saved_wifi_profiles().into_iter().map(|(ssid, _)| ssid).collect());
    mark_saved(&mut networks, saved);
    sort_networks(&mut networks);
    Some(networks)
}

fn forget_saved_ssids() {
    *SAVED_SSIDS.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}

fn emit_scanned(networks: Option<Vec<WifiNetwork>>) {
    if let Some(networks) = networks {
        emit(NetEvent::Wifi(networks));
    }
    emit(NetEvent::WifiInfo(connected_wifi_info()));
}

#[must_use]
pub fn parse_vpn_connections(text: &str) -> Vec<VpnConnection> {
    let mut vpns: Vec<VpnConnection> = text
        .lines()
        .filter_map(|line| {
            let fields = split_terse(line);
            let [name, kind, active, ..] = fields.as_slice() else { return None };
            let kind = match kind.as_str() {
                "wireguard" => VpnKind::WireGuard,
                "vpn" => VpnKind::Vpn,
                _ => return None,
            };
            (!name.is_empty()).then(|| VpnConnection { name: name.clone(), kind, active: active == "yes" })
        })
        .collect();
    vpns.sort_by(|a, b| b.active.cmp(&a.active).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    vpns
}

fn scan_vpn() -> Vec<VpnConnection> {
    stdout_of("nmcli", &["-t", "-f", "NAME,TYPE,ACTIVE", "con", "show"])
        .map(|text| parse_vpn_connections(&text))
        .unwrap_or_default()
}

fn saved_wifi_profiles() -> Vec<(String, String)> {
    let Some(profiles) = stdout_of("nmcli", &["-t", "-f", "UUID,TYPE", "con", "show"]) else {
        return Vec::new();
    };
    profiles
        .lines()
        .filter_map(|line| {
            let fields = split_terse(line);
            let [uuid, kind, ..] = fields.as_slice() else { return None };
            if uuid.is_empty() || !matches!(kind.as_str(), "802-11-wireless" | "wifi") {
                return None;
            }
            let ssid =
                stdout_of("nmcli", &["--escape", "no", "-g", "802-11-wireless.ssid", "con", "show", "uuid", uuid])?;
            let ssid = ssid.trim_end_matches(['\r', '\n']);
            (!ssid.is_empty()).then(|| (ssid.to_string(), uuid.clone()))
        })
        .collect()
}

fn connect_wifi(ssid: &str, password: Option<&str>, saved: bool) -> Result<(), ConnectError> {
    if saved && password.is_none() {
        let uuid = saved_wifi_profiles()
            .into_iter()
            .find_map(|(profile_ssid, uuid)| (profile_ssid == ssid).then_some(uuid))
            .ok_or(ConnectError::Failed)?;
        return run_with_timeout("nmcli", &["con", "up", "uuid", &uuid], WIFI_CONNECT_TIMEOUT)
            .filter(|output| output.status.success())
            .map(|_| ())
            .ok_or(ConnectError::Failed);
    }
    let args = wifi_connect_args(ssid, password.is_some(), saved);
    let Some(output) = run_with_input("nmcli", &args, password, WIFI_CONNECT_TIMEOUT) else {
        return Err(ConnectError::Failed);
    };
    if output.status.success() {
        return Ok(());
    }
    Err(classify_wifi_failure(&String::from_utf8_lossy(&output.stderr), password.is_some()))
}

fn wifi_device_for_ssid(text: &str, ssid: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let fields = split_terse(line);
        let [active, name, device, ..] = fields.as_slice() else { return None };
        (active.trim() == "*" && name == ssid && !device.is_empty()).then(|| device.clone())
    })
}

fn forget_wifi(ssid: &str) -> Result<(), ConnectError> {
    let uuids: Vec<String> = saved_wifi_profiles()
        .into_iter()
        .filter_map(|(profile_ssid, uuid)| (profile_ssid == ssid).then_some(uuid))
        .collect();
    if uuids.is_empty() {
        return disconnect_wifi(ssid);
    }
    let deleted = uuids.iter().all(|uuid| {
        run_with_timeout("nmcli", &["con", "delete", "uuid", uuid], WIFI_CONNECT_TIMEOUT)
            .is_some_and(|output| output.status.success())
    });
    if deleted { Ok(()) } else { Err(ConnectError::Failed) }
}

#[must_use]
pub fn parse_connected_wifi(text: &str) -> Option<(String, u32, u32)> {
    text.lines().find_map(|line| {
        let fields = split_terse(line);
        let [active, device, freq, rate, ..] = fields.as_slice() else { return None };
        let number = |text: &str| text.split_whitespace().next().and_then(|n| n.parse().ok()).unwrap_or(0);
        (active.trim() == "*" && !device.is_empty()).then(|| (device.clone(), number(freq), number(rate)))
    })
}

fn connected_wifi_info() -> Option<WifiInfo> {
    let list = stdout_of("nmcli", &["-t", "-f", "IN-USE,DEVICE,FREQ,RATE", "dev", "wifi", "list", "--rescan", "no"])?;
    let (device, freq_mhz, rate_mbps) = parse_connected_wifi(&list)?;
    let ip = stdout_of("nmcli", &["-g", "IP4.ADDRESS", "dev", "show", &device])
        .and_then(|text| text.lines().next().map(|line| line.split('/').next().unwrap_or_default().trim().to_string()))
        .unwrap_or_default();
    Some(WifiInfo { ip, freq_mhz, rate_mbps })
}

fn disconnect_wifi(ssid: &str) -> Result<(), ConnectError> {
    let device = stdout_of("nmcli", &["-t", "-f", "IN-USE,SSID,DEVICE", "dev", "wifi", "list", "--rescan", "no"])
        .and_then(|text| wifi_device_for_ssid(&text, ssid))
        .ok_or(ConnectError::Failed)?;
    run_with_timeout("nmcli", &["dev", "disconnect", &device], WIFI_CONNECT_TIMEOUT)
        .filter(|output| output.status.success())
        .map(|_| ())
        .ok_or(ConnectError::Failed)
}

fn scan_bluetooth() -> Vec<BtDevice> {
    let listed = stdout_of("bluetoothctl", &["devices", "Paired"])
        .map(|text| parse_bt_devices(&text))
        .filter(|devices| !devices.is_empty())
        .or_else(|| stdout_of("bluetoothctl", &["paired-devices"]).map(|text| parse_bt_devices(&text)))
        .unwrap_or_default();
    let mut devices: Vec<BtDevice> = listed
        .into_iter()
        .map(|(mac, name)| {
            let info = stdout_of("bluetoothctl", &["info", &mac]).map(|text| parse_bt_info(&text)).unwrap_or_default();
            device_from(mac, name, &info)
        })
        .collect();
    sort_devices(&mut devices);
    devices
}

#[must_use]
pub const fn use_device(drawer: crate::topbar::Drawer, name: String) -> NetRequest {
    match drawer {
        crate::topbar::Drawer::Output => NetRequest::UseOutput { name },
        crate::topbar::Drawer::Input => NetRequest::UseInput { name },
    }
}

#[must_use]
pub const fn scan_for(detail: Detail) -> Option<NetRequest> {
    match detail {
        Detail::Wifi => Some(NetRequest::RescanWifi),
        Detail::Bluetooth => Some(NetRequest::ScanBluetooth),
        Detail::Sound => Some(NetRequest::ScanOutputs),
        Detail::Vpn => Some(NetRequest::ScanVpn),
        Detail::Battery | Detail::Power => None,
    }
}

#[must_use]
pub const fn rescan_every(detail: Detail, on_battery: bool) -> Duration {
    match (detail, on_battery) {
        (Detail::Wifi, false) => WIFI_SCAN_SETTLE,
        (Detail::Wifi, true) => Duration::from_secs(8),
        _ => RESCAN_INTERVAL,
    }
}

fn scan_detail(detail: Detail) {
    if let Some(scan) = scan_for(detail) {
        request(scan);
    }
    if detail == Detail::Sound {
        request(NetRequest::ScanInputs);
    }
}

pub(crate) fn open_settings_panel(slug: &str) {
    let mut command = Command::new("glide-settings");
    command.arg(slug);
    if let Err(err) = crate::spawn_detached(command) {
        tracing::warn!("could not open glide-settings {slug}: {err}");
    }
}

impl GlideState {
    pub fn handle_net_event(&mut self, event: NetEvent) {
        if self.topbar.apply_net_event(event) {
            self.queue_redraw();
        }
        self.sync_prompt_caps_lock();
    }

    pub(crate) fn sync_prompt_caps_lock(&mut self) {
        let on = self.seat.get_keyboard().is_some_and(|keyboard| keyboard.modifier_state().caps_lock);
        if self.topbar.set_caps_lock(on) {
            self.queue_redraw();
        }
    }

    pub fn tick_quick_settings_panel(&mut self) {
        let Some(detail) = self.topbar.detail() else {
            self.session.qs_scanned = None;
            return;
        };
        if detail == Detail::Wifi {
            request(NetRequest::ScanWifi);
        }
        let now = std::time::Instant::now();
        let every = rescan_every(detail, self.topbar.scans_on_battery());
        if self.session.qs_scanned.is_none_or(|at| now.saturating_duration_since(at) >= every) {
            self.session.qs_scanned = Some(now);
            match detail {
                Detail::Wifi => request(NetRequest::RefreshWifi),
                _ => scan_detail(detail),
            }
        }
    }

    pub(crate) fn toggle_networking(&mut self) {
        let was_on = self.topbar.system_status().ethernet_connected;
        crate::system_info::set_networking(!was_on);
        self.update_system_status(|status| status.ethernet_connected = !was_on);
    }

    pub(crate) fn run_power_button(&mut self, action: PowerAction) {
        self.topbar.close_popup();
        self.perform_power_action(action);
        self.queue_redraw();
    }

    pub(crate) fn run_qs_panel_action(&mut self, action: PanelAction) {
        match action {
            PanelAction::Open(detail) => {
                if self.topbar.open_detail(detail) {
                    self.session.qs_scanned = Some(std::time::Instant::now());
                    scan_detail(detail);
                    if detail == Detail::Vpn {
                        self.netbird_request(crate::netbird::Request::Refresh);
                    }
                }
            }
            PanelAction::Back => {
                self.topbar.close_detail();
            }
            PanelAction::Toggle => match self.topbar.detail() {
                Some(Detail::Wifi) if !self.topbar.system_status().wifi_available => self.toggle_networking(),
                Some(Detail::Wifi) => {
                    self.handle_qs_action(crate::topbar::TopbarAction::QsToggleWifi);
                    request(NetRequest::ScanWifi);
                }
                Some(Detail::Bluetooth) => {
                    self.handle_qs_action(crate::topbar::TopbarAction::QsToggleBluetooth);
                    request(NetRequest::ScanBluetooth);
                }
                Some(Detail::Vpn | Detail::Battery | Detail::Sound | Detail::Power) | None => {}
            },
            PanelAction::Row(index) => {
                self.activate_detail_row(index);
                self.sync_prompt_caps_lock();
            }
            PanelAction::FocusPassword => self.topbar.focus_password(),
            PanelAction::PromptReveal => {
                self.topbar.toggle_reveal();
            }
            PanelAction::Rescan => request(NetRequest::RescanWifi),
            PanelAction::Forget | PanelAction::Disconnect => {
                if let Some(ssid) = self.topbar.expanded_network().map(str::to_string) {
                    self.topbar.toggle_info(&ssid);
                    self.topbar.begin_disconnect(&ssid);
                    request(if action == PanelAction::Forget {
                        NetRequest::ForgetWifi { ssid }
                    } else {
                        NetRequest::DisconnectWifi { ssid }
                    });
                }
            }
            PanelAction::PromptCancel => self.run_qs_field_key(crate::topbar::FieldKey::Cancel),
            PanelAction::PromptSubmit => self.run_qs_field_key(crate::topbar::FieldKey::Submit),
            PanelAction::Settings => {
                let detail = self.topbar.detail();
                self.topbar.close_popup();
                match detail {
                    Some(Detail::Battery) => crate::system_info::open_power_settings(),
                    Some(Detail::Bluetooth) => open_settings_panel("bluetooth"),
                    Some(Detail::Sound) => open_settings_panel("sound"),
                    Some(Detail::Vpn) => open_settings_panel("vpn"),
                    _ => open_settings_panel("network"),
                }
            }
        }
        self.queue_redraw();
    }

    fn activate_detail_row(&mut self, index: u16) {
        match self.topbar.detail_row(index) {
            Some(RowTarget::Network(network)) if network.connected => self.topbar.toggle_info(&network.ssid),
            Some(RowTarget::Network(network)) if network.security == Security::Enterprise && !network.saved => {
                self.topbar.close_popup();
                open_settings_panel("network");
            }
            Some(RowTarget::Network(network)) if network.saved || !network.security.is_secured() => {
                self.topbar.begin_connect(&network.ssid);
                request(NetRequest::ConnectWifi { ssid: network.ssid, password: None, saved: network.saved });
            }
            Some(RowTarget::Network(network)) => self.topbar.toggle_prompt(&network.ssid),
            Some(RowTarget::Device(device)) => {
                self.topbar.begin_connect(&device.mac);
                request(NetRequest::Bluetooth { connect: !device.connected, mac: device.mac });
            }
            Some(RowTarget::Output(output)) if !output.active => {
                self.topbar.begin_connect(&output.name);
                request(NetRequest::UseOutput { name: output.name });
            }
            Some(target @ (RowTarget::Netbird | RowTarget::Vpn(_))) => self.switch_vpn(target),
            Some(RowTarget::Output(_)) | None => {}
        }
    }

    pub(crate) fn switch_vpn(&mut self, target: RowTarget) {
        match target {
            RowTarget::Netbird => self.toggle_netbird(),
            RowTarget::Vpn(vpn) => {
                self.topbar.begin_connect(&vpn.name);
                request(NetRequest::Vpn { up: !vpn.active, name: vpn.name });
            }
            RowTarget::Network(_) | RowTarget::Device(_) | RowTarget::Output(_) => {}
        }
    }

    pub(crate) fn toggle_vpn_tile(&mut self) {
        if let Some(target) = self.topbar.vpn_toggle_target() {
            self.switch_vpn(target);
        }
    }

    pub(crate) fn refresh_quick_settings_lists() {
        request(NetRequest::ScanVpn);
        request(NetRequest::ScanBluetooth);
    }

    pub(crate) fn run_qs_field_key(&mut self, key: crate::topbar::FieldKey) {
        match self.topbar.prompt_key(key) {
            PromptOutcome::Unchanged => return,
            PromptOutcome::Changed | PromptOutcome::Cancelled => {}
            PromptOutcome::Submit { ssid, password } => {
                self.topbar.begin_connect(&ssid);
                request(NetRequest::ConnectWifi { ssid, password: Some(password), saved: false });
            }
        }
        self.queue_redraw();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terse_fields_keep_escaped_colons_and_backslashes() {
        assert_eq!(split_terse(r"*:Cafe\: Guest:WPA2:81:▂▄▆█"), ["*", "Cafe: Guest", "WPA2", "81", "▂▄▆█"]);
        assert_eq!(split_terse(r"a\\b:c"), [r"a\b", "c"]);
        assert_eq!(split_terse(" ::--:0:"), [" ", "", "--", "0", ""]);
    }

    #[test]
    fn duplicate_ssids_collapse_to_the_strongest_and_hidden_ones_are_dropped() {
        let text = [
            " :Home:WPA2:40:▂▄__",
            "*:Home:WPA2:72:▂▄▆_",
            " :Home:WPA2:55:▂▄▆_",
            " ::WPA2:90:▂▄▆█",
            " :Library:--:30:▂▄__",
            r" :Lab\:5G:WPA3:12:▂___",
        ]
        .join("\n");
        let networks = parse_wifi_list(&text);
        assert_eq!(networks.len(), 3);
        let home = &networks[0];
        assert_eq!(home.ssid, "Home");
        assert_eq!(home.signal, 72);
        assert!(home.connected && home.security == Security::Passphrase);
        assert_eq!(networks[1].security, Security::Open, "-- means an open network");
        assert_eq!(networks[2].ssid, "Lab:5G");
    }

    #[test]
    fn enterprise_networks_are_told_apart_from_passphrase_ones() {
        let networks = parse_wifi_list(" :eduroam:WPA2 802.1X:70:x\n :Office:WPA3 EAP:60:x\n :Home:WPA1 WPA2:50:x");
        let kinds: Vec<Security> = networks.iter().map(|network| network.security).collect();
        assert_eq!(kinds, [Security::Enterprise, Security::Enterprise, Security::Passphrase]);
        assert!(Security::Enterprise.is_secured() && !Security::Open.is_secured());
        let merged = parse_wifi_list(" :Campus:WPA2:40:x\n :Campus:WPA2 802.1X:70:x");
        assert_eq!(merged[0].security, Security::Enterprise, "the strictest access point wins");
    }

    #[test]
    fn the_connected_network_sorts_first_then_by_bars_then_by_name() {
        let network = |ssid: &str, signal: u8, connected: bool| WifiNetwork {
            ssid: ssid.to_string(),
            security: Security::Passphrase,
            signal,
            connected,
            saved: false,
        };
        let mut networks = vec![
            network("b", 90, false),
            network("weak-home", 20, true),
            network("a", 90, false),
            network("c", 95, false),
            network("d", 60, false),
        ];
        sort_networks(&mut networks);
        let order: Vec<&str> = networks.iter().map(|n| n.ssid.as_str()).collect();
        assert_eq!(order, ["weak-home", "a", "b", "c", "d"], "rows with the same bars keep still while signal jitters");
        let mut jittered = networks.clone();
        jittered[1].signal = 80;
        assert!(
            glide_topbar::qs_net::same_rows(&networks, &jittered),
            "a signal change inside one bar redraws nothing"
        );
        jittered[4].signal = 40;
        assert!(!glide_topbar::qs_net::same_rows(&networks, &jittered), "losing a bar does");
    }

    #[test]
    fn saved_profiles_are_only_wireless_ones() {
        let saved = vec!["Home".to_string(), "Cafe: Guest".to_string()];
        let mut networks = parse_wifi_list(" :Cafe\\: Guest:WPA2:50:x\n :Other:WPA2:50:x");
        mark_saved(&mut networks, &saved);
        assert!(networks[0].saved);
        assert!(!networks[1].saved);
    }

    #[test]
    fn the_connected_wifi_device_is_found_among_others() {
        let text = "*:Home:wlan0\n*:Cafe\\: Guest:wlan1\n";
        assert_eq!(wifi_device_for_ssid(text, "Cafe: Guest").as_deref(), Some("wlan1"));
        assert_eq!(wifi_device_for_ssid(text, "Other"), None);
    }

    #[test]
    fn signal_strength_maps_onto_four_bars_and_an_icon() {
        let network = |signal: u8| WifiNetwork {
            ssid: String::new(),
            security: Security::Open,
            signal,
            connected: false,
            saved: false,
        };
        assert_eq!(network(0).icon(), "network-wireless-signal-none");
        assert_eq!(network(10).level(), 1);
        assert_eq!(network(49).level(), 2);
        assert_eq!(network(50).level(), 3);
        assert_eq!(network(100).icon(), "network-wireless-signal-excellent");
    }

    #[test]
    fn paired_devices_parse_with_names_containing_spaces() {
        let text = "Device 11:22:33:44:55:66 WH-1000XM4 Headphones\nDevice AA:BB:CC:DD:EE:FF\nnoise line\n";
        let devices = parse_bt_devices(text);
        assert_eq!(
            devices,
            [
                ("11:22:33:44:55:66".to_string(), "WH-1000XM4 Headphones".to_string()),
                ("AA:BB:CC:DD:EE:FF".to_string(), "AA:BB:CC:DD:EE:FF".to_string()),
            ]
        );
    }

    #[test]
    fn device_info_reads_connection_icon_and_battery() {
        let text = "Device 11:22:33:44:55:66 (public)\n\tName: WH-1000XM4\n\tAlias: My Phones\n\tIcon: audio-headset\n\tPaired: yes\n\tConnected: yes\n\tBattery Percentage: 0x50 (80)\n";
        let info = parse_bt_info(text);
        assert_eq!(info.name.as_deref(), Some("My Phones"));
        assert!(info.connected);
        assert_eq!(info.battery, Some(80));
        let device = device_from("11:22:33:44:55:66".into(), "listed".into(), &info);
        assert_eq!(device.kind, BtKind::Headphones);
        assert_eq!(device.kind.icon(), "audio-headphones");
        assert_eq!(parse_bt_info("\tConnected: no\n"), BtInfo::default());
    }

    #[test]
    fn device_icons_fall_back_to_bluetooth() {
        assert_eq!(BtKind::from_icon("input-mouse"), BtKind::Mouse);
        assert_eq!(BtKind::from_icon("input-keyboard"), BtKind::Keyboard);
        assert_eq!(BtKind::from_icon("phone"), BtKind::Phone);
        assert_eq!(BtKind::from_icon("input-gaming").icon(), "bluetooth");
    }

    #[test]
    fn connected_devices_sort_first_then_by_name() {
        let device = |name: &str, connected: bool| BtDevice {
            mac: String::new(),
            name: name.to_string(),
            kind: BtKind::Other,
            connected,
            battery: None,
        };
        let mut devices = vec![device("zeta", false), device("Mouse", true), device("alpha", false)];
        sort_devices(&mut devices);
        let order: Vec<&str> = devices.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(order, ["Mouse", "alpha", "zeta"]);
    }

    #[test]
    fn missing_secrets_read_as_a_wrong_password() {
        assert_eq!(
            classify_wifi_failure(
                "Error: Connection activation failed: Secrets were required, but not provided.",
                true
            ),
            ConnectError::WrongPassword
        );
        assert_eq!(classify_wifi_failure("Error: No network with SSID 'x' found.", false), ConnectError::Failed);
    }

    #[test]
    fn bluetoothctl_failures_are_detected_in_its_output() {
        assert!(bluetooth_succeeded(true, "Attempting to connect\nConnection successful\n"));
        assert!(!bluetooth_succeeded(true, "Failed to connect: org.bluez.Error.Failed\n"));
        assert!(!bluetooth_succeeded(false, ""));
    }

    #[test]
    fn a_connect_request_never_prints_its_password() {
        let request = NetRequest::ConnectWifi { ssid: "Home".into(), password: Some("hunter22".into()), saved: false };
        let printed = format!("{request:?}");
        assert!(printed.contains("Home"));
        assert!(!printed.contains("hunter22"));
    }

    #[test]
    fn a_wifi_password_goes_over_stdin_not_argv() {
        assert_eq!(wifi_connect_args("Home", true, false), ["--ask", "dev", "wifi", "connect", "Home"]);
        assert_eq!(wifi_connect_args("Home", true, true), ["--ask", "dev", "wifi", "connect", "Home"]);
        assert_eq!(wifi_connect_args("Home", false, true), ["con", "up", "id", "Home"]);
        let output = run_with_input("cat", &[], Some("hunter22"), Duration::from_secs(5)).expect("cat runs");
        assert_eq!(output.stdout, b"hunter22\n");
    }

    const SINKS_JSON: &str = r#"[
        {"index":56,"state":"RUNNING","name":"alsa_output.pci-0000_00_1f.3.analog-stereo",
         "description":"Built-in Audio Analog Stereo","active_port":"analog-output-speaker",
         "properties":{"device.bus":"pci","device.form_factor":"internal"}},
        {"index":71,"state":"SUSPENDED","name":"alsa_output.pci-0000_01_00.1.hdmi-stereo",
         "description":"HDA NVidia Digital Stereo (HDMI)","active_port":"hdmi-output-0","properties":{}},
        {"index":90,"state":"IDLE","name":"bluez_output.AA_BB.1","description":"WH-1000XM4",
         "active_port":null,"properties":{"device.bus":"bluetooth","device.form_factor":"headphone"}}
    ]"#;

    #[test]
    fn pactl_json_sinks_keep_their_names_descriptions_and_kind() {
        let mut outputs = parse_sinks_json(SINKS_JSON, "bluez_output.AA_BB.1");
        sort_outputs(&mut outputs);
        let summary: Vec<(&str, &str, bool)> =
            outputs.iter().map(|output| (output.description.as_str(), output.icon, output.active)).collect();
        assert_eq!(
            summary,
            [
                ("WH-1000XM4", "audio-headphones", true),
                ("Built-in Audio Analog Stereo", "audio-speakers", false),
                ("HDA NVidia Digital Stereo (HDMI)", "video-display", false),
            ]
        );
        assert_eq!(outputs[0].name, "bluez_output.AA_BB.1");
        assert!(parse_sinks_json("not json", "").is_empty());
        assert!(parse_sinks_json("{}", "").is_empty());
    }

    const SOURCES_JSON: &str = r#"[
        {"index":51,"name":"alsa_output.pci.HiFi__Speaker__sink.monitor","description":"Monitor of Speaker",
         "monitor_source":"alsa_output.pci.HiFi__Speaker__sink","active_port":"[Out] Speaker",
         "properties":{"device.bus":"pci","device.class":"monitor"}},
        {"index":52,"name":"alsa_input.pci.HiFi__Mic1__source","description":"Digital Microphone",
         "monitor_source":"","active_port":"[In] Mic1","properties":{"device.bus":"pci","device.class":"sound"}},
        {"index":60,"name":"bluez_input.AA_BB.0","description":"WH-1000XM4","monitor_source":"",
         "active_port":null,"properties":{"device.bus":"bluetooth","device.form_factor":"headset"}},
        {"index":61,"name":"v4l2_input.cam","description":"HD Webcam","monitor_source":"",
         "properties":{"device.form_factor":"webcam"}}
    ]"#;

    #[test]
    fn pactl_json_sources_leave_out_the_monitors() {
        let mut inputs = parse_sources_json(SOURCES_JSON, "alsa_input.pci.HiFi__Mic1__source");
        sort_outputs(&mut inputs);
        let summary: Vec<(&str, &str, bool)> =
            inputs.iter().map(|input| (input.description.as_str(), input.icon, input.active)).collect();
        assert_eq!(
            summary,
            [
                ("Digital Microphone", "audio-input-microphone", true),
                ("HD Webcam", "camera-web", false),
                ("WH-1000XM4", "audio-headset", false),
            ]
        );
        let short = "51\tspeaker.monitor\tPipeWire\ts32le\tSUSPENDED\n52\tmic\tPipeWire\ts32le\tSUSPENDED\n";
        let inputs = parse_sources_short(short, "mic");
        assert_eq!(inputs.len(), 1);
        assert!(inputs[0].active);
        assert!(matches!(use_device(crate::topbar::Drawer::Input, "mic".into()), NetRequest::UseInput { .. }));
    }

    #[test]
    fn pages_without_a_list_to_fetch_scan_nothing() {
        for detail in [Detail::Battery, Detail::Power] {
            assert!(scan_for(detail).is_none(), "{detail:?}");
        }
        assert!(matches!(scan_for(Detail::Wifi), Some(NetRequest::RescanWifi)), "the page asks for a real scan");
        assert!(matches!(scan_for(Detail::Vpn), Some(NetRequest::ScanVpn)));
    }

    #[test]
    fn the_wifi_page_scans_back_to_back_on_mains_and_less_often_on_battery() {
        assert_eq!(rescan_every(Detail::Wifi, false), WIFI_SCAN_SETTLE, "the next scan starts as the last settles");
        assert!(rescan_every(Detail::Wifi, true) > rescan_every(Detail::Wifi, false));
        assert_eq!(rescan_every(Detail::Bluetooth, true), RESCAN_INTERVAL);
        assert!(PAGE_TICK < WIFI_SCAN_SETTLE, "the cached list is read between scans");
    }

    #[test]
    fn the_connected_network_names_its_device_band_and_rate() {
        let text = " :wlan0:2437 MHz:54 Mbit/s\n*:wlan0:5180 MHz:866 Mbit/s\n";
        assert_eq!(parse_connected_wifi(text), Some(("wlan0".to_string(), 5180, 866)));
        assert_eq!(parse_connected_wifi(" :wlan0:2437 MHz:54 Mbit/s\n"), None);
    }

    #[test]
    fn vpn_connections_are_the_vpn_and_wireguard_profiles_active_first() {
        let text = "Home:802-11-wireless:yes\nOffice:wireguard:no\nProton\\: CH:vpn:yes\nLAN:802-3-ethernet:yes\n";
        let vpns = parse_vpn_connections(text);
        assert_eq!(
            vpns,
            [
                VpnConnection { name: "Proton: CH".into(), kind: VpnKind::Vpn, active: true },
                VpnConnection { name: "Office".into(), kind: VpnKind::WireGuard, active: false },
            ]
        );
    }

    #[test]
    fn short_sink_lines_fall_back_to_the_sink_name() {
        let text = "56\talsa_output.pci.analog-stereo\tPipeWire\ts32le 2ch 48000Hz\tRUNNING\n\
                    71\talsa_output.pci.hdmi-stereo\tPipeWire\ts32le 2ch 48000Hz\tSUSPENDED\n";
        let outputs = parse_sinks_short(text, "alsa_output.pci.analog-stereo");
        assert_eq!(outputs.len(), 2);
        assert!(outputs[0].active && !outputs[1].active);
        assert_eq!(outputs[1].description, "alsa_output.pci.hdmi-stereo");
        assert_eq!(outputs[1].icon, "video-display");
    }

    #[test]
    fn output_icons_prefer_the_form_factor_over_the_port() {
        assert_eq!(output_icon(Some("headset"), None, "analog-output-speaker"), "audio-headset");
        assert_eq!(output_icon(Some("speaker"), Some("bluetooth"), ""), "audio-speakers");
        assert_eq!(output_icon(None, None, "analog-output-headphones"), "audio-headphones");
        assert_eq!(output_icon(None, Some("bluetooth"), ""), "audio-headphones");
        assert_eq!(output_icon(None, None, "analog-output-lineout"), "audio-speakers");
    }

    #[test]
    fn the_sound_panel_scans_outputs_and_a_switch_request_names_its_sink() {
        assert!(matches!(scan_for(Detail::Sound), Some(NetRequest::ScanOutputs)));
        let request = NetRequest::UseOutput { name: "hdmi".into() };
        assert_eq!(format!("{request:?}"), "UseOutput { name: \"hdmi\" }");
    }
}
