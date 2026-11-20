use super::{
    Anim, Canvas, Color, E_OUT, GlyphPainter, LocalRect, Logical, MemoryRenderBufferRenderElement, OpenPopup, Placed,
    Point, QS_PAD, QS_WIDTH, QsRenderContext, Rectangle, Scale, Slot, TextAlign, TextStyle, Topbar, TopbarTheme, band,
    local_rect, motion, push_placed, text_w,
};

use crate::i18n::{Key, t};
use crate::power::{PowerAction, is_destructive};
use crate::qs_net::{
    AudioOutput, BtDevice, ConnectError, NetEvent, Security, VpnConnection, VpnKind, WifiInfo, WifiNetwork,
};
use crate::system_info::{BatteryInfo, BatteryState, PowerProfile};
use glide_shell::field::{CaretMove, TextField};
use glide_theme::shape::{R_ROUND, R1, R2, R3};
use glide_theme::spacing::{SP1, SP2, SP3};
use smithay::input::keyboard::Keysym;

mod netbird;

pub use netbird::{NetbirdPending, NetbirdUpdate, remaining_text};

const DETAIL_BACK: i32 = 28;
const DETAIL_BACK_ICON: i32 = 16;
const DETAIL_HEAD: i32 = 34;
const DETAIL_SWITCH_W: i32 = 44;
const DETAIL_SWITCH_H: i32 = 26;
const DETAIL_SWITCH_THUMB: i32 = 20;
const DETAIL_SWITCH_INSET: i32 = 3;
const MENU_ROW: i32 = 34;
const MENU_ICON: i32 = 16;
const MENU_SEP: i32 = SP1 * 2 + 1;
const CONFIRM_PAD: i32 = SP3;
const CONFIRM_NOTE_LINES: usize = 2;
const BATTERY_HERO: i32 = SP1 + 33 + 16;
const SEGMENTS: i32 = 34;
const SEGMENT_INSET: i32 = 2;
const SEGMENT_ICON: i32 = 14;
const SEGMENT_HINT: i32 = SP2 + 32 + SP1;
const MENU_ITEM: TextStyle = TextStyle { size_px: 13.5, line_height_px: 20.0, weight: 450, ..TextStyle::CAP };
const KEYCAP: TextStyle = TextStyle { size_px: 11.0, line_height_px: 14.0, weight: 450, ..TextStyle::NUM };
const DETAIL_SECTION: i32 = 28;
const DETAIL_CARD: i32 = 48;
const DETAIL_CARD_GAP: i32 = 2;
const DETAIL_GROUP_GAP: i32 = SP2 - DETAIL_CARD_GAP;
const DETAIL_SETTINGS: i32 = 34;
const DETAIL_CHEVRON: i32 = 14;
const CARD_PAD_X: i32 = 10;
const ROW_PAD: i32 = SP2;
const ROW_GAP: i32 = 12;
const ROW_WELL: i32 = 32;
const ROW_GLYPH: i32 = 16;
const ROW_TAIL: i32 = 16;
const EMPTY_WELL: i32 = 40;
const EMPTY_PAD_TOP: i32 = 20;
const EMPTY_PAD_BOTTOM: i32 = 24;
const EMPTY_TEXT_LINES: usize = 2;
const LINK_TEXT: TextStyle = TextStyle { size_px: 13.0, line_height_px: 19.0, weight: 450, ..TextStyle::CAP };
const EMPTY_TEXT: TextStyle = LINK_TEXT;
const INFO_TEXT: TextStyle = TextStyle { size_px: 12.0, line_height_px: 16.0, weight: 450, ..TextStyle::NUM };
const PROMPT_PAD_TOP: i32 = SP1;
const PROMPT_PAD_BOTTOM: i32 = 10;
const PROMPT_FIELD: i32 = 34;
const PROMPT_ERROR: i32 = 16;
const PROMPT_PILL: i32 = 28;
const PROMPT_REVEAL: i32 = 24;
const PROMPT_EYE: i32 = 14;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Detail {
    Wifi,
    Bluetooth,
    Battery,
    Sound,
    Vpn,
    Power,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelAction {
    Open(Detail),
    Back,
    Toggle,
    Row(u16),
    FocusPassword,
    PromptReveal,
    PromptCancel,
    PromptSubmit,
    Forget,
    Disconnect,
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FieldKey {
    Type(char),
    Backspace,
    DeleteWord,
    Delete,
    Caret(CaretMove),
    SelectAll,
    Submit,
    Cancel,
}

impl std::fmt::Debug for FieldKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Type(_) => f.write_str("Type(*)"),
            Self::Backspace => f.write_str("Backspace"),
            Self::DeleteWord => f.write_str("DeleteWord"),
            Self::Delete => f.write_str("Delete"),
            Self::Caret(movement) => write!(f, "Caret({movement:?})"),
            Self::SelectAll => f.write_str("SelectAll"),
            Self::Submit => f.write_str("Submit"),
            Self::Cancel => f.write_str("Cancel"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QsKeyMode {
    Normal,
    Detail,
    Typing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowTarget {
    Network(WifiNetwork),
    Device(BtDevice),
    Output(AudioOutput),
    Netbird,
    Vpn(VpnConnection),
}

#[derive(PartialEq, Eq)]
pub enum PromptOutcome {
    Unchanged,
    Changed,
    Cancelled,
    Submit { ssid: String, password: String },
}

pub const POWER_ROWS: [PowerAction; 5] =
    [PowerAction::Lock, PowerAction::Sleep, PowerAction::LogOut, PowerAction::Restart, PowerAction::ShutDown];

const fn power_row_label(action: PowerAction) -> Key {
    match action {
        PowerAction::Lock => Key::QsLockScreen,
        PowerAction::Sleep => Key::QsSleepRow,
        PowerAction::LogOut => Key::QsLogOutRow,
        PowerAction::Restart => Key::QsRestartRow,
        PowerAction::ShutDown => Key::QsShutDownRow,
    }
}

const fn power_question(action: PowerAction) -> Key {
    match action {
        PowerAction::Restart => Key::QsAskRestartShort,
        PowerAction::ShutDown => Key::QsAskShutDownShort,
        PowerAction::LogOut | PowerAction::Lock | PowerAction::Sleep => Key::QsAskLogOutShort,
    }
}

const fn power_verb(action: PowerAction) -> Key {
    match action {
        PowerAction::Restart => Key::PowerConfirmRestart,
        PowerAction::ShutDown => Key::QsConfirmShutDown,
        PowerAction::LogOut | PowerAction::Lock | PowerAction::Sleep => Key::PowerConfirmLogOut,
    }
}

pub(super) fn apps_note(apps: &[String]) -> String {
    let names = match apps {
        [] => return t(Key::QsNoAppsOpen).to_string(),
        [one] => one.clone(),
        [first, second] => format!("{first} {} {second}", t(Key::QsAnd)),
        [first, second, rest @ ..] => {
            let more = t(Key::QsAndMore).replace("{n}", &rest.len().to_string());
            format!("{first}, {second} {more}")
        }
    };
    t(Key::QsAppsWillClose).replace("{apps}", &names)
}

pub(super) const fn power_icon(action: PowerAction) -> &'static str {
    match action {
        PowerAction::Lock => "system-lock-screen",
        PowerAction::Sleep => "system-suspend",
        PowerAction::Restart => "system-reboot",
        PowerAction::ShutDown => "system-shutdown",
        PowerAction::LogOut => "system-log-out",
    }
}

pub(super) fn power_glyph(action: PowerAction) -> GlyphPainter {
    match action {
        PowerAction::Lock => crate::paint::draw_lock_glyph_over,
        PowerAction::Sleep => |b, w, h, side, _, s, c| crate::paint::draw_moon_glyph_over(b, w, h, side, s, c),
        PowerAction::Restart | PowerAction::ShutDown | PowerAction::LogOut => crate::paint::draw_power_glyph_over,
    }
}

pub(super) const fn more_slot(detail: Detail) -> Slot {
    match detail {
        Detail::Wifi => Slot::QsWifiMore,
        Detail::Bluetooth => Slot::QsBluetoothMore,
        Detail::Battery => Slot::QsBattery,
        Detail::Sound => Slot::QsVolumeMore,
        Detail::Vpn => Slot::QsNetbirdMore,
        Detail::Power => Slot::QsPowerMenu,
    }
}

fn time_line(info: &BatteryInfo) -> String {
    let state = match info.state {
        BatteryState::Charging => Key::TbBatteryCharging,
        BatteryState::Discharging => Key::TbBatteryOnBattery,
        BatteryState::Full => Key::TbBatteryFull,
        BatteryState::NotCharging => Key::TbBatteryNotCharging,
    };
    let eta = info.minutes.and_then(|minutes| {
        let key = match info.state {
            BatteryState::Charging => Key::TbBatteryTimeToFull,
            BatteryState::Discharging => Key::TbBatteryTimeLeft,
            _ => return None,
        };
        Some(t(key).replace("{}", &crate::i18n::duration_label(minutes)))
    });
    match eta {
        Some(eta) => format!("{} \u{b7} {eta}", t(state)),
        None => t(state).to_string(),
    }
}

fn battery_state_line(status: &crate::system_info::SystemStatus) -> String {
    status.battery.as_ref().map_or_else(
        || t(if status.battery_charging { Key::TbBatteryCharging } else { Key::TbBatteryOnBattery }).to_string(),
        time_line,
    )
}

pub(super) fn battery_head_line(status: &crate::system_info::SystemStatus) -> String {
    let Some(info) = status.battery.as_ref() else {
        return battery_state_line(status);
    };
    let estimate = |key: Key| info.minutes.map(|minutes| t(key).replace("{}", &crate::i18n::duration_label(minutes)));
    let line = match info.state {
        BatteryState::Discharging => estimate(Key::TbBatteryTimeLeft),
        BatteryState::Charging => estimate(Key::TbBatteryTimeToFull),
        BatteryState::Full | BatteryState::NotCharging => None,
    };
    line.unwrap_or_else(|| time_line(info))
}

pub fn field_key(
    ctrl_only: bool, plain: bool, typed: Option<char>, pressed: impl Fn(Keysym) -> bool,
) -> Option<FieldKey> {
    if pressed(Keysym::Escape) {
        return Some(FieldKey::Cancel);
    }
    if pressed(Keysym::Return) || pressed(Keysym::KP_Enter) {
        return Some(FieldKey::Submit);
    }
    if pressed(Keysym::BackSpace) {
        return Some(if ctrl_only { FieldKey::DeleteWord } else { FieldKey::Backspace });
    }
    if pressed(Keysym::Delete) {
        return Some(FieldKey::Delete);
    }
    let caret = [
        (Keysym::Left, CaretMove::Left, CaretMove::WordLeft),
        (Keysym::Right, CaretMove::Right, CaretMove::WordRight),
        (Keysym::Home, CaretMove::Home, CaretMove::Home),
        (Keysym::End, CaretMove::End, CaretMove::End),
    ];
    for (sym, plain_move, word_move) in caret {
        if pressed(sym) {
            return Some(FieldKey::Caret(if ctrl_only { word_move } else { plain_move }));
        }
    }
    if ctrl_only && (pressed(Keysym::a) || pressed(Keysym::A)) {
        return Some(FieldKey::SelectAll);
    }
    if !plain {
        return None;
    }
    typed.filter(|ch| !ch.is_control()).map(FieldKey::Type)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Pending {
    Connect(String),
    Disconnect(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Failure {
    key: String,
    error: ConnectError,
}

struct Prompt {
    ssid: String,
    field: TextField,
    caps_lock: bool,
    reveal: bool,
}

impl Prompt {
    fn new(ssid: String) -> Self {
        Self { ssid, field: TextField::default(), caps_lock: false, reveal: false }
    }

    fn shown(&self) -> String {
        if self.reveal { self.field.text().to_string() } else { masked(self.field.text()) }
    }
}

impl std::fmt::Debug for Prompt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Prompt")
            .field("ssid", &self.ssid)
            .field("len", &self.field.text().chars().count())
            .field("caps_lock", &self.caps_lock)
            .field("reveal", &self.reveal)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowStatus {
    Idle,
    Connecting,
    Disconnecting,
    Failed(ConnectError),
}

#[derive(Debug, Default)]
pub(super) struct DetailPanel {
    pub(super) view: Option<Detail>,
    pub(super) shown: Option<Detail>,
    pub(super) slide: Anim,
    wifi: Vec<WifiNetwork>,
    wifi_ready: bool,
    devices: Vec<BtDevice>,
    devices_ready: bool,
    outputs: Vec<AudioOutput>,
    outputs_ready: bool,
    inputs: Option<Vec<AudioOutput>>,
    vpn_list: Option<Vec<VpnConnection>>,
    wifi_info: Option<WifiInfo>,
    expanded: Option<String>,
    sound_scroll: usize,
    list_scroll: usize,
    pending: Option<Pending>,
    failure: Option<Failure>,
    prompt: Option<Prompt>,
    pub(super) armed: Option<PowerAction>,
    pub(super) apps: Vec<String>,
    pub(super) netbird: netbird::NetbirdView,
    pub(super) rev: u64,
}

impl DetailPanel {
    pub(super) const fn bump(&mut self) {
        self.rev = self.rev.wrapping_add(1);
    }

    pub(super) fn vpns(&self) -> &[VpnConnection] {
        self.vpn_list.as_deref().unwrap_or_default()
    }

    fn status_for(&self, key: &str) -> RowStatus {
        match &self.pending {
            Some(Pending::Connect(pending)) if pending == key => return RowStatus::Connecting,
            Some(Pending::Disconnect(pending)) if pending == key => {
                return RowStatus::Disconnecting;
            }
            _ => {}
        }
        match &self.failure {
            Some(failure) if failure.key == key => RowStatus::Failed(failure.error),
            _ => RowStatus::Idle,
        }
    }

    pub(super) fn slide_progress(&self) -> f64 {
        self.slide.value().clamp(0.0, 1.0)
    }

    pub(super) fn is_animating(&self) -> bool {
        self.slide.is_animating()
    }

    pub(super) fn reset(&mut self) {
        self.view = None;
        self.slide.jump(0.0);
        self.prompt = None;
        self.expanded = None;
        self.failure = None;
        self.bump();
    }

    pub(super) fn drop_prompt(&mut self) {
        if self.prompt.take().is_some() {
            self.bump();
        }
    }

    pub(super) fn collapse(&mut self) -> bool {
        let collapsed = self.prompt.take().is_some() | self.expanded.take().is_some();
        if collapsed {
            self.bump();
        }
        collapsed
    }

    pub(super) fn connected_device_names(&self) -> Vec<&str> {
        self.devices.iter().filter(|device| device.connected).map(|device| device.name.as_str()).collect()
    }

    fn failed(&self, index: usize) -> bool {
        let Some(network) = self.wifi.get(index) else {
            return false;
        };
        self.failure.as_ref().is_some_and(|failure| failure.key == network.ssid)
    }

    pub(super) const fn devices_ready(&self, drawer: super::Drawer) -> bool {
        match drawer {
            super::Drawer::Output => self.outputs_ready,
            super::Drawer::Input => self.inputs.is_some(),
        }
    }

    pub(super) fn devices(&self, drawer: super::Drawer) -> &[AudioOutput] {
        match drawer {
            super::Drawer::Output => &self.outputs,
            super::Drawer::Input => self.inputs.as_deref().unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmptyIcon {
    Airplane,
    WifiOff,
    Wifi,
    Bluetooth,
    Vpn,
}

impl EmptyIcon {
    fn symbol(self) -> (&'static str, GlyphPainter) {
        match self {
            Self::Airplane => ("airplane-mode", crate::paint::draw_wifi_waves_glyph_over),
            Self::WifiOff => ("network-wireless-offline", crate::paint::draw_wifi_waves_glyph_over),
            Self::Wifi => ("network-wireless", crate::paint::draw_wifi_waves_glyph_over),
            Self::Bluetooth => ("bluetooth-disabled", crate::paint::draw_bluetooth_glyph_over),
            Self::Vpn => ("network-vpn", crate::paint::draw_lock_glyph_over),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Empty {
    icon: EmptyIcon,
    title: Key,
    text: Option<Key>,
}

impl Empty {
    const fn wifi_off(airplane: bool) -> Self {
        if airplane {
            Self { icon: EmptyIcon::Airplane, title: Key::QsAirplaneOnTitle, text: Some(Key::QsAirplaneOnText) }
        } else {
            Self { icon: EmptyIcon::WifiOff, title: Key::QsWifiOffTitle, text: Some(Key::QsWifiOffText) }
        }
    }

    const fn bluetooth_off(airplane: bool) -> Self {
        let title = if airplane { Key::QsAirplaneOnTitle } else { Key::QsBluetoothOffTitle };
        Self { icon: EmptyIcon::Bluetooth, title, text: Some(Key::QsBluetoothOffText) }
    }

    const fn note(icon: EmptyIcon, title: Key) -> Self {
        Self { icon, title, text: None }
    }
}

fn empty_height(painter: Option<&crate::paint::Painter>, empty: Empty, width: i32) -> i32 {
    let lines = match (painter, empty.text) {
        (_, None) => 0,
        (None, Some(_)) => EMPTY_TEXT_LINES,
        (Some(painter), Some(text)) => {
            crate::notifications::wrap_lines(painter, t(text), &EMPTY_TEXT, width - SP3 * 2, EMPTY_TEXT_LINES).len()
        }
    };
    let text = i32::try_from(lines).unwrap_or(2) * EMPTY_TEXT.line_height_px as i32;
    let gap = if lines > 0 { SP1 } else { 0 };
    EMPTY_PAD_TOP + EMPTY_WELL + SP2 + SP1 + super::QS_TITLE.line_height_px as i32 + gap + text + EMPTY_PAD_BOTTOM
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ItemKind {
    Section(Key),
    Message(Key),
    Empty(Empty),
    Ethernet,
    Separator,
    Airplane,
    VpnNetbird,
    Vpn(usize),
    MicSlider,
    Network(usize),
    Info(usize),
    Device(usize),
    Output(usize),
    Input(usize),
    Prompt(usize),
    BatteryHero,
    Segments,
    Segment(PowerProfile),
    SegmentsHint,
    PowerRow(PowerAction),
    PowerConfirm(PowerAction),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DetailItem {
    pub(super) kind: ItemKind,
    pub(super) rect: LocalRect,
    pub(super) slot: Option<Slot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DetailGeometry {
    pub(super) back: LocalRect,
    pub(super) toggle: LocalRect,
    pub(super) title_left: i32,
    pub(super) title_right: i32,
    pub(super) body: LocalRect,
    pub(super) divider_bottom: i32,
    pub(super) settings: LocalRect,
}

pub(super) fn detail_geometry(height: i32, radio: bool, link: bool) -> DetailGeometry {
    let row_w = QS_WIDTH - QS_PAD * 2;
    let back = local_rect(QS_PAD, QS_PAD + (DETAIL_HEAD - DETAIL_BACK) / 2, DETAIL_BACK, DETAIL_BACK);
    let toggle = local_rect(
        QS_WIDTH - QS_PAD - DETAIL_SWITCH_W,
        QS_PAD + (DETAIL_HEAD - DETAIL_SWITCH_H) / 2,
        DETAIL_SWITCH_W,
        DETAIL_SWITCH_H,
    );
    let trailing = if radio { DETAIL_SWITCH_W } else { super::QS_READOUT_W };
    let body_top = QS_PAD + DETAIL_HEAD + SP2;
    let settings = local_rect(QS_PAD, height - QS_PAD - DETAIL_SETTINGS, row_w, DETAIL_SETTINGS);
    let divider_bottom = if link { settings.loc.y - SP2 - 1 } else { height - QS_PAD + SP1 };
    let body = local_rect(QS_PAD, body_top, row_w, (divider_bottom - SP1 - body_top).max(0));
    DetailGeometry {
        back,
        toggle,
        title_left: back.loc.x + back.size.w + SP2,
        title_right: QS_WIDTH - QS_PAD - trailing - SP2,
        body,
        divider_bottom,
        settings,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PromptParts {
    pub(super) field: LocalRect,
    pub(super) reveal: LocalRect,
    pub(super) error: Option<LocalRect>,
    pub(super) cancel: LocalRect,
    pub(super) connect: LocalRect,
}

const ACCORDION_INDENT: i32 = ROW_PAD + ROW_WELL + ROW_GAP;

const fn prompt_height(failed: bool) -> i32 {
    let error = if failed { PROMPT_ERROR + SP2 } else { 0 };
    PROMPT_PAD_TOP + PROMPT_FIELD + SP2 + error + PROMPT_PILL + PROMPT_PAD_BOTTOM
}

pub(super) fn prompt_parts(rect: LocalRect, failed: bool, (cancel_w, connect_w): (i32, i32)) -> PromptParts {
    let (left, right) = (rect.loc.x + ACCORDION_INDENT, rect.loc.x + rect.size.w - ROW_PAD);
    let width = right - left;
    let mut y = rect.loc.y + PROMPT_PAD_TOP;
    let field = local_rect(left, y, width, PROMPT_FIELD);
    let reveal_x = field.loc.x + field.size.w - 3 - PROMPT_REVEAL;
    let reveal = local_rect(reveal_x, y + (PROMPT_FIELD - PROMPT_REVEAL) / 2, PROMPT_REVEAL, PROMPT_REVEAL);
    y += PROMPT_FIELD + SP2;
    let error = failed.then(|| {
        let error = local_rect(left, y, width, PROMPT_ERROR);
        y += PROMPT_ERROR + SP2;
        error
    });
    let connect = local_rect(right - connect_w, y, connect_w, PROMPT_PILL);
    let cancel = local_rect(connect.loc.x - SP2 - cancel_w, y, cancel_w, PROMPT_PILL);
    PromptParts { field, reveal, error, cancel, connect }
}

pub(super) struct InfoParts {
    pub(super) line: LocalRect,
    pub(super) forget: LocalRect,
    pub(super) disconnect: LocalRect,
}

const INFO_HEIGHT: i32 = PROMPT_PAD_TOP + PROMPT_ERROR + SP2 + PROMPT_PILL + PROMPT_PAD_BOTTOM;

pub(super) fn info_parts(rect: LocalRect, (forget_w, disconnect_w): (i32, i32)) -> InfoParts {
    let (left, right) = (rect.loc.x + ACCORDION_INDENT, rect.loc.x + rect.size.w - ROW_PAD);
    let y = rect.loc.y + PROMPT_PAD_TOP;
    let line = local_rect(left, y, right - left, PROMPT_ERROR);
    let buttons = y + PROMPT_ERROR + SP2;
    let disconnect = local_rect(right - disconnect_w, buttons, disconnect_w, PROMPT_PILL);
    let forget = local_rect(disconnect.loc.x - SP2 - forget_w, buttons, forget_w, PROMPT_PILL);
    InfoParts { line, forget, disconnect }
}

struct Column<'a> {
    left: i32,
    width: i32,
    y: i32,
    bottom: i32,
    items: &'a mut Vec<DetailItem>,
}

impl Column<'_> {
    const fn fits(&self, height: i32) -> bool {
        self.y + height <= self.bottom
    }

    fn push(&mut self, kind: ItemKind, height: i32, slot: Option<Slot>) -> bool {
        if !self.fits(height) {
            return false;
        }
        self.items.push(DetailItem { kind, rect: local_rect(self.left, self.y, self.width, height), slot });
        self.y += height;
        true
    }

    fn section(&mut self, key: Key) -> bool {
        self.push(ItemKind::Section(key), DETAIL_SECTION, None)
    }

    fn card(&mut self, kind: ItemKind, slot: Option<Slot>) -> bool {
        let pushed = self.push(kind, DETAIL_CARD, slot);
        if pushed {
            self.y += DETAIL_CARD_GAP;
        }
        pushed
    }

    fn message(&mut self, key: Key) {
        self.card(ItemKind::Message(key), None);
    }

    fn empty(&mut self, empty: Empty, painter: Option<&crate::paint::Painter>) {
        let height = empty_height(painter, empty, self.width);
        self.push(ItemKind::Empty(empty), height, None);
    }

    const fn end_group(&mut self) {
        self.y += DETAIL_GROUP_GAP;
    }
}

fn row_slot(index: usize) -> Option<Slot> {
    u16::try_from(index).ok().map(Slot::QsDetailRow)
}

fn push_network(column: &mut Column<'_>, panel: &DetailPanel, index: usize) -> bool {
    let Some(network) = panel.wifi.get(index) else {
        return false;
    };
    let prompt = panel.prompt.as_ref().filter(|prompt| prompt.ssid == network.ssid);
    let failed = panel.failure.as_ref().is_some_and(|failure| failure.key == network.ssid);
    let info = network.connected && panel.expanded.as_deref() == Some(network.ssid.as_str());
    let (kind, extra) = match (prompt, info) {
        (Some(_), _) => (Some(ItemKind::Prompt(index)), prompt_height(failed)),
        (None, true) => (Some(ItemKind::Info(index)), INFO_HEIGHT),
        (None, false) => (None, 0),
    };
    if !column.fits(DETAIL_CARD + extra) {
        return false;
    }
    column.card(ItemKind::Network(index), row_slot(index));
    if let Some(kind) = kind {
        column.y -= DETAIL_CARD_GAP;
        column.push(kind, extra, None);
        column.y += DETAIL_CARD_GAP;
    }
    true
}

const AIRPLANE_BLOCK: i32 = MENU_SEP + DETAIL_CARD;
const PAGE_PROBE: i32 = 4096;

fn wifi_items(
    column: &mut Column<'_>, panel: &DetailPanel, status: &crate::system_info::SystemStatus,
    painter: Option<&crate::paint::Painter>,
) {
    if status.ethernet_available {
        column.section(Key::QsWired);
        column.card(ItemKind::Ethernet, None);
        column.end_group();
    }
    if !status.wifi_available {
        return;
    }
    let bottom = column.bottom;
    column.bottom -= AIRPLANE_BLOCK;
    wifi_list(column, panel, status, painter);
    column.bottom = bottom;
    column.push(ItemKind::Separator, MENU_SEP, None);
    column.push(ItemKind::Airplane, DETAIL_CARD, Some(Slot::QsAirplane));
}

fn wifi_list(
    column: &mut Column<'_>, panel: &DetailPanel, status: &crate::system_info::SystemStatus,
    painter: Option<&crate::paint::Painter>,
) {
    if !status.wifi_enabled || status.airplane {
        return column.empty(Empty::wifi_off(status.airplane), painter);
    }
    if !column.section(Key::WiFi) {
        return;
    }
    if panel.wifi.is_empty() {
        let title = if panel.wifi_ready { Key::QsNoNetworks } else { Key::QsSearching };
        return column.empty(Empty::note(EmptyIcon::Wifi, title), painter);
    }
    let (connected, available): (Vec<usize>, Vec<usize>) =
        (0..panel.wifi.len()).partition(|index| panel.wifi[*index].connected);
    let skipped = panel.list_scroll.min(panel.wifi.len().saturating_sub(1));
    for index in connected.into_iter().chain(available).skip(skipped) {
        if !push_network(column, panel, index) {
            return;
        }
    }
}

fn bluetooth_items(
    column: &mut Column<'_>, panel: &DetailPanel, status: &crate::system_info::SystemStatus,
    painter: Option<&crate::paint::Painter>,
) {
    if !status.bluetooth_on {
        return column.empty(Empty::bluetooth_off(status.airplane), painter);
    }
    if !column.section(Key::QsDevicesSection) {
        return;
    }
    if panel.devices.is_empty() {
        let title = if panel.devices_ready { Key::QsNoDevices } else { Key::QsSearching };
        return column.empty(Empty::note(EmptyIcon::Bluetooth, title), painter);
    }
    let (connected, paired): (Vec<usize>, Vec<usize>) =
        (0..panel.devices.len()).partition(|index| panel.devices[*index].connected);
    let skipped = panel.list_scroll.min(panel.devices.len().saturating_sub(1));
    for index in connected.into_iter().chain(paired).skip(skipped) {
        if !column.card(ItemKind::Device(index), row_slot(index)) {
            return;
        }
    }
}

fn sound_items(column: &mut Column<'_>, panel: &DetailPanel, mic: bool) {
    let count = panel.outputs.len() + panel.inputs.as_ref().map_or(0, Vec::len);
    let mut skipped = panel.sound_scroll.min(count.saturating_sub(1));
    let groups = [
        (Key::QsOutputSection, super::Drawer::Output, Key::QsNoOutputs),
        (Key::QsInputSection, super::Drawer::Input, Key::QsNoInputs),
    ];
    for (section, drawer, empty) in groups {
        let devices = panel.devices(drawer);
        if skipped >= devices.len() && !devices.is_empty() {
            skipped -= devices.len();
            continue;
        }
        if !column.section(section) {
            return;
        }
        if devices.is_empty() {
            column.message(if panel.devices_ready(drawer) { empty } else { Key::QsSearching });
        }
        for index in skipped..devices.len() {
            let (kind, slot) = match drawer {
                super::Drawer::Output => (ItemKind::Output(index), row_slot(index)),
                super::Drawer::Input => {
                    (ItemKind::Input(index), u16::try_from(index).ok().map(|i| Slot::QsDrawerItem(drawer, i)))
                }
            };
            if !column.card(kind, slot) {
                return;
            }
        }
        skipped = 0;
        column.end_group();
    }
    if mic {
        column.y += SP1 - DETAIL_GROUP_GAP - DETAIL_CARD_GAP;
        let row = super::qs_slider_row(column.left, column.y, column.width, true);
        if column.push(ItemKind::MicSlider, super::QS_ROW, None) {
            column.items.push(DetailItem { kind: ItemKind::MicSlider, rect: row.icon, slot: Some(Slot::QsMicMute) });
            column.items.push(DetailItem { kind: ItemKind::MicSlider, rect: row.track, slot: Some(Slot::QsMicTrack) });
        }
    }
}

fn battery_items(column: &mut Column<'_>, status: &crate::system_info::SystemStatus) {
    if status.battery_pct.is_some() {
        column.push(ItemKind::BatteryHero, BATTERY_HERO, None);
    }
    if status.power_profile.is_some() && column.section(Key::QsPowerMode) {
        let at = column.y;
        if column.push(ItemKind::Segments, SEGMENTS, None) {
            for (profile, rect) in segment_rects(local_rect(column.left, at, column.width, SEGMENTS)) {
                let slot = Some(Slot::QsProfile(profile));
                column.items.push(DetailItem { kind: ItemKind::Segment(profile), rect, slot });
            }
            column.push(ItemKind::SegmentsHint, SEGMENT_HINT, None);
        }
    }
}

pub(super) fn segment_rects(track: LocalRect) -> [(PowerProfile, LocalRect); 3] {
    let inner = track.size.w - SEGMENT_INSET * 2;
    let edge = |index: i32| track.loc.x + SEGMENT_INSET + inner * index / 3;
    let mut index = 0;
    super::QS_PROFILES.map(|profile| {
        let (left, right) = (edge(index), edge(index + 1));
        index += 1;
        (profile, local_rect(left, track.loc.y + SEGMENT_INSET, right - left, track.size.h - SEGMENT_INSET * 2))
    })
}

pub(super) fn confirm_height(painter: Option<&crate::paint::Painter>, apps: &[String], width: i32) -> i32 {
    let note_lines = painter.map_or(CONFIRM_NOTE_LINES, |painter| {
        let text = apps_note(apps);
        crate::notifications::wrap_lines(painter, &text, &super::QS_NOTE, width - CONFIRM_PAD * 2, CONFIRM_NOTE_LINES)
            .len()
            .max(1)
    });
    let note = i32::try_from(note_lines).unwrap_or(2) * super::QS_NOTE.line_height_px as i32;
    SP1 / 2 + CONFIRM_PAD * 2 + super::QS_TITLE.line_height_px as i32 + SP2 + note + SP2 + super::QS_CONFIRM_PILL + SP2
}

fn power_items(column: &mut Column<'_>, panel: &DetailPanel, painter: Option<&crate::paint::Painter>) {
    for action in POWER_ROWS {
        if action == PowerAction::LogOut {
            column.push(ItemKind::Separator, MENU_SEP, None);
        }
        column.push(ItemKind::PowerRow(action), MENU_ROW, Some(Slot::QsPowerButton(action)));
        if panel.armed == Some(action) && is_destructive(action) {
            let height = confirm_height(painter, &panel.apps, column.width);
            column.push(ItemKind::PowerConfirm(action), height, None);
        }
    }
}

pub(super) fn detail_items(
    panel: &DetailPanel, detail: Detail, status: &crate::system_info::SystemStatus, body: LocalRect,
    painter: Option<&crate::paint::Painter>,
) -> Vec<DetailItem> {
    let mut items = Vec::new();
    let mut column = Column {
        left: body.loc.x,
        width: body.size.w,
        y: body.loc.y,
        bottom: body.loc.y + body.size.h,
        items: &mut items,
    };
    match detail {
        Detail::Wifi => wifi_items(&mut column, panel, status, painter),
        Detail::Bluetooth => bluetooth_items(&mut column, panel, status, painter),
        Detail::Sound => sound_items(&mut column, panel, status.mic_available),
        Detail::Battery => battery_items(&mut column, status),
        Detail::Vpn => vpn_items(&mut column, panel, painter),
        Detail::Power => power_items(&mut column, panel, painter),
    }
    items
}

fn vpn_items(column: &mut Column<'_>, panel: &DetailPanel, painter: Option<&crate::paint::Painter>) {
    let netbird = usize::from(panel.netbird.installed);
    if netbird + panel.vpns().len() == 0 {
        let title = if panel.vpn_list.is_some() { Key::QsNoVpn } else { Key::QsSearching };
        return column.empty(Empty::note(EmptyIcon::Vpn, title), painter);
    }
    if !column.section(Key::QsVpnSection) {
        return;
    }
    if netbird == 1 && !column.card(ItemKind::VpnNetbird, row_slot(0)) {
        return;
    }
    for index in 0..panel.vpns().len() {
        if !column.card(ItemKind::Vpn(index), row_slot(index + netbird)) {
            return;
        }
    }
}

fn radio_on(detail: Detail, _panel: &DetailPanel, status: &crate::system_info::SystemStatus) -> bool {
    match detail {
        Detail::Wifi if status.wifi_available => status.wifi_enabled && !status.airplane,
        Detail::Wifi => status.ethernet_connected,
        Detail::Bluetooth => status.bluetooth_on,
        Detail::Vpn | Detail::Battery | Detail::Sound | Detail::Power => false,
    }
}

const fn has_radio(detail: Detail) -> bool {
    matches!(detail, Detail::Wifi | Detail::Bluetooth)
}

const fn has_link(detail: Detail) -> bool {
    !matches!(detail, Detail::Power)
}

impl Topbar {
    pub fn scroll_detail_list(&mut self, steps: i32) -> bool {
        let count = match self.detail() {
            Some(Detail::Sound) => return self.scroll_sound_devices(steps),
            Some(Detail::Wifi) if self.qs_panel.prompt.is_none() => self.qs_panel.wifi.len(),
            Some(Detail::Bluetooth) => self.qs_panel.devices.len(),
            _ => return false,
        };
        let current = self.qs_panel.list_scroll.min(count.saturating_sub(1));
        let next = current.saturating_add_signed(steps as isize).min(count.saturating_sub(1));
        if next == self.qs_panel.list_scroll {
            return false;
        }
        self.qs_panel.list_scroll = next;
        self.qs_panel.bump();
        self.touch();
        true
    }

    pub fn scroll_sound_devices(&mut self, steps: i32) -> bool {
        if self.detail() != Some(Detail::Sound) {
            return false;
        }
        let count = self.qs_panel.outputs.len() + self.qs_panel.inputs.as_ref().map_or(0, Vec::len);
        let current = self.qs_panel.sound_scroll.min(count.saturating_sub(1));
        let next = current.saturating_add_signed(steps as isize).min(count.saturating_sub(1));
        if next == self.qs_panel.sound_scroll {
            return false;
        }
        self.qs_panel.sound_scroll = next;
        self.qs_panel.bump();
        self.touch();
        true
    }

    #[must_use]
    pub fn detail(&self) -> Option<Detail> {
        if self.open_popup == Some(OpenPopup::QuickSettings) { self.qs_panel.view } else { None }
    }

    pub fn open_detail(&mut self, detail: Detail) -> bool {
        if self.open_popup != Some(OpenPopup::QuickSettings) || self.qs_panel.view == Some(detail) {
            return false;
        }
        let keyboard = self.focused.is_some();
        if detail != Detail::Power {
            self.forget_power();
        }
        self.qs_panel.view = Some(detail);
        self.qs_panel.sound_scroll = 0;
        self.qs_panel.list_scroll = 0;
        self.qs_panel.shown = Some(detail);
        self.qs_panel.slide.retarget(1.0, motion::t2(), E_OUT);
        self.qs_panel.prompt = None;
        self.qs_panel.failure = None;
        self.qs_panel.bump();
        self.set_focused(keyboard.then_some(Slot::QsBack));
        self.touch();
        true
    }

    pub fn close_detail(&mut self) -> bool {
        let Some(detail) = self.qs_panel.view.take() else {
            return false;
        };
        let keyboard = self.focused.is_some();
        self.qs_panel.slide.retarget(0.0, motion::t2(), E_OUT);
        self.qs_panel.prompt = None;
        self.qs_panel.armed = None;
        self.qs_panel.bump();
        self.set_focused(keyboard.then_some(more_slot(detail)));
        self.touch();
        true
    }

    #[must_use]
    pub fn vpn_toggle_target(&self) -> Option<RowTarget> {
        let panel = &self.qs_panel;
        if let Some(active) = panel.vpns().iter().find(|vpn| vpn.active) {
            return Some(RowTarget::Vpn(active.clone()));
        }
        if panel.netbird.installed {
            return Some(RowTarget::Netbird);
        }
        panel.vpns().first().cloned().map(RowTarget::Vpn)
    }

    #[must_use]
    pub fn detail_row(&self, index: u16) -> Option<RowTarget> {
        let index = usize::from(index);
        match self.detail()? {
            Detail::Wifi => self.qs_panel.wifi.get(index).cloned().map(RowTarget::Network),
            Detail::Bluetooth => self.qs_panel.devices.get(index).cloned().map(RowTarget::Device),
            Detail::Sound => self.qs_panel.outputs.get(index).cloned().map(RowTarget::Output),
            Detail::Battery | Detail::Power => None,
            Detail::Vpn => {
                let netbird = usize::from(self.qs_panel.netbird.installed);
                if index < netbird {
                    Some(RowTarget::Netbird)
                } else {
                    self.qs_panel.vpns().get(index - netbird).cloned().map(RowTarget::Vpn)
                }
            }
        }
    }

    fn device_slot_for(&self, drawer: super::Drawer, key: &str) -> Option<Slot> {
        let index = self.qs_panel.devices(drawer).iter().position(|device| device.name == key)?;
        Some(Slot::QsDrawerItem(drawer, u16::try_from(index).ok()?))
    }

    fn row_slot_for(&self, key: &str) -> Option<Slot> {
        let row = |index: usize| u16::try_from(index).ok().map(Slot::QsDetailRow);
        match self.qs_panel.view {
            Some(Detail::Wifi) => row(self.qs_panel.wifi.iter().position(|n| n.ssid == key)?),
            Some(Detail::Bluetooth) => row(self.qs_panel.devices.iter().position(|d| d.mac == key)?),
            Some(Detail::Sound) => self
                .qs_panel
                .outputs
                .iter()
                .position(|o| o.name == key)
                .and_then(row)
                .or_else(|| self.device_slot_for(super::Drawer::Input, key)),
            Some(Detail::Vpn) => {
                let netbird = usize::from(self.qs_panel.netbird.installed);
                row(self.qs_panel.vpns().iter().position(|vpn| vpn.name == key)? + netbird)
            }
            Some(Detail::Battery | Detail::Power) | None => None,
        }
    }

    fn settle_focus_on_row(&mut self, key: &str) {
        if self.focused.is_some()
            && let Some(slot) = self.row_slot_for(key)
        {
            self.set_focused(Some(slot));
        }
    }

    pub fn begin_connect(&mut self, key: &str) {
        self.qs_panel.pending = Some(Pending::Connect(key.to_string()));
        self.qs_panel.failure = None;
        self.qs_panel.prompt = None;
        self.settle_focus_on_row(key);
        self.qs_panel.bump();
        self.touch();
    }

    pub fn begin_disconnect(&mut self, key: &str) {
        self.qs_panel.pending = Some(Pending::Disconnect(key.to_string()));
        self.qs_panel.failure = None;
        self.qs_panel.prompt = None;
        self.qs_panel.bump();
        self.touch();
    }

    pub fn toggle_prompt(&mut self, ssid: &str) {
        if self.qs_panel.prompt.as_ref().is_some_and(|prompt| prompt.ssid == ssid) {
            self.qs_panel.prompt = None;
            self.settle_focus_on_row(ssid);
        } else {
            self.qs_panel.prompt = Some(Prompt::new(ssid.to_string()));
            self.qs_panel.expanded = None;
            self.set_focused(Some(Slot::QsPasswordField));
        }
        self.qs_panel.failure = None;
        self.qs_panel.bump();
        self.touch();
    }

    pub fn toggle_info(&mut self, ssid: &str) {
        if self.qs_panel.expanded.as_deref() == Some(ssid) {
            self.qs_panel.expanded = None;
        } else {
            self.qs_panel.expanded = Some(ssid.to_string());
            self.qs_panel.prompt = None;
        }
        self.qs_panel.bump();
        self.touch();
    }

    #[must_use]
    pub fn expanded_network(&self) -> Option<&str> {
        self.qs_panel.expanded.as_deref()
    }

    pub fn set_caps_lock(&mut self, on: bool) -> bool {
        let Some(prompt) = self.qs_panel.prompt.as_mut() else {
            return false;
        };
        if prompt.caps_lock == on {
            return false;
        }
        prompt.caps_lock = on;
        self.qs_panel.bump();
        true
    }

    pub fn toggle_reveal(&mut self) -> bool {
        let Some(prompt) = self.qs_panel.prompt.as_mut() else {
            return false;
        };
        prompt.reveal = !prompt.reveal;
        self.qs_panel.bump();
        self.touch();
        true
    }

    pub fn focus_password(&mut self) {
        if self.qs_panel.prompt.is_some() && self.focused != Some(Slot::QsPasswordField) {
            self.set_focused(Some(Slot::QsPasswordField));
        }
    }

    pub fn prompt_key(&mut self, key: FieldKey) -> PromptOutcome {
        let Some(prompt) = self.qs_panel.prompt.as_mut() else {
            return PromptOutcome::Unchanged;
        };
        let changed = match key {
            FieldKey::Type(ch) => prompt.field.insert(ch),
            FieldKey::Backspace => prompt.field.backspace(),
            FieldKey::DeleteWord => prompt.field.delete_word_back(),
            FieldKey::Delete => prompt.field.delete(),
            FieldKey::Caret(movement) => prompt.field.move_caret(movement),
            FieldKey::SelectAll => prompt.field.select_all(),
            FieldKey::Submit => {
                if prompt.field.is_empty() {
                    return PromptOutcome::Unchanged;
                }
                let ssid = prompt.ssid.clone();
                let password = prompt.field.text().to_string();
                self.qs_panel.prompt = None;
                self.qs_panel.bump();
                self.touch();
                return PromptOutcome::Submit { ssid, password };
            }
            FieldKey::Cancel => {
                let ssid = prompt.ssid.clone();
                self.qs_panel.prompt = None;
                self.settle_focus_on_row(&ssid);
                self.qs_panel.bump();
                self.touch();
                return PromptOutcome::Cancelled;
            }
        };
        if !changed {
            return PromptOutcome::Unchanged;
        }
        self.qs_panel.bump();
        PromptOutcome::Changed
    }

    #[must_use]
    pub fn qs_key_mode(&self) -> QsKeyMode {
        if self.detail().is_none() {
            return QsKeyMode::Normal;
        }
        if self.qs_panel.prompt.is_some() && self.focused == Some(Slot::QsPasswordField) {
            QsKeyMode::Typing
        } else {
            QsKeyMode::Detail
        }
    }

    fn settle_pending(&mut self, key: &str, result: Result<(), ConnectError>) {
        let matches = matches!(
            &self.qs_panel.pending,
            Some(Pending::Connect(pending) | Pending::Disconnect(pending)) if pending == key
        );
        if matches {
            self.qs_panel.pending = None;
        }
        self.qs_panel.failure = result.err().map(|error| Failure { key: key.to_string(), error });
    }

    pub fn apply_net_event(&mut self, event: NetEvent) -> bool {
        match event {
            NetEvent::Vpn(vpns) => {
                if self.qs_panel.vpn_list.as_ref() == Some(&vpns) {
                    return false;
                }
                self.qs_panel.vpn_list = Some(vpns);
            }
            NetEvent::VpnResult { name, result } => self.settle_pending(&name, result),
            NetEvent::WifiInfo(info) => {
                if self.qs_panel.wifi_info == info {
                    return false;
                }
                self.qs_panel.wifi_info = info;
            }
            NetEvent::Wifi(networks) => {
                if self.qs_panel.wifi_ready && self.qs_panel.wifi == networks {
                    return false;
                }
                self.qs_panel.wifi = networks;
                self.qs_panel.wifi_ready = true;
            }
            NetEvent::Bluetooth(devices) => {
                if self.qs_panel.devices_ready && self.qs_panel.devices == devices {
                    return false;
                }
                self.qs_panel.devices = devices;
                self.qs_panel.devices_ready = true;
            }
            NetEvent::WifiResult { ssid, result } => {
                self.settle_pending(&ssid, result);
                if result == Err(ConnectError::WrongPassword) && self.detail() == Some(Detail::Wifi) {
                    self.qs_panel.prompt = Some(Prompt::new(ssid));
                    self.set_focused(Some(Slot::QsPasswordField));
                }
            }
            NetEvent::BluetoothResult { mac, result } => self.settle_pending(&mac, result),
            NetEvent::Outputs(outputs) => {
                if self.qs_panel.outputs_ready && self.qs_panel.outputs == outputs {
                    return false;
                }
                self.qs_panel.outputs = outputs;
                self.qs_panel.outputs_ready = true;
            }
            NetEvent::OutputResult { name, result } | NetEvent::InputResult { name, result } => {
                self.settle_pending(&name, result);
            }
            NetEvent::Inputs(inputs) => {
                if self.qs_panel.inputs.as_ref() == Some(&inputs) {
                    return false;
                }
                self.qs_panel.inputs = Some(inputs);
            }
        }
        self.qs_panel.bump();
        self.touch();
        true
    }

    pub(super) fn page_height(&self, painter: Option<&crate::paint::Painter>, detail: Detail) -> i32 {
        let probe = detail_geometry(PAGE_PROBE, has_radio(detail), has_link(detail));
        let items = detail_items(&self.qs_panel, detail, &self.system_status, probe.body, painter);
        let bottom = items.iter().map(|item| item.rect.loc.y + item.rect.size.h).max().unwrap_or(probe.body.loc.y);
        let below = PAGE_PROBE - (probe.body.loc.y + probe.body.size.h);
        (bottom + below).min(super::QS_PAGE_MAX)
    }

    pub(super) fn detail_slots(&self, painter: &crate::paint::Painter) -> Option<(i32, Vec<(Slot, LocalRect)>)> {
        let detail = self.qs_panel.view?;
        let radio = has_radio(detail);
        let height = self.page_height(Some(painter), detail);
        let geometry = detail_geometry(height, radio, has_link(detail));
        let mut slots = vec![(Slot::QsBack, geometry.back)];
        if radio {
            slots.push((Slot::QsDetailToggle, geometry.toggle));
        }
        for item in detail_items(&self.qs_panel, detail, &self.system_status, geometry.body, Some(painter)) {
            if let Some(slot) = item.slot {
                slots.push((slot, item.rect));
            }
            if let ItemKind::Prompt(index) = item.kind {
                let parts = prompt_parts(item.rect, self.qs_panel.failed(index), prompt_pills(painter));
                slots.push((Slot::QsPromptReveal, parts.reveal));
                slots.push((Slot::QsPasswordField, parts.field));
                slots.push((Slot::QsPromptCancel, parts.cancel));
                slots.push((Slot::QsPromptConnect, parts.connect));
            }
            if let ItemKind::Info(_) = item.kind {
                let parts = info_parts(item.rect, info_pills(painter));
                slots.push((Slot::QsNetForget, parts.forget));
                slots.push((Slot::QsNetDisconnect, parts.disconnect));
            }
            if let ItemKind::PowerConfirm(action) = item.kind {
                let (cancel, confirm) = confirm_buttons(painter, item.rect, action);
                slots.push((Slot::QsPowerCancel, cancel));
                slots.push((Slot::QsPowerConfirm(action), confirm));
            }
        }
        if has_link(detail) {
            slots.push((Slot::QsDetailSettings, geometry.settings));
        }
        Some((height, slots))
    }
}

fn info_pills(painter: &crate::paint::Painter) -> (i32, i32) {
    (
        super::qs_confirm_pill_width(painter, t(Key::QsForgetNetwork)),
        super::qs_confirm_pill_width(painter, t(Key::QsDisconnectShort)),
    )
}

fn prompt_pills(painter: &crate::paint::Painter) -> (i32, i32) {
    (
        super::qs_confirm_pill_width(painter, t(Key::PowerCancel)),
        super::qs_confirm_pill_width(painter, t(Key::QsConnectShort)),
    )
}

pub(super) type DetailKey = ((u64, u64, u64), Option<Slot>, Detail, i32, u8);

#[allow(clippy::too_many_arguments)]
pub(super) fn push_detail<R>(
    topbar: &mut Topbar, renderer: &mut R, painter: &crate::paint::Painter, icons: &mut crate::icon::IconLoader,
    rect: Rectangle<i32, Logical>, reveal: f64, scale: Scale<f64>, output_scale: i32,
    elements: &mut Vec<MemoryRenderBufferRenderElement<R>>,
) where
    R: crate::raster::ShellRenderer,
{
    let Some(detail) = topbar.qs_panel.shown else {
        return;
    };
    let slide = topbar.qs_panel.slide_progress();
    if slide <= 0.0 {
        return;
    }
    let hovered = topbar.hovered_in(Slot::in_quick_settings).filter(|_| topbar.qs_panel.view.is_some());
    let profile = super::quantize(topbar.profile_anim.value(), super::MOTION_QUANTUM);
    let revs = (topbar.qs_panel.rev, topbar.detail_rev, topbar.status_rev);
    let key: DetailKey = (revs, hovered, detail, rect.size.h, profile);
    let theme = &topbar.theme;
    let status = &topbar.system_status;
    let panel = &topbar.qs_panel;
    let Some(buffer) = topbar.qs_detail_cache.resolve(key, output_scale, || {
        let ctx = QsRenderContext { painter, theme, hovered };
        let mut canvas = Canvas::new(rect.size.w, rect.size.h, output_scale)?.with_icons(Some(icons));
        let position = f64::from(profile) / super::MOTION_QUANTUM;
        draw_detail(&mut canvas, &ctx, panel, detail, status, position);
        Some(canvas.finish())
    }) else {
        return;
    };
    let shift = ((1.0 - slide) * f64::from(QS_PAD)).round() as i32;
    let alpha = slide as f32;
    push_placed(
        renderer,
        buffer,
        Placed {
            popup: rect,
            element: Rectangle::new(Point::from((rect.loc.x + shift, rect.loc.y)), rect.size),
            progress: reveal,
            alpha,
        },
        scale,
        elements,
    );
}

fn confirm_buttons(painter: &crate::paint::Painter, rect: LocalRect, action: PowerAction) -> (LocalRect, LocalRect) {
    let cancel_w = super::qs_confirm_pill_width(painter, t(Key::PowerCancel));
    let confirm_w = super::qs_confirm_pill_width(painter, t(power_verb(action)));
    let right = rect.loc.x + rect.size.w - CONFIRM_PAD;
    let y = rect.loc.y + rect.size.h - SP2 - CONFIRM_PAD - super::QS_CONFIRM_PILL;
    let confirm = local_rect(right - confirm_w, y, confirm_w, super::QS_CONFIRM_PILL);
    let cancel = local_rect(confirm.loc.x - SP2 - cancel_w, y, cancel_w, super::QS_CONFIRM_PILL);
    (cancel, confirm)
}

pub(super) fn draw_detail(
    c: &mut Canvas<'_>, ctx: &QsRenderContext<'_>, panel: &DetailPanel, detail: Detail,
    status: &crate::system_info::SystemStatus, profile: f64,
) {
    let geometry = detail_geometry(c.h, has_radio(detail), has_link(detail));
    draw_detail_header(c, ctx, &geometry, detail, (panel, status));

    for item in detail_items(panel, detail, status, geometry.body, Some(ctx.painter)) {
        draw_item(c, ctx, (panel, status, profile), item);
    }

    if !has_link(detail) {
        return;
    }
    c.fill(local_rect(QS_PAD, geometry.divider_bottom, c.w - QS_PAD * 2, 1), ctx.theme.line, 0.0);
    let settings = geometry.settings;
    let settings_label = t(match detail {
        Detail::Wifi => Key::QsLinkNetwork,
        Detail::Bluetooth => Key::QsLinkBluetooth,
        Detail::Battery | Detail::Power => Key::QsLinkPower,
        Detail::Sound => Key::QsLinkSound,
        Detail::Vpn => Key::QsLinkVpn,
    });
    let (left, right, top, bottom) = band(settings);
    let theme = ctx.theme;
    let icon = local_rect(left + CARD_PAD_X, top + (settings.size.h - MENU_ICON) / 2, MENU_ICON, MENU_ICON);
    super::qs_symbol(c, icon, ("emblem-system", crate::paint::draw_gear_glyph_over), MENU_ICON, theme.muted);
    let ext = local_rect(
        right - CARD_PAD_X - DETAIL_CHEVRON,
        top + (settings.size.h - DETAIL_CHEVRON) / 2,
        DETAIL_CHEVRON,
        DETAIL_CHEVRON,
    );
    if !c.icon("external-link", DETAIL_CHEVRON, (ext.loc.x, ext.loc.y), theme.faint) {
        c.text_in(ctx.painter, ext, TextAlign::Center, "\u{2197}", theme.faint, &TextStyle::CAP);
    }
    let text = (icon.loc.x + MENU_ICON + CARD_PAD_X, ext.loc.x - SP2, top, bottom);
    c.text(ctx.painter, text, TextAlign::Left, settings_label, theme.muted, &LINK_TEXT);
}

fn draw_item(
    c: &mut Canvas<'_>, ctx: &QsRenderContext<'_>,
    (panel, status, profile): (&DetailPanel, &crate::system_info::SystemStatus, f64), item: DetailItem,
) {
    let rect = item.rect;
    match item.kind {
        ItemKind::Section(key) => {
            let label = local_rect(rect.loc.x + SP2, rect.loc.y, rect.size.w - SP2 * 2, rect.size.h);
            c.text_in(ctx.painter, label, TextAlign::Left, t(key), ctx.theme.faint, &TextStyle::MICRO);
        }
        ItemKind::Message(key) => {
            c.text_in(ctx.painter, rect, TextAlign::Center, t(key), ctx.theme.faint, &TextStyle::CAP);
        }
        ItemKind::Empty(empty) => draw_empty(c, ctx, rect, empty),
        ItemKind::Ethernet => draw_ethernet_row(c, ctx, rect, status),
        ItemKind::Separator => {
            c.fill(local_rect(rect.loc.x, rect.loc.y + SP1, rect.size.w, 1), ctx.theme.line, 0.0);
        }
        ItemKind::Airplane => draw_airplane_row(c, ctx, rect, status.airplane),
        ItemKind::Network(index) => {
            if let Some(network) = panel.wifi.get(index) {
                draw_network_row(c, ctx, rect, network, panel.status_for(&network.ssid));
            }
        }
        ItemKind::Prompt(index) => {
            if let Some(prompt) = panel.prompt.as_ref() {
                draw_prompt(c, ctx, rect, prompt, panel.failed(index));
            }
        }
        ItemKind::Info(_) => draw_info(c, ctx, rect, panel.wifi_info.as_ref()),
        ItemKind::Device(index) => {
            if let Some(device) = panel.devices.get(index) {
                draw_device_row(c, ctx, rect, device, panel.status_for(&device.mac));
            }
        }
        ItemKind::Output(index) | ItemKind::Input(index) => {
            let drawer =
                if matches!(item.kind, ItemKind::Output(_)) { super::Drawer::Output } else { super::Drawer::Input };
            if let Some(device) = panel.devices(drawer).get(index) {
                draw_audio_row(c, ctx, rect, device, panel.status_for(&device.name));
            }
        }
        ItemKind::MicSlider if item.slot.is_none() => draw_mic_slider(c, ctx, rect, status),
        ItemKind::BatteryHero => draw_battery_hero(c, ctx, rect, status),
        ItemKind::Segments => draw_segments(c, ctx, rect, (status.power_profile, profile)),
        ItemKind::Segment(_) | ItemKind::MicSlider => {}
        ItemKind::SegmentsHint => {
            if let Some(active) = status.power_profile {
                let lines = crate::notifications::wrap_lines(
                    ctx.painter,
                    t(profile_hint(active)),
                    &super::QS_NOTE,
                    rect.size.w - SP2 * 2,
                    2,
                );
                let line_h = super::QS_NOTE.line_height_px as i32;
                for (index, line) in (0..).zip(lines) {
                    let top = rect.loc.y + SP2 + index * line_h;
                    let band = (rect.loc.x + SP2, rect.loc.x + rect.size.w - SP2, top, top + line_h);
                    c.text(ctx.painter, band, TextAlign::Left, &line, ctx.theme.muted, &super::QS_NOTE);
                }
            }
        }
        ItemKind::PowerRow(action) => draw_power_row(c, ctx, rect, action, panel.armed == Some(action)),
        ItemKind::PowerConfirm(action) => draw_power_confirm(c, ctx, rect, action, &panel.apps),
        ItemKind::VpnNetbird => draw_netbird_row(c, ctx, rect, &panel.netbird),
        ItemKind::Vpn(index) => {
            if let Some(vpn) = panel.vpns().get(index) {
                draw_vpn_row(c, ctx, rect, vpn, panel.status_for(&vpn.name));
            }
        }
    }
}

fn draw_battery_hero(
    c: &mut Canvas<'_>, ctx: &QsRenderContext<'_>, rect: LocalRect, status: &crate::system_info::SystemStatus,
) {
    let Some(pct) = status.battery_pct else {
        return;
    };
    let theme = ctx.theme;
    let low = battery_low(theme, status);
    let top = rect.loc.y + SP1;
    let pct_text = super::qs_percent(pct);
    let left = rect.loc.x + SP2;
    let pct_w = text_w(ctx.painter, &pct_text, &super::QS_BATTERY_PCT);
    let line_h = super::QS_BATTERY_PCT.line_height_px as i32;
    let ink = if low { theme.danger } else { theme.label };
    c.text(
        ctx.painter,
        (left, left + pct_w, top, top + line_h),
        TextAlign::Left,
        &pct_text,
        ink,
        &super::QS_BATTERY_PCT,
    );
    let (w, h) = (super::battery::CARD_W, super::battery::CARD_H);
    let right = rect.loc.x + rect.size.w - SP2;
    let cell = super::battery::Cell::new(right - w, top + (line_h - h) / 2, w, h);
    let fill = if low {
        theme.danger
    } else if status.battery_charging {
        theme.ok
    } else {
        theme.label
    };
    super::battery::draw(
        c,
        &cell,
        super::battery::Look { pct, charging: status.battery_charging, ink: theme.label, fill },
    );
    let text_left = left + pct_w + SP3;
    let text_right = cell.body.loc.x - SP3;
    let name_h = super::QS_TITLE.line_height_px as i32;
    let sub_h = super::QS_NOTE.line_height_px as i32;
    let text_top = top + (line_h - name_h - sub_h) / 2;
    let state = t(if status.battery_charging { Key::TbBatteryCharging } else { Key::TbBatteryOnBattery });
    let name_band = (text_left, text_right, text_top, text_top + name_h);
    c.text(ctx.painter, name_band, TextAlign::Left, state, theme.label, &super::QS_TITLE);
    let line = battery_head_line(status);
    let band = (text_left, text_right, text_top + name_h, text_top + name_h + sub_h);
    c.text(ctx.painter, band, TextAlign::Left, &line, theme.muted, &super::QS_NOTE);
}

fn draw_segments(
    c: &mut Canvas<'_>, ctx: &QsRenderContext<'_>, track: LocalRect, (active, position): (Option<PowerProfile>, f64),
) {
    let theme = ctx.theme;
    c.fill(track, theme.track_soft, R2);
    let segments = segment_rects(track);
    if active.is_some() {
        let (first, second) = (segments[0].1, segments[1].1);
        let step = f64::from(second.loc.x - first.loc.x);
        let x = first.loc.x + (step * position.clamp(0.0, 2.0)).round() as i32;
        c.fill(local_rect(x, first.loc.y, first.size.w, first.size.h), theme.track, R1);
    }
    for (profile, rect) in segments {
        let colour = if active == Some(profile) { theme.label } else { theme.muted };
        let label = t(super::power_profile_label(profile));
        let content = SEGMENT_ICON + SP1 + SP1 / 2 + text_w(ctx.painter, label, &super::QS_PILL_TEXT);
        let x = (rect.loc.x + (rect.size.w - content) / 2).max(rect.loc.x + SP1);
        let icon = local_rect(x, rect.loc.y + (rect.size.h - SEGMENT_ICON) / 2, SEGMENT_ICON, SEGMENT_ICON);
        super::qs_symbol(
            c,
            icon,
            (super::power_profile_icon(profile), crate::paint::draw_power_glyph_over),
            SEGMENT_ICON,
            colour,
        );
        let band =
            (x + SEGMENT_ICON + SP1 + SP1 / 2, rect.loc.x + rect.size.w - SP1, rect.loc.y, rect.loc.y + rect.size.h);
        c.text(ctx.painter, band, TextAlign::Left, label, colour, &super::QS_PILL_TEXT);
    }
}

fn draw_power_row(c: &mut Canvas<'_>, ctx: &QsRenderContext<'_>, rect: LocalRect, action: PowerAction, armed: bool) {
    let theme = ctx.theme;
    let (icon_ink, label_ink) = if armed { (theme.danger, theme.danger) } else { (theme.muted, theme.label) };
    let icon = local_rect(rect.loc.x + CARD_PAD_X, rect.loc.y + (rect.size.h - MENU_ICON) / 2, MENU_ICON, MENU_ICON);
    super::qs_symbol(c, icon, (power_icon(action), power_glyph(action)), MENU_ICON, icon_ink);
    let (top, bottom) = (rect.loc.y, rect.loc.y + rect.size.h);
    let left = icon.loc.x + MENU_ICON + CARD_PAD_X;
    let mut right = rect.loc.x + rect.size.w - CARD_PAD_X;
    if action == PowerAction::Lock {
        let keycap = "Super+L";
        let width = text_w(ctx.painter, keycap, &KEYCAP);
        c.text(ctx.painter, (right - width, right, top, bottom), TextAlign::Right, keycap, theme.faint, &KEYCAP);
        right -= width + SP2;
    }
    c.text(ctx.painter, (left, right, top, bottom), TextAlign::Left, t(power_row_label(action)), label_ink, &MENU_ITEM);
}

fn draw_power_confirm(
    c: &mut Canvas<'_>, ctx: &QsRenderContext<'_>, rect: LocalRect, action: PowerAction, apps: &[String],
) {
    let theme = ctx.theme;
    let card = local_rect(rect.loc.x, rect.loc.y + SP1 / 2, rect.size.w, rect.size.h - SP1 / 2 - SP2);
    c.fill(card, theme.danger_soft, R2);
    let (left, right) = (card.loc.x + CONFIRM_PAD, card.loc.x + card.size.w - CONFIRM_PAD);
    let mut y = card.loc.y + CONFIRM_PAD;
    let title_h = super::QS_TITLE.line_height_px as i32;
    c.text(
        ctx.painter,
        (left, right, y, y + title_h),
        TextAlign::Left,
        t(power_question(action)),
        theme.label,
        &super::QS_TITLE,
    );
    y += title_h + SP2;
    let line_h = super::QS_NOTE.line_height_px as i32;
    for line in crate::notifications::wrap_lines(
        ctx.painter,
        &apps_note(apps),
        &super::QS_NOTE,
        right - left,
        CONFIRM_NOTE_LINES,
    ) {
        c.text(ctx.painter, (left, right, y, y + line_h), TextAlign::Left, &line, theme.muted, &super::QS_NOTE);
        y += line_h;
    }
    let (cancel, confirm) = confirm_buttons(ctx.painter, rect, action);
    c.fill(cancel, theme.track_soft, R2);
    c.text_in(ctx.painter, cancel, TextAlign::Center, t(Key::PowerCancel), theme.label, &super::QS_PILL_TEXT);
    c.fill(confirm, theme.danger, R2);
    c.text_in(ctx.painter, confirm, TextAlign::Center, t(power_verb(action)), theme.view_bg, &super::QS_PILL_TEXT);
}

fn battery_low(theme: &TopbarTheme, status: &crate::system_info::SystemStatus) -> bool {
    status.battery_pct.is_some_and(|pct| !status.battery_charging && pct <= theme.battery_low_pct)
}

fn draw_detail_header(
    c: &mut Canvas<'_>, ctx: &QsRenderContext<'_>, geometry: &DetailGeometry, detail: Detail,
    (panel, status): (&DetailPanel, &crate::system_info::SystemStatus),
) {
    let theme = ctx.theme;
    let back = geometry.back;
    let side = DETAIL_BACK_ICON;
    let at = (back.loc.x + (back.size.w - side) / 2, back.loc.y + (back.size.h - side) / 2);
    if !c.icon("go-previous", side, at, theme.label) {
        c.text_in(ctx.painter, back, TextAlign::Center, "\u{2039}", theme.label, &TextStyle::HEAD);
    }
    let title = t(match detail {
        Detail::Wifi => Key::QsNetwork,
        Detail::Bluetooth => Key::Bluetooth,
        Detail::Battery => Key::QsBatteryTitle,
        Detail::Sound => Key::SettingsSound,
        Detail::Vpn => Key::QsVpn,
        Detail::Power => Key::QsPowerTitle,
    });
    let top = QS_PAD;
    let bottom = top + DETAIL_HEAD;
    let title_band = (geometry.title_left, geometry.title_right, top, bottom);
    c.text(ctx.painter, title_band, TextAlign::Left, title, theme.label, &super::QS_TITLE_PAGE);
    if has_radio(detail) {
        draw_page_switch(c, theme, geometry.toggle, radio_on(detail, panel, status));
    }
}

const fn profile_hint(profile: PowerProfile) -> Key {
    match profile {
        PowerProfile::PowerSaver => Key::QsProfileSaverHint,
        PowerProfile::Balanced => Key::QsProfileBalancedHint,
        PowerProfile::Performance => Key::QsProfilePerformanceHint,
    }
}

fn status_subtitle(theme: &TopbarTheme, status: RowStatus) -> Option<(String, Color)> {
    let (key, danger) = match status {
        RowStatus::Idle => return None,
        RowStatus::Connecting => (Key::QsRowConnecting, false),
        RowStatus::Disconnecting => (Key::QsRowDisconnecting, false),
        RowStatus::Failed(ConnectError::WrongPassword) => (Key::QsRowWrongPassword, true),
        RowStatus::Failed(ConnectError::Failed) => (Key::QsCouldNotConnect, true),
    };
    let colour = if danger { theme.danger } else { theme.muted };
    Some((t(key).to_string(), colour))
}

const fn busy(status: RowStatus) -> bool {
    matches!(status, RowStatus::Connecting | RowStatus::Disconnecting)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tail {
    Nothing,
    Check,
    Lock,
    External,
    Busy,
    Switch(bool),
}

struct RowSpec<'a> {
    icon: &'a str,
    glyph: GlyphPainter,
    title: &'a str,
    subtitle: (String, Option<Color>),
    on: bool,
    tail: Tail,
}

fn draw_spinner(c: &mut Canvas<'_>, theme: &TopbarTheme, rect: LocalRect) {
    c.stroke(rect, 2.0, R_ROUND, theme.track);
    let arc = local_rect(rect.loc.x + rect.size.w / 2 - 1, rect.loc.y, 2, 2);
    c.fill(arc, theme.active_background, R_ROUND);
}

fn draw_row(c: &mut Canvas<'_>, ctx: &QsRenderContext<'_>, rect: LocalRect, spec: &RowSpec<'_>) {
    let theme = ctx.theme;
    if spec.on {
        c.fill(rect, theme.active_soft, R2);
    }
    let well = local_rect(rect.loc.x + ROW_PAD, rect.loc.y + (rect.size.h - ROW_WELL) / 2, ROW_WELL, ROW_WELL);
    let (fill, symbol) =
        if spec.on { (theme.active_background, theme.active_label) } else { (theme.track_soft, theme.muted) };
    c.fill(well, fill, R2);
    super::qs_symbol(c, well, (spec.icon, spec.glyph), ROW_GLYPH, symbol);

    let mut right = rect.loc.x + rect.size.w - ROW_PAD;
    let centre_y = rect.loc.y + rect.size.h / 2;
    let tail_ink = if spec.on { theme.active_soft_label } else { theme.faint };
    let tail = local_rect(right - ROW_TAIL, centre_y - ROW_TAIL / 2, ROW_TAIL, ROW_TAIL);
    match spec.tail {
        Tail::Nothing => {}
        Tail::Check => {
            super::qs_symbol(c, tail, ("object-select", crate::paint::draw_tick_glyph_over), ROW_TAIL, tail_ink);
        }
        Tail::Lock => {
            let lock = ("network-wireless-encrypted", crate::paint::draw_lock_glyph_over as GlyphPainter);
            super::qs_symbol(c, tail, lock, ROW_TAIL, tail_ink);
        }
        Tail::External => {
            if !c.icon("external-link", ROW_TAIL, (tail.loc.x, tail.loc.y), tail_ink) {
                c.text_in(ctx.painter, tail, TextAlign::Center, "\u{2197}", tail_ink, &TextStyle::CAP);
            }
        }
        Tail::Busy => draw_spinner(c, theme, tail),
        Tail::Switch(on) => {
            let switch =
                local_rect(right - DETAIL_SWITCH_W, centre_y - DETAIL_SWITCH_H / 2, DETAIL_SWITCH_W, DETAIL_SWITCH_H);
            draw_page_switch(c, theme, switch, on);
            right = switch.loc.x - ROW_GAP;
        }
    }
    if !matches!(spec.tail, Tail::Nothing | Tail::Switch(_)) {
        right = tail.loc.x - SP1 - SP1 / 2;
    }

    let left = well.loc.x + ROW_WELL + ROW_GAP;
    let title_h = super::QS_TITLE.line_height_px as i32;
    let sub_h = super::QS_NOTE.line_height_px as i32;
    let top = rect.loc.y + (rect.size.h - title_h - sub_h) / 2;
    let title_ink = if spec.on { theme.active_soft_label } else { theme.label };
    c.text(ctx.painter, (left, right, top, top + title_h), TextAlign::Left, spec.title, title_ink, &super::QS_TITLE);
    let sub_ink = spec.subtitle.1.unwrap_or(if spec.on { theme.active_soft_label } else { theme.muted });
    let band = (left, right, top + title_h, top + title_h + sub_h);
    c.text(ctx.painter, band, TextAlign::Left, &spec.subtitle.0, sub_ink, &super::QS_NOTE);
}

fn draw_page_switch(c: &mut Canvas<'_>, theme: &TopbarTheme, rect: LocalRect, on: bool) {
    c.fill(rect, if on { theme.active_background } else { theme.track }, R_ROUND);
    let x = if on {
        rect.loc.x + rect.size.w - DETAIL_SWITCH_INSET - DETAIL_SWITCH_THUMB
    } else {
        rect.loc.x + DETAIL_SWITCH_INSET
    };
    let knob = local_rect(x, rect.loc.y + DETAIL_SWITCH_INSET, DETAIL_SWITCH_THUMB, DETAIL_SWITCH_THUMB);
    c.fill(knob, if on { super::QS_THUMB_FACE } else { theme.thumb }, R_ROUND);
}

fn subtitle_with(status: (&TopbarTheme, RowStatus), fallback: String) -> (String, Option<Color>) {
    status_subtitle(status.0, status.1).map_or((fallback, None), |(text, colour)| (text, Some(colour)))
}

fn network_subtitle(network: &WifiNetwork) -> Key {
    if network.connected {
        Key::QsRowConnected
    } else if network.security == Security::Enterprise && !network.saved {
        Key::QsRowEnterprise
    } else if network.saved {
        Key::QsSaved
    } else if network.security.is_secured() {
        Key::QsSecured
    } else {
        Key::QsOpenNetwork
    }
}

fn draw_network_row(
    c: &mut Canvas<'_>, ctx: &QsRenderContext<'_>, rect: LocalRect, network: &WifiNetwork, status: RowStatus,
) {
    let tail = if busy(status) {
        Tail::Busy
    } else if network.connected {
        Tail::Check
    } else if network.security == Security::Enterprise {
        Tail::External
    } else if network.security.is_secured() && !network.saved {
        Tail::Lock
    } else {
        Tail::Nothing
    };
    let spec = RowSpec {
        icon: network.icon(),
        glyph: crate::paint::draw_wifi_waves_glyph_over,
        title: &network.ssid,
        subtitle: subtitle_with((ctx.theme, status), t(network_subtitle(network)).to_string()),
        on: network.connected,
        tail,
    };
    draw_row(c, ctx, rect, &spec);
}

fn draw_device_row(
    c: &mut Canvas<'_>, ctx: &QsRenderContext<'_>, rect: LocalRect, device: &BtDevice, status: RowStatus,
) {
    let fallback = if device.connected {
        device.battery.map_or_else(
            || t(Key::QsRowConnected).to_string(),
            |pct| format!("{} \u{b7} {}", t(Key::QsRowConnected), t(Key::QsRowBattery).replace("{}", &pct.to_string())),
        )
    } else {
        t(Key::QsRowNotConnected).to_string()
    };
    let tail = if busy(status) {
        Tail::Busy
    } else if device.connected {
        Tail::Check
    } else {
        Tail::Nothing
    };
    let spec = RowSpec {
        icon: device.kind.icon(),
        glyph: crate::paint::draw_bluetooth_glyph_over,
        title: &device.name,
        subtitle: subtitle_with((ctx.theme, status), fallback),
        on: device.connected,
        tail,
    };
    draw_row(c, ctx, rect, &spec);
}

fn draw_audio_row(
    c: &mut Canvas<'_>, ctx: &QsRenderContext<'_>, rect: LocalRect, device: &AudioOutput, status: RowStatus,
) {
    let tail = if busy(status) {
        Tail::Busy
    } else if device.active {
        Tail::Check
    } else {
        Tail::Nothing
    };
    let spec = RowSpec {
        icon: device.icon,
        glyph: |b, w, h, side, stroke, s, c| crate::paint::draw_speaker_glyph_over(b, w, h, side, stroke, false, s, c),
        title: &device.description,
        subtitle: subtitle_with((ctx.theme, status), t(super::qs_device_kind(device.icon)).to_string()),
        on: device.active,
        tail,
    };
    draw_row(c, ctx, rect, &spec);
}

fn draw_mic_slider(
    c: &mut Canvas<'_>, ctx: &QsRenderContext<'_>, rect: LocalRect, status: &crate::system_info::SystemStatus,
) {
    let row = super::qs_slider_row(rect.loc.x, rect.loc.y, rect.size.w, true);
    let (muted, pct) = (status.mic_muted, status.mic_pct);
    let colour = super::draw_qs_mute_button(c, ctx.theme, row.icon, muted);
    super::draw_qs_mic_glyph(c, row.icon, (muted, colour));
    super::draw_qs_readout(c, ctx, row.readout, Some((pct, muted)));
    let thumb = super::QsThumb::of(Slot::QsMicTrack, ctx.hovered, None);
    super::draw_qs_track(c, ctx.theme, row.track, super::QsTrack { pct, muted, enabled: true, thumb });
}

const fn vpn_kind_key(kind: VpnKind) -> Key {
    match kind {
        VpnKind::WireGuard => Key::QsVpnWireGuard,
        VpnKind::Vpn => Key::QsVpnGeneric,
    }
}

fn draw_vpn_row(
    c: &mut Canvas<'_>, ctx: &QsRenderContext<'_>, rect: LocalRect, vpn: &VpnConnection, status: RowStatus,
) {
    let kind = t(vpn_kind_key(vpn.kind));
    let fallback = if vpn.active { format!("{} \u{b7} {kind}", t(Key::QsRowConnected)) } else { kind.to_string() };
    let tail = if busy(status) {
        Tail::Busy
    } else if vpn.active {
        Tail::Check
    } else {
        Tail::Nothing
    };
    let spec = RowSpec {
        icon: "network-vpn",
        glyph: crate::paint::draw_lock_glyph_over,
        title: &vpn.name,
        subtitle: subtitle_with((ctx.theme, status), fallback),
        on: vpn.active,
        tail,
    };
    draw_row(c, ctx, rect, &spec);
}

fn draw_netbird_row(c: &mut Canvas<'_>, ctx: &QsRenderContext<'_>, rect: LocalRect, view: &netbird::NetbirdView) {
    let (on, busy, subtitle) = view.row();
    let tail = if busy {
        Tail::Busy
    } else if on {
        Tail::Check
    } else {
        Tail::Nothing
    };
    let spec = RowSpec {
        icon: "network-vpn",
        glyph: crate::paint::draw_lock_glyph_over,
        title: "NetBird",
        subtitle: (subtitle, None),
        on,
        tail,
    };
    draw_row(c, ctx, rect, &spec);
}

fn draw_ethernet_row(
    c: &mut Canvas<'_>, ctx: &QsRenderContext<'_>, rect: LocalRect, status: &crate::system_info::SystemStatus,
) {
    let on = status.ethernet_connected;
    let spec = RowSpec {
        icon: if on { "network-wired" } else { "network-wired-disconnected" },
        glyph: crate::paint::draw_wifi_waves_glyph_over,
        title: t(Key::QsEthernetShort),
        subtitle: (t(if on { Key::QsRowConnected } else { Key::QsCableDisconnected }).to_string(), None),
        on,
        tail: if on { Tail::Check } else { Tail::Nothing },
    };
    draw_row(c, ctx, rect, &spec);
}

fn draw_airplane_row(c: &mut Canvas<'_>, ctx: &QsRenderContext<'_>, rect: LocalRect, on: bool) {
    let spec = RowSpec {
        icon: "airplane-mode",
        glyph: crate::paint::draw_wifi_waves_glyph_over,
        title: t(Key::AirplaneMode),
        subtitle: (t(Key::QsAirplaneSub).to_string(), None),
        on: false,
        tail: Tail::Switch(on),
    };
    draw_row(c, ctx, rect, &spec);
}

fn draw_empty(c: &mut Canvas<'_>, ctx: &QsRenderContext<'_>, rect: LocalRect, empty: Empty) {
    let theme = ctx.theme;
    let centre = rect.loc.x + rect.size.w / 2;
    let mut y = rect.loc.y + EMPTY_PAD_TOP;
    let well = local_rect(centre - EMPTY_WELL / 2, y, EMPTY_WELL, EMPTY_WELL);
    c.fill(well, theme.fill_2, R3);
    super::qs_symbol(c, well, empty.icon.symbol(), super::QS_ICON, theme.muted);
    y += EMPTY_WELL + SP2 + SP1;
    let title_h = super::QS_TITLE.line_height_px as i32;
    let (left, right) = (rect.loc.x + SP3, rect.loc.x + rect.size.w - SP3);
    c.text(
        ctx.painter,
        (left, right, y, y + title_h),
        TextAlign::Center,
        t(empty.title),
        theme.label,
        &super::QS_TITLE,
    );
    y += title_h + SP1;
    if let Some(text) = empty.text {
        let line_h = EMPTY_TEXT.line_height_px as i32;
        for line in crate::notifications::wrap_lines(ctx.painter, t(text), &EMPTY_TEXT, right - left, EMPTY_TEXT_LINES)
        {
            c.text(ctx.painter, (left, right, y, y + line_h), TextAlign::Center, &line, theme.muted, &EMPTY_TEXT);
            y += line_h;
        }
    }
}

fn draw_button(c: &mut Canvas<'_>, ctx: &QsRenderContext<'_>, rect: LocalRect, label: &str, primary: bool) {
    let theme = ctx.theme;
    let (fill, ink) =
        if primary { (theme.active_background, theme.active_label) } else { (theme.track_soft, theme.label) };
    c.fill(rect, fill, R2);
    c.text_in(ctx.painter, rect, TextAlign::Center, label, ink, &super::QS_PILL_TEXT);
}

fn draw_prompt(c: &mut Canvas<'_>, ctx: &QsRenderContext<'_>, rect: LocalRect, prompt: &Prompt, failed: bool) {
    let parts = prompt_parts(rect, failed, prompt_pills(ctx.painter));
    draw_password_field(c, ctx, (parts.field, parts.reveal), prompt, failed);
    if let Some(error) = parts.error {
        let text = t(Key::QsWrongPasswordRetry);
        c.text_in(ctx.painter, error, TextAlign::Left, text, ctx.theme.danger, &super::QS_NOTE);
    }
    draw_button(c, ctx, parts.cancel, t(Key::PowerCancel), false);
    draw_button(c, ctx, parts.connect, t(Key::QsConnectShort), true);
}

fn info_line(info: &WifiInfo) -> String {
    let band = match info.freq_mhz {
        0 => None,
        1..3000 => Some(t(Key::QsBand24)),
        3000..5925 => Some(t(Key::QsBand5)),
        _ => Some(t(Key::QsBand6)),
    };
    let rate = (info.rate_mbps > 0).then(|| t(Key::QsRate).replace("{}", &info.rate_mbps.to_string()));
    std::iter::once(info.ip.clone())
        .filter(|ip| !ip.is_empty())
        .chain(band.map(str::to_string))
        .chain(rate)
        .collect::<Vec<_>>()
        .join(" \u{b7} ")
}

fn draw_info(c: &mut Canvas<'_>, ctx: &QsRenderContext<'_>, rect: LocalRect, info: Option<&WifiInfo>) {
    let parts = info_parts(rect, info_pills(ctx.painter));
    let line = info.map(info_line).unwrap_or_default();
    c.text_in(ctx.painter, parts.line, TextAlign::Left, &line, ctx.theme.muted, &INFO_TEXT);
    draw_button(c, ctx, parts.forget, t(Key::QsForgetNetwork), false);
    draw_button(c, ctx, parts.disconnect, t(Key::QsDisconnectShort), false);
}

fn draw_eye(c: &mut Canvas<'_>, ctx: &QsRenderContext<'_>, rect: LocalRect, revealed: bool) {
    let colour = if ctx.hovered == Some(Slot::QsPromptReveal) { ctx.theme.label } else { ctx.theme.faint };
    let side = PROMPT_EYE;
    let (x, y) = (rect.loc.x + (rect.size.w - side) / 2, rect.loc.y + (rect.size.h - side) / 2);
    let icon = if revealed { "view-conceal" } else { "view-reveal" };
    if c.icon(icon, side, (x, y), colour) {
        return;
    }
    c.stroke(local_rect(x, y + 3, side, side - 6), 1.5, R_ROUND, colour);
    c.fill(local_rect(x + side / 2 - 2, y + side / 2 - 2, 4, 4), colour, R_ROUND);
    if revealed {
        c.fill(local_rect(x, y + side / 2, side, 1), colour, 0.0);
    }
}

fn draw_password_field(
    c: &mut Canvas<'_>, ctx: &QsRenderContext<'_>, (boxed, reveal): (LocalRect, LocalRect), prompt: &Prompt,
    failed: bool,
) {
    let theme = ctx.theme;
    let field = &prompt.field;
    let (edge, ring) =
        if failed { (theme.danger, theme.danger_soft) } else { (theme.active_background, theme.acc_ring) };
    let halo = local_rect(boxed.loc.x - 3, boxed.loc.y - 3, boxed.size.w + 6, boxed.size.h + 6);
    c.fill(halo, ring, R2 + 3.0);
    c.fill(boxed, theme.view_bg, R2);
    c.stroke(boxed, 1.0, R2, edge);
    draw_eye(c, ctx, reveal, prompt.reveal);
    let (left, _, top, bottom) = band(boxed);
    let mut right = reveal.loc.x;
    if prompt.caps_lock {
        let cap = &TextStyle::CAP;
        let label = t(Key::CapsLockOn);
        let width = text_w(ctx.painter, label, cap);
        c.text(
            ctx.painter,
            (right - SP1 - width, right - SP1, top, bottom),
            TextAlign::Right,
            label,
            theme.danger,
            cap,
        );
        right -= width + SP1;
    }
    let (text_left, text_right) = (left + SP2, right - SP1);
    let text_band = (text_left, text_right, top, bottom);
    let style = TextStyle::CALL;
    if field.is_empty() {
        c.text(ctx.painter, text_band, TextAlign::Left, t(Key::QsPasswordPlaceholder), theme.faint, &style);
    } else {
        let shown = prompt.shown();
        if field.selection().is_some() {
            let width = text_w(ctx.painter, &shown, &style).min(text_right - text_left);
            let selection = local_rect(text_left, top + SP1, width, boxed.size.h - SP1 * 2);
            c.fill(selection, theme.active_soft, 0.0);
        }
        c.text(ctx.painter, text_band, TextAlign::Left, &shown, theme.label, &style);
    }
    let typed = &field.text()[..field.caret()];
    let before = if prompt.reveal { typed.to_string() } else { masked(typed) };
    let caret_x = (text_left + text_w(ctx.painter, &before, &style)).min(text_right);
    let caret = local_rect(caret_x, top + SP1 + 2, 1, boxed.size.h - SP1 * 2 - 4);
    c.fill(caret, theme.active_background, 0.0);
}

fn masked(text: &str) -> String {
    text.chars().map(|_| '\u{2022}').collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TopbarAction, TopbarTheme};
    use smithay::utils::Size;

    #[test]
    fn sound_scroll_reaches_inputs_after_many_outputs() {
        let device =
            |name: &str| AudioOutput { name: name.into(), description: name.into(), icon: "audio-card", active: false };
        let mut panel = DetailPanel {
            outputs: vec![device("one"), device("two"), device("three")],
            inputs: Some(vec![device("microphone")]),
            ..Default::default()
        };
        let body = local_rect(0, 0, 300, DETAIL_SECTION + SP3 + DETAIL_CARD);
        let status = crate::system_info::SystemStatus::default();
        let input = |item: &DetailItem| item.kind == ItemKind::Input(0);
        assert!(!detail_items(&panel, Detail::Sound, &status, body, None).iter().any(input));
        panel.sound_scroll = 3;
        let items = detail_items(&panel, Detail::Sound, &status, body, None);
        assert!(items.iter().any(|item| item.kind == ItemKind::Input(0)));
    }

    fn output() -> Rectangle<i32, Logical> {
        Rectangle::new(Point::from((0, 0)), Size::from((1920, 1080)))
    }

    fn relayout(bar: &mut Topbar) {
        bar.layout_slots(&crate::paint::Painter::new(14.0, None), output());
    }

    fn network(ssid: &str, secured: bool, connected: bool) -> WifiNetwork {
        let security = if secured { Security::Passphrase } else { Security::Open };
        WifiNetwork { ssid: ssid.to_string(), security, signal: 60, connected, saved: false }
    }

    fn open_wifi() -> Topbar {
        let mut bar = Topbar::new(TopbarTheme::default());
        bar.set_system_status(crate::system_info::SystemStatus {
            wifi_enabled: true,
            bluetooth_on: true,
            ..Default::default()
        });
        bar.toggle_popup(OpenPopup::QuickSettings);
        relayout(&mut bar);
        assert!(bar.open_detail(Detail::Wifi));
        bar.apply_net_event(NetEvent::Wifi(vec![
            network("Home", true, true),
            network("Cafe", false, false),
            network("Neighbour", true, false),
        ]));
        relayout(&mut bar);
        bar
    }

    fn centre(rect: Rectangle<i32, Logical>) -> Point<f64, Logical> {
        Point::from((f64::from(rect.loc.x + rect.size.w / 2), f64::from(rect.loc.y + rect.size.h / 2)))
    }

    #[test]
    fn the_chevron_opens_the_wifi_panel_and_back_returns_to_the_tiles() {
        let mut bar = Topbar::new(TopbarTheme::default());
        bar.toggle_popup(OpenPopup::QuickSettings);
        relayout(&mut bar);
        let more = bar.slot_rect(Slot::QsWifiMore).expect("chevron slot");
        let body = bar.slot_rect(Slot::QsWifi).expect("tile slot");
        assert!(body.loc.x + body.size.w <= more.loc.x, "the chevron sits after the tile body");
        assert_eq!(bar.action_at(centre(more)), Some(TopbarAction::QsPanel(PanelAction::Open(Detail::Wifi))));

        assert!(bar.open_detail(Detail::Wifi));
        assert!(!bar.open_detail(Detail::Wifi), "already there");
        relayout(&mut bar);
        assert_eq!(bar.qs_key_mode(), QsKeyMode::Detail);
        assert!(bar.slot_rect(Slot::QsWifi).is_none(), "tiles leave with the main view");
        let back = bar.slot_rect(Slot::QsBack).expect("back button");
        assert_eq!(bar.action_at(centre(back)), Some(TopbarAction::QsPanel(PanelAction::Back)));
        assert!(bar.slot_rect(Slot::QsDetailSettings).is_some());

        assert!(bar.close_detail());
        assert!(!bar.close_detail());
        relayout(&mut bar);
        assert!(bar.slot_rect(Slot::QsWifi).is_some());
        assert_eq!(bar.qs_key_mode(), QsKeyMode::Normal);
    }

    #[test]
    fn keyboard_focus_follows_the_view_it_came_from() {
        let mut bar = Topbar::new(TopbarTheme::default());
        bar.toggle_popup(OpenPopup::QuickSettings);
        relayout(&mut bar);
        bar.set_focused(Some(Slot::QsBluetoothMore));
        bar.open_detail(Detail::Bluetooth);
        assert_eq!(bar.focused, Some(Slot::QsBack));
        bar.close_detail();
        assert_eq!(bar.focused, Some(Slot::QsBluetoothMore));
    }

    #[test]
    fn reopening_quick_settings_starts_on_the_main_view() {
        let mut bar = open_wifi();
        bar.close_popup();
        bar.toggle_popup(OpenPopup::QuickSettings);
        assert_eq!(bar.detail(), None);
        assert!(bar.qs_panel.slide_progress() <= 0.0);
    }

    #[test]
    fn rows_list_the_networks_in_order_and_resolve_back_to_them() {
        let bar = open_wifi();
        let first = bar.slot_rect(Slot::QsDetailRow(0)).expect("row 0");
        let second = bar.slot_rect(Slot::QsDetailRow(1)).expect("row 1");
        let third = bar.slot_rect(Slot::QsDetailRow(2)).expect("row 2");
        assert_eq!(first.size.h, DETAIL_CARD);
        assert_eq!(second.loc.y, first.loc.y + DETAIL_CARD + DETAIL_CARD_GAP, "one list, the connected network first");
        assert_eq!(third.loc.y, second.loc.y + DETAIL_CARD + DETAIL_CARD_GAP, "rows sit 2 px apart");
        assert_eq!(bar.detail_row(1), Some(RowTarget::Network(network("Cafe", false, false))));
        assert_eq!(bar.detail_row(9), None);
    }

    #[test]
    fn each_view_groups_its_cards_under_section_labels() {
        let bar = open_wifi();
        let geometry = detail_geometry(super::super::QS_PAGE_MAX, true, true);
        let items = detail_items(&bar.qs_panel, Detail::Wifi, bar.system_status(), geometry.body, None);
        let kinds: Vec<ItemKind> = items.iter().map(|item| item.kind).collect();
        assert_eq!(
            kinds,
            [
                ItemKind::Section(Key::WiFi),
                ItemKind::Network(0),
                ItemKind::Network(1),
                ItemKind::Network(2),
                ItemKind::Separator,
                ItemKind::Airplane,
            ]
        );
        assert!(items.iter().all(|item| geometry.body.contains(item.rect.loc)));

        let status = crate::system_info::SystemStatus {
            battery_pct: Some(70),
            power_profile: Some(PowerProfile::Balanced),
            ..Default::default()
        };
        let battery = detail_items(&bar.qs_panel, Detail::Battery, &status, geometry.body, None);
        assert_eq!(battery[0].kind, ItemKind::BatteryHero);
        let profiles: Vec<Option<Slot>> =
            battery.iter().filter(|item| matches!(item.kind, ItemKind::Segment(_))).map(|item| item.slot).collect();
        assert_eq!(profiles, super::super::QS_PROFILES.map(|profile| Some(Slot::QsProfile(profile))));
        let off = crate::system_info::SystemStatus { wifi_enabled: false, ..status };
        let wifi_off = detail_items(&bar.qs_panel, Detail::Wifi, &off, geometry.body, None);
        let kinds: Vec<ItemKind> = wifi_off.iter().map(|item| item.kind).collect();
        assert_eq!(kinds, [ItemKind::Empty(Empty::wifi_off(false)), ItemKind::Separator, ItemKind::Airplane]);
        let wired = crate::system_info::SystemStatus { wifi_available: false, ethernet_available: true, ..off };
        let kinds: Vec<ItemKind> = detail_items(&bar.qs_panel, Detail::Wifi, &wired, geometry.body, None)
            .iter()
            .map(|item| item.kind)
            .collect();
        assert_eq!(kinds, [ItemKind::Section(Key::QsWired), ItemKind::Ethernet], "a desktop has no airplane mode");
    }

    #[test]
    fn arrows_walk_the_rows_from_the_header_down_to_settings() {
        let mut bar = open_wifi();
        bar.set_focused(Some(Slot::QsBack));
        let mut visited = vec![Slot::QsBack];
        while bar.focus_nav(0, 1) {
            visited.push(bar.focused.expect("focus"));
        }
        assert_eq!(
            visited,
            [
                Slot::QsBack,
                Slot::QsDetailRow(0),
                Slot::QsDetailRow(1),
                Slot::QsDetailRow(2),
                Slot::QsAirplane,
                Slot::QsDetailSettings,
            ]
        );
    }

    #[test]
    fn the_connected_network_unfolds_its_details_and_one_accordion_is_open_at_a_time() {
        let mut bar = open_wifi();
        bar.apply_net_event(NetEvent::WifiInfo(Some(WifiInfo {
            ip: "192.168.1.24".into(),
            freq_mhz: 5180,
            rate_mbps: 866,
        })));
        bar.toggle_info("Home");
        relayout(&mut bar);
        let home = bar.slot_rect(Slot::QsDetailRow(0)).expect("home row");
        let forget = bar.slot_rect(Slot::QsNetForget).expect("forget");
        let disconnect = bar.slot_rect(Slot::QsNetDisconnect).expect("disconnect");
        assert!(forget.loc.y > home.loc.y + home.size.h && forget.loc.x + forget.size.w < disconnect.loc.x);
        assert_eq!(bar.action_at(centre(forget)), Some(TopbarAction::QsPanel(PanelAction::Forget)));
        assert_eq!(bar.action_at(centre(disconnect)), Some(TopbarAction::QsPanel(PanelAction::Disconnect)));
        assert_eq!(bar.expanded_network(), Some("Home"));
        let line = info_line(bar.qs_panel.wifi_info.as_ref().expect("info"));
        assert!(line.starts_with("192.168.1.24") && line.contains("866"), "{line}");

        bar.toggle_prompt("Neighbour");
        assert_eq!(bar.expanded_network(), None, "the password drawer folds the details");
        bar.toggle_info("Home");
        assert!(bar.qs_panel.prompt.is_none(), "and the other way round");
        assert!(bar.qs_escape(), "Esc folds the accordion first");
        assert_eq!(bar.expanded_network(), None);
        assert_eq!(bar.detail(), Some(Detail::Wifi));
    }

    #[test]
    fn a_switched_off_radio_lists_nothing() {
        let mut bar = open_wifi();
        let mut status = bar.system_status().clone();
        status.wifi_enabled = false;
        bar.set_system_status(status);
        relayout(&mut bar);
        assert!(bar.slot_rect(Slot::QsDetailRow(0)).is_none());
        assert!(bar.slot_rect(Slot::QsDetailToggle).is_some());
    }

    #[test]
    fn a_secured_network_asks_for_a_password_under_its_row() {
        let mut bar = open_wifi();
        bar.toggle_prompt("Neighbour");
        relayout(&mut bar);
        let row = bar.slot_rect(Slot::QsDetailRow(2)).expect("neighbour row");
        let field = bar.slot_rect(Slot::QsPasswordField).expect("field row");
        let cancel = bar.slot_rect(Slot::QsPromptCancel).expect("cancel pill");
        let connect = bar.slot_rect(Slot::QsPromptConnect).expect("connect pill");
        assert!(field.loc.y > row.loc.y + row.size.h, "the drawer opens under its card");
        assert!(field.loc.y + field.size.h < cancel.loc.y && cancel.loc.x + cancel.size.w < connect.loc.x);
        assert_eq!(bar.action_at(centre(connect)), Some(TopbarAction::QsPanel(PanelAction::PromptSubmit)));
        assert_eq!(bar.action_at(centre(cancel)), Some(TopbarAction::QsPanel(PanelAction::PromptCancel)));
        assert_eq!(bar.qs_key_mode(), QsKeyMode::Typing);

        for ch in "pa55".chars() {
            assert!(bar.prompt_key(FieldKey::Type(ch)) == PromptOutcome::Changed);
        }
        assert!(bar.prompt_key(FieldKey::Backspace) == PromptOutcome::Changed);
        assert!(bar.prompt_key(FieldKey::Type('w')) == PromptOutcome::Changed);
        let submitted = bar.prompt_key(FieldKey::Submit);
        assert!(submitted == PromptOutcome::Submit { ssid: "Neighbour".to_string(), password: "pa5w".to_string() });
        assert_eq!(bar.qs_key_mode(), QsKeyMode::Detail, "the field is gone");
    }

    #[test]
    fn the_password_field_warns_about_caps_lock() {
        let mut bar = open_wifi();
        assert!(!bar.set_caps_lock(true), "no field open, nothing to warn");
        bar.toggle_prompt("Neighbour");
        assert!(bar.set_caps_lock(true));
        assert!(!bar.set_caps_lock(true), "an unchanged state does not repaint");
        assert!(bar.qs_panel.prompt.as_ref().is_some_and(|prompt| prompt.caps_lock));
        assert!(bar.set_caps_lock(false));
    }

    #[test]
    fn escape_cancels_the_prompt_and_an_empty_field_never_submits() {
        let mut bar = open_wifi();
        bar.toggle_prompt("Neighbour");
        assert!(bar.prompt_key(FieldKey::Submit) == PromptOutcome::Unchanged);
        assert!(bar.prompt_key(FieldKey::Cancel) == PromptOutcome::Cancelled);
        assert!(bar.prompt_key(FieldKey::Type('x')) == PromptOutcome::Unchanged);
        assert_eq!(bar.focused, Some(Slot::QsDetailRow(2)));
    }

    #[test]
    fn a_wrong_password_reopens_the_prompt_with_the_error() {
        let mut bar = open_wifi();
        bar.begin_connect("Neighbour");
        assert_eq!(bar.qs_panel.status_for("Neighbour"), RowStatus::Connecting);
        bar.apply_net_event(NetEvent::WifiResult {
            ssid: "Neighbour".to_string(),
            result: Err(ConnectError::WrongPassword),
        });
        assert_eq!(bar.qs_panel.status_for("Neighbour"), RowStatus::Failed(ConnectError::WrongPassword));
        assert_eq!(bar.qs_key_mode(), QsKeyMode::Typing);
        bar.apply_net_event(NetEvent::WifiResult { ssid: "Neighbour".to_string(), result: Ok(()) });
        assert_eq!(bar.qs_panel.status_for("Neighbour"), RowStatus::Idle);
    }

    #[test]
    fn an_identical_scan_does_not_repaint() {
        let mut bar = open_wifi();
        let list = bar.qs_panel.wifi.clone();
        assert!(!bar.apply_net_event(NetEvent::Wifi(list)));
    }

    #[test]
    fn closing_the_popup_forgets_a_typed_password() {
        let mut bar = open_wifi();
        bar.toggle_prompt("Neighbour");
        bar.prompt_key(FieldKey::Type('s'));
        bar.close_popup();
        assert!(bar.qs_panel.prompt.is_none());
    }

    #[test]
    fn typing_keys_map_onto_field_edits() {
        let key = |sym: Keysym, ctrl: bool, typed: Option<char>| field_key(ctrl, !ctrl, typed, |want| want == sym);
        assert_eq!(key(Keysym::Escape, false, None), Some(FieldKey::Cancel));
        assert_eq!(key(Keysym::Return, false, None), Some(FieldKey::Submit));
        assert_eq!(key(Keysym::BackSpace, true, None), Some(FieldKey::DeleteWord));
        assert_eq!(key(Keysym::Left, true, None), Some(FieldKey::Caret(CaretMove::WordLeft)));
        assert_eq!(key(Keysym::a, true, Some('a')), Some(FieldKey::SelectAll));
        assert_eq!(key(Keysym::a, false, Some('a')), Some(FieldKey::Type('a')));
        assert_eq!(key(Keysym::Down, false, None), None, "arrows leave the field");
        assert_eq!(field_key(false, false, Some('q'), |_| false), None);
        assert_eq!(format!("{:?}", FieldKey::Type('s')), "Type(*)");
    }

    #[test]
    fn every_power_row_is_laid_out_in_order_and_reports_its_action() {
        let mut bar = Topbar::new(TopbarTheme::default());
        bar.toggle_popup(OpenPopup::QuickSettings);
        bar.open_detail(Detail::Power);
        relayout(&mut bar);
        let mut previous_bottom = i32::MIN;
        for action in POWER_ROWS {
            let rect = bar.slot_rect(Slot::QsPowerButton(action)).expect("power row");
            assert!(rect.loc.y >= previous_bottom);
            assert_eq!(rect.size.h, MENU_ROW);
            previous_bottom = rect.loc.y + rect.size.h;
            let expected =
                if is_destructive(action) { TopbarAction::QsArmPower(action) } else { TopbarAction::QsPower(action) };
            assert_eq!(bar.action_at(centre(rect)), Some(expected));
        }
    }

    #[test]
    fn the_battery_time_line_joins_state_and_estimate() {
        let info = BatteryInfo {
            pct: 50,
            state: BatteryState::Discharging,
            minutes: Some(135),
            health: crate::system_info::BatteryHealth::default(),
            profile: None,
        };
        let eta = t(Key::TbBatteryTimeLeft).replace("{}", &crate::i18n::duration_label(135));
        assert_eq!(time_line(&info), format!("{} \u{b7} {eta}", t(Key::TbBatteryOnBattery)));
        assert!(crate::i18n::duration_label(135).contains("15"));
        let full = BatteryInfo { state: BatteryState::Full, minutes: Some(10), ..info };
        assert_eq!(time_line(&full), t(Key::TbBatteryFull));
    }

    #[test]
    fn the_battery_row_shows_the_estimate_alone_when_there_is_one() {
        let info = BatteryInfo {
            pct: 50,
            state: BatteryState::Discharging,
            minutes: Some(135),
            health: crate::system_info::BatteryHealth::default(),
            profile: None,
        };
        let status =
            |battery| crate::system_info::SystemStatus { battery_pct: Some(50), battery, ..Default::default() };
        let left = t(Key::TbBatteryTimeLeft).replace("{}", &crate::i18n::duration_label(135));
        assert_eq!(battery_head_line(&status(Some(info))), left);
        let charging = BatteryInfo { state: BatteryState::Charging, ..info };
        let full = t(Key::TbBatteryTimeToFull).replace("{}", &crate::i18n::duration_label(135));
        assert_eq!(battery_head_line(&status(Some(charging))), full);
        let unknown = BatteryInfo { minutes: None, ..info };
        assert_eq!(battery_head_line(&status(Some(unknown))), t(Key::TbBatteryOnBattery));
        let done = BatteryInfo { state: BatteryState::Full, ..info };
        assert_eq!(battery_head_line(&status(Some(done))), t(Key::TbBatteryFull));
        assert_eq!(battery_head_line(&status(None)), t(Key::TbBatteryOnBattery));
    }

    #[test]
    fn without_a_battery_report_the_state_falls_back_to_the_charging_flag() {
        let status =
            crate::system_info::SystemStatus { battery_pct: Some(40), battery_charging: true, ..Default::default() };
        assert_eq!(battery_state_line(&status), t(Key::TbBatteryCharging));
        assert!(!has_radio(Detail::Battery) && !has_radio(Detail::Power));
        assert_eq!(more_slot(Detail::Battery), Slot::QsBattery);
    }

    fn sink(name: &str, active: bool) -> AudioOutput {
        AudioOutput { name: name.to_string(), description: name.to_uppercase(), icon: "audio-speakers", active }
    }

    fn two_sinks() -> NetEvent {
        NetEvent::Outputs(vec![sink("speakers", true), sink("hdmi", false)])
    }

    #[test]
    fn the_volume_chevron_opens_a_list_of_audio_outputs() {
        let mut bar = Topbar::new(TopbarTheme::default());
        bar.toggle_popup(OpenPopup::QuickSettings);
        relayout(&mut bar);
        let more = bar.slot_rect(Slot::QsVolumeMore).expect("volume chevron");
        assert_eq!(bar.action_at(centre(more)), Some(TopbarAction::QsPanel(PanelAction::Open(Detail::Sound))));
        assert!(bar.open_detail(Detail::Sound));
        relayout(&mut bar);
        assert!(bar.slot_rect(Slot::QsDetailToggle).is_none(), "outputs have no radio");
        assert!(bar.slot_rect(Slot::QsDetailRow(0)).is_none(), "nothing listed before the scan");

        assert!(bar.apply_net_event(two_sinks()));
        relayout(&mut bar);
        assert!(bar.slot_rect(Slot::QsDetailRow(1)).is_some());
        assert_eq!(bar.detail_row(1), Some(RowTarget::Output(sink("hdmi", false))));
        assert!(!bar.apply_net_event(two_sinks()));

        bar.set_focused(Some(Slot::QsBack));
        bar.close_detail();
        assert_eq!(bar.focused, Some(Slot::QsVolumeMore), "focus returns to the chevron");
    }

    #[test]
    fn a_failed_output_switch_marks_its_row() {
        let mut bar = Topbar::new(TopbarTheme::default());
        bar.toggle_popup(OpenPopup::QuickSettings);
        bar.open_detail(Detail::Sound);
        bar.apply_net_event(two_sinks());
        bar.begin_connect("hdmi");
        assert_eq!(bar.qs_panel.status_for("hdmi"), RowStatus::Connecting);
        bar.apply_net_event(NetEvent::OutputResult { name: "hdmi".into(), result: Err(ConnectError::Failed) });
        assert_eq!(bar.qs_panel.status_for("hdmi"), RowStatus::Failed(ConnectError::Failed));
    }

    #[test]
    fn the_detail_list_stays_inside_the_popup() {
        let height = super::super::QS_PAGE_MAX;
        for radio in [true, false] {
            let geometry = detail_geometry(height, radio, true);
            assert!(geometry.body.size.h >= (DETAIL_CARD + DETAIL_CARD_GAP) * 6, "room for a useful list");
            assert!(geometry.body.loc.y + geometry.body.size.h < geometry.divider_bottom);
            assert_eq!(geometry.settings.loc.y + geometry.settings.size.h, height - QS_PAD);
            assert!(geometry.title_left < geometry.title_right);
            assert!(geometry.body.loc.y > geometry.back.loc.y + geometry.back.size.h);
        }
        assert_eq!(
            detail_geometry(height, true, true).toggle.size,
            smithay::utils::Size::from((DETAIL_SWITCH_W, DETAIL_SWITCH_H))
        );
        assert_eq!(prompt_height(true) - prompt_height(false), PROMPT_ERROR + SP2);
    }

    #[test]
    fn every_detail_view_draws_without_icons_at_both_scales() {
        let mut bar = open_wifi();
        bar.toggle_prompt("Neighbour");
        bar.apply_net_event(NetEvent::WifiResult {
            ssid: "Neighbour".to_string(),
            result: Err(ConnectError::WrongPassword),
        });
        assert!(bar.qs_panel.prompt.is_some() && bar.qs_panel.failed(2), "the drawer reopens with its error");
        let device = |mac: &str, connected: bool, battery: Option<u8>| BtDevice {
            mac: mac.to_string(),
            name: mac.to_uppercase(),
            kind: crate::qs_net::BtKind::Headphones,
            connected,
            battery,
        };
        bar.apply_net_event(NetEvent::Bluetooth(vec![device("aa", true, Some(80)), device("bb", false, None)]));
        bar.apply_net_event(two_sinks());
        bar.apply_net_event(NetEvent::Inputs(Vec::new()));
        assert!(bar.toggle_reveal());
        bar.begin_connect("hdmi");
        let status = crate::system_info::SystemStatus {
            wifi_enabled: true,
            bluetooth_on: true,
            volume_muted: true,
            battery_pct: Some(9),
            power_profile: Some(PowerProfile::PowerSaver),
            battery: Some(BatteryInfo {
                pct: 9,
                state: BatteryState::Discharging,
                minutes: None,
                health: crate::system_info::BatteryHealth { wear_pct: Some(91), ..Default::default() },
                profile: None,
            }),
            ..Default::default()
        };
        connect_netbird(&mut bar);
        let painter = crate::paint::Painter::new(14.0, None);
        let theme = TopbarTheme::default();
        bar.qs_panel.armed = Some(PowerAction::ShutDown);
        bar.qs_panel.apps = vec!["Firefox".into()];
        let details = [Detail::Wifi, Detail::Bluetooth, Detail::Sound, Detail::Battery, Detail::Vpn, Detail::Power];
        for detail in details {
            let height = bar.page_height(Some(&painter), detail);
            for (scale, hovered) in [(1, None), (2, Some(Slot::QsPromptCancel)), (2, Some(Slot::QsDetailSettings))] {
                let ctx = QsRenderContext { painter: &painter, theme: &theme, hovered };
                let mut canvas = Canvas::new(QS_WIDTH, height, scale).expect("detail canvas");
                draw_detail(&mut canvas, &ctx, &bar.qs_panel, detail, &status, 0.5);
            }
        }
    }

    fn connect_netbird(bar: &mut Topbar) {
        use glide_netbird as nb;
        bar.set_netbird_installed(true);
        let status = nb::Status {
            state: nb::State::Connected,
            session_expires_at: Some(std::time::SystemTime::now() + std::time::Duration::from_mins(10)),
            peers: vec![nb::Peer { status: nb::PeerStatus::Connected, ..nb::Peer::default() }, nb::Peer::default()],
            ..nb::Status::default()
        };
        bar.apply_netbird(NetbirdUpdate::Status(Box::new(status)));
        bar.apply_netbird(NetbirdUpdate::Profiles(vec![
            nb::Profile {
                id: "default".into(),
                name: "default".into(),
                active: true,
                email: Some("me@example.com".into()),
            },
            nb::Profile { id: "work".into(), name: "Work".into(), active: false, email: None },
        ]));
        bar.apply_netbird(NetbirdUpdate::Networks(vec![
            nb::Network { id: "exit".into(), range: "0.0.0.0/0".into(), selected: true, ..nb::Network::default() },
            nb::Network { id: "office".into(), range: "10.0.0.0/8".into(), selected: true, ..nb::Network::default() },
        ]));
    }

    fn vpn(name: &str, active: bool) -> VpnConnection {
        VpnConnection { name: name.into(), kind: VpnKind::WireGuard, active }
    }

    #[test]
    fn the_vpn_page_lists_netbird_and_every_networkmanager_connection() {
        let mut bar = Topbar::new(TopbarTheme::default());
        connect_netbird(&mut bar);
        bar.apply_net_event(NetEvent::Vpn(vec![vpn("Office", true), vpn("Proton", false)]));
        let body = detail_geometry(600, has_radio(Detail::Vpn), true).body;
        let kinds: Vec<ItemKind> = detail_items(&bar.qs_panel, Detail::Vpn, &bar.system_status, body, None)
            .into_iter()
            .map(|i| i.kind)
            .collect();
        assert_eq!(
            kinds,
            [ItemKind::Section(Key::QsVpnSection), ItemKind::VpnNetbird, ItemKind::Vpn(0), ItemKind::Vpn(1)]
        );
        assert!(!has_radio(Detail::Vpn), "one active connection, chosen by its row");
        assert_eq!(more_slot(Detail::Vpn), Slot::QsNetbirdMore);
        bar.toggle_popup(OpenPopup::QuickSettings);
        bar.open_detail(Detail::Vpn);
        assert_eq!(bar.detail_row(0), Some(RowTarget::Netbird));
        assert_eq!(bar.detail_row(2), Some(RowTarget::Vpn(vpn("Proton", false))));
        assert_eq!(
            bar.vpn_toggle_target(),
            Some(RowTarget::Vpn(vpn("Office", true))),
            "the tile turns the active one off"
        );
    }

    #[test]
    fn without_any_vpn_the_page_explains_itself() {
        let bar = Topbar::new(TopbarTheme::default());
        let body = detail_geometry(600, false, true).body;
        let items = detail_items(&bar.qs_panel, Detail::Vpn, &bar.system_status, body, None);
        assert!(matches!(items.as_slice(), [DetailItem { kind: ItemKind::Empty(_), slot: None, .. }]));
        assert_eq!(bar.vpn_toggle_target(), None);
    }

    #[test]
    fn the_eye_shows_the_typed_password_and_a_new_prompt_starts_hidden() {
        let mut bar = open_wifi();
        bar.toggle_prompt("Neighbour");
        relayout(&mut bar);
        for ch in "pa55".chars() {
            bar.prompt_key(FieldKey::Type(ch));
        }
        let field = bar.slot_rect(Slot::QsPasswordField).expect("field");
        let eye = bar.slot_rect(Slot::QsPromptReveal).expect("eye");
        assert!(field.contains(eye.loc), "the eye sits inside the field");
        assert_eq!(
            bar.action_at(centre(eye)),
            Some(TopbarAction::QsPanel(PanelAction::PromptReveal)),
            "it wins over the field"
        );
        let prompt = || bar.qs_panel.prompt.as_ref().expect("prompt");
        assert_eq!(prompt().shown(), "\u{2022}".repeat(4));
        assert!(bar.toggle_reveal());
        assert_eq!(bar.qs_panel.prompt.as_ref().expect("prompt").shown(), "pa55");
        assert!(format!("{:?}", bar.qs_panel.prompt.as_ref().expect("prompt")).contains("len: 4"));
        assert!(
            !format!("{:?}", bar.qs_panel.prompt.as_ref().expect("prompt")).contains("pa55"),
            "Debug never prints it"
        );
        assert_eq!(bar.qs_key_mode(), QsKeyMode::Typing, "the field keeps the keyboard");
        bar.prompt_key(FieldKey::Cancel);
        bar.toggle_prompt("Neighbour");
        assert!(!bar.qs_panel.prompt.as_ref().expect("prompt").reveal);
        bar.prompt_key(FieldKey::Cancel);
        assert!(!bar.toggle_reveal(), "no field, nothing to show");
    }
}
