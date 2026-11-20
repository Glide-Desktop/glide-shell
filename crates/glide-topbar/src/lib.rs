use std::collections::HashMap;
use std::time::{Duration, Instant};

use glide_shell::i18n::Key;

use glide_theme::TextStyle;
use glide_theme::motion::{self, CubicBezier, E_IO, E_OUT, E_OVER, ease_io, ease_out, lerp};
use glide_theme::shape::{R_ROUND, R1, R2, R3};
use glide_theme::spacing::{SP1, SP2};
use glide_ui::{Reveal, Tween};
use smithay::{
    backend::renderer::element::{
        Kind,
        memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
    },
    utils::{Logical, Point, Rectangle, Scale, Size},
};

use crate::global_menu::MenuPath;
use crate::power::{PowerAction, is_destructive};
use crate::system_info::PowerProfile;
use glide_paint::{self as paint, TextAlign};
use glide_shell::popup_menu::text_px as text_w;
use glide_shell::raster::{Canvas, ShellRenderer};
use glide_shell::{i18n, icon, popup_menu, raster};
use glide_theme::Color;

pub mod global_menu;
pub mod headset;
pub mod keychain_prompt;
pub mod netbird;
pub mod notifications;
pub mod phone;
pub mod polkit;
pub mod power;
pub mod qs_net;
pub mod system_info;

mod panels;

pub mod calendar;

mod island;

mod glyphs;

mod battery;

pub use island::{ALERT_HOLD, IslandAlert};

mod status;

mod face_tip;
mod tray;

mod headset_view;
mod phone_view;

mod polkit_view;

pub mod passwords_view;

mod keychain_view;

pub use tray::{ICON as TRAY_ICON, IconArt, MAX_SCALE as TRAY_MAX_SCALE, TrayButton, TrayClick, TrayIcon};

pub use panels::{
    Detail, FieldKey, NetbirdPending, NetbirdUpdate, PanelAction, PromptOutcome, QsKeyMode, RowTarget, field_key,
    remaining_text,
};

pub const HEIGHT: i32 = 34;

const PAD_X: i32 = glide_theme::spacing::SP2;
const BUTTON_HEIGHT: i32 = 24;
const BUTTON_PAD_X: i32 = glide_theme::spacing::SP2;
const ITEM_GAP: i32 = 2;
const BUTTON_RADIUS: f32 = glide_theme::shape::R2;
const BUTTON_MIN_WIDTH: i32 = 28;

const APP_ICON: i32 = 18;
const APP_PAD_LEFT: i32 = 5;
const APP_PAD_RIGHT: i32 = 9;
const APP_GAP: i32 = SP2;
const APP_STYLE: TextStyle = TextStyle {
    size_px: TextStyle::TOPBAR.size_px,
    line_height_px: TextStyle::TOPBAR.line_height_px,
    ..TextStyle::HEAD
};
const APP_NAME_MAX: usize = 36;

const SEG_HEIGHT: i32 = 20;
const SEG_INSET: i32 = 2;
const SEG_GAP: i32 = 2;
const SEG_MIN_WIDTH: i32 = 25;
const SEG_RADIUS: f32 = glide_theme::shape::R1;
const SEG_PAD_X: i32 = 3;
const SEG_TEXT_PAD: i32 = 5;
const SEG_LIFT_DARK: f32 = 0.35;
const SEG_LIFT_LIGHT: f32 = 0.08;
const SEG_STYLE: TextStyle =
    TextStyle { size_px: 10.5, line_height_px: 14.0, weight: 500, tracking_em: 0.03, ..TextStyle::CAP };

const SWITCH_WIDTH: i32 = 28;
const SWITCH_HEIGHT: i32 = 16;
const SWITCH_THUMB: i32 = 12;
const SWITCH_INSET: i32 = 2;

const COFFEE_GLYPH: i32 = status::ICON;
const COFFEE_PAD: i32 = 6;
const COFFEE_GAP: i32 = 7;
const COFFEE_PAD_END: i32 = 6;

const LANG_MENU_WIDTH: i32 = 196;
const LANG_MENU_PAD: i32 = glide_theme::spacing::SP1;
const LANG_MENU_ITEM_HEIGHT: i32 = 34;
const LANG_MENU_DIVIDER: i32 = glide_theme::spacing::SP1 * 2 + 1;

const STATUS_SEP_WIDTH: i32 = 13;

const QS_WIDTH: i32 = 368;
const QS_PAD: i32 = glide_theme::spacing::SP3;
const QS_GRID_GAP: i32 = glide_theme::spacing::SP2;
const QS_RADIUS: f32 = R3;
const QS_ROW: i32 = 34;
const QS_ROW_GAP: i32 = SP1;
const QS_DIVIDER: i32 = SP2 * 2 + 1;
const QS_STRIP_GAP: i32 = SP2;
const QS_TILE_HEIGHT: i32 = 56;
const QS_TILE_PAD: i32 = glide_theme::spacing::SP3;
const QS_TILE_PAD_END: i32 = SP2;
const QS_TILE_ICON: i32 = 20;
const QS_MORE_W: i32 = 32;
const QS_MORE_ICON: i32 = 16;
const QS_HAIRLINE_INSET: i32 = 14;
const QS_SLIDER_TRACK_H: i32 = 6;
const QS_SLIDER_THUMB: i32 = 14;
const QS_SLIDER_MARGIN: i32 = SP2;
const QS_READOUT_W: i32 = 40;
const QS_DRILL: i32 = 28;
const QS_ICON_BTN: i32 = 34;
const QS_CHIP_PAD: i32 = 10;
const QS_CONFIRM_PILL: i32 = 28;
const QS_CONFIRM_PILL_PAD: i32 = 11;
const QS_PAGE_MAX: i32 = 640;

const POPUP_GAP: i32 = glide_theme::spacing::SP2;
const SHADOW_CACHE_LIMIT: usize = 48;
const POPUP_RADIUS: f32 = glide_theme::shape::R3;
const POPUP_RISE: f64 = 6.0;
const POPUP_SCALE_FROM: f64 = 0.94;
const BADGE_SCALE_FROM: f64 = 0.6;
const BADGE_QUANTUM: f64 = 16.0;
const MOTION_QUANTUM: f64 = 32.0;

#[derive(Debug, Clone, Copy)]
struct Anim {
    tween: Tween,
    duration: Duration,
    ease: CubicBezier,
}

impl Anim {
    fn at(value: f64) -> Self {
        Self { tween: Tween::at(value), duration: Duration::ZERO, ease: E_OUT }
    }

    fn target(&self) -> f64 {
        self.tween.target()
    }

    fn retarget(&mut self, to: f64, duration: Duration, ease: CubicBezier) -> bool {
        if (self.target() - to).abs() < f64::EPSILON {
            return false;
        }
        let current = self.value();
        self.tween.jump(current);
        self.duration = duration;
        self.ease = ease;
        self.tween.retarget(to, duration, |t| ease.ease_f64(t))
    }

    fn jump(&mut self, to: f64) {
        self.tween.jump(to);
    }

    fn value(&self) -> f64 {
        let ease = self.ease;
        self.tween.value(self.duration, |t| ease.ease_f64(t))
    }

    fn value_with(&self, ease: CubicBezier) -> f64 {
        self.tween.value(self.duration, |t| ease.ease_f64(t))
    }

    fn is_animating(&self) -> bool {
        self.tween.is_animating(self.duration)
    }

    fn switch(&self) -> SwitchMotion {
        SwitchMotion { thumb: self.value(), track: self.value_with(E_OUT) }
    }
}

impl Default for Anim {
    fn default() -> Self {
        Self::at(0.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct SwitchMotion {
    thumb: f64,
    track: f64,
}

impl SwitchMotion {
    fn key(self) -> (u8, u8) {
        (quantize(self.thumb, MOTION_QUANTUM), quantize(self.track, MOTION_QUANTUM))
    }

    fn thumb_x(self, track_x: i32, width: i32, inset: i32, thumb: i32) -> i32 {
        switch_thumb_x(track_x, width, inset, thumb, self.thumb)
    }

    fn track_colour(self, off: Color, on: Color) -> Color {
        off.lerp(on, self.track as f32)
    }
}

fn quantize(value: f64, steps: f64) -> u8 {
    (value.clamp(0.0, 2.0) * steps).round() as u8
}

fn switch_thumb_x(track_x: i32, width: i32, inset: i32, thumb: i32, t: f64) -> i32 {
    let off = f64::from(track_x + inset);
    let on = f64::from(track_x + width - inset - thumb);
    let x = lerp(off, on, t).round() as i32;
    x.clamp(track_x, track_x + width - thumb)
}

fn segment_pill(first: (i32, i32), second: (i32, i32), t: f64) -> (i32, i32) {
    let t = t.clamp(0.0, 1.0);
    let x = lerp(f64::from(first.0), f64::from(second.0), t).round() as i32;
    let w = lerp(f64::from(first.1), f64::from(second.1), t).round() as i32;
    (x, w)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenPopup {
    NotificationCenter,
    LanguageMenu,
    QuickSettings,
    MediaPlayer,
    AppMenu,
    Privacy,
    Headset,
    Phone,
    Passwords,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Drawer {
    Output,
    Input,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MenuRowOutcome {
    Submenu,
    Activate(glide_appmenu::Activation),
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopbarAction {
    OpenAppMenu(u16),
    AppMenuRow(MenuPath),
    MediaPlayPause,
    MediaNext,
    MediaPrevious,
    MediaSeek(u8),
    ToggleMediaPlayer,
    CycleLayout,
    SelectLayout(usize),
    ToggleCaffeine,
    ToggleDoNotDisturb,
    ToggleNotificationCenter,
    ToggleLanguageMenu,
    ToggleQuickSettings,
    ClearAllNotifications,
    ActivateNotification(u32),
    DismissNotification(u32),
    OpenInputSettings,
    ClosePopup,
    DismissIslandAlert,
    Notification(crate::notifications::NotificationAction),
    PeekNotice,
    QsToggleWifi,
    QsToggleNetworking,
    QsToggleBluetooth,
    QsToggleVpn,
    QsToggleNightLight,
    QsToggleAirplane,
    QsToggleDarkTheme,
    QsSetPowerProfile(PowerProfile),
    QsVolumeMute,
    QsVolumeAdjust(u8),
    QsMicMute,
    QsMicAdjust(u8),
    QsUseDevice(Drawer, u16),
    QsBrightnessAdjust(u8),
    QsScreenshot,
    QsOpenSettings,
    QsArmPower(PowerAction),
    QsDisarmPower,
    QsPower(PowerAction),
    QsPanel(PanelAction),
    ToggleLauncher,
    TogglePrivacy,
    ToggleHeadset,
    HeadsetUseOutput,
    HeadsetDisconnect,
    HeadsetSettings,
    TogglePhone,
    PhoneRing,
    PhoneAcceptPair,
    PhoneRejectPair,
    PhoneClearNotifications,
    PhoneOpenApp,
    TogglePasswords,
    PasswordsField,
    PasswordsRow(u8),
    PasswordsMenu(u8),
    PasswordsGenerate,
    PasswordsOpen,
    PasswordsUnlock,
    PasswordsCreate,
    PasswordsConnect,
    CalendarShift(i8),
    CalendarToday,
    Tray(TrayClick),
    Polkit(crate::polkit::Control),
    Keychain(crate::keychain_prompt::Control),
    StopRecording,
}

impl TopbarAction {
    #[must_use]
    pub const fn for_headset(self) -> bool {
        matches!(self, Self::ToggleHeadset | Self::HeadsetUseOutput | Self::HeadsetDisconnect | Self::HeadsetSettings)
    }

    #[must_use]
    pub const fn for_phone(self) -> bool {
        matches!(
            self,
            Self::TogglePhone
                | Self::PhoneRing
                | Self::PhoneAcceptPair
                | Self::PhoneRejectPair
                | Self::PhoneClearNotifications
                | Self::PhoneOpenApp
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Slot {
    MenuTitle(u16),
    MenuRow(MenuPath),
    Clock,
    IslandMedia,
    IslandAlert,
    IslandAlertClose,
    Notice,
    NoticeClose,
    NoticeButton(u8),
    NoticeField,
    NoticeSend,
    Pill,
    IslandToMedia,
    IslandToNotifications,
    IslandCollapse,
    PopMediaPrev,
    PopMediaPlay,
    PopMediaNext,
    PopMediaSeek,
    LayoutCycle,
    LayoutPick(usize),
    LayoutDropdown,
    Caffeine,
    Status,
    LangMenuItem(usize),
    LangMenuSettings,
    NcClearAll,
    NcDndToggle,
    NcItem(u32),
    NcItemDismiss(u32),
    NcItemAction(u32, u8),
    QsWifi,
    QsBluetooth,
    QsNetbird,
    QsSaver,
    QsNightLight,
    QsScreenshot,
    QsAirplane,
    QsDnd,
    QsDarkTheme,
    QsCaffeine,
    QsVolumeMute,
    QsVolumeTrack,
    QsMicMute,
    QsMicTrack,
    QsMicMore,
    QsDrawerItem(Drawer, u16),
    QsBrightnessTrack,
    QsProfile(PowerProfile),
    QsSettings,
    QsLock,
    QsPowerMenu,
    QsPowerButton(PowerAction),
    QsPowerCancel,
    QsPowerConfirm(PowerAction),
    QsWifiMore,
    QsBluetoothMore,
    QsNetbirdMore,
    QsSaverMore,
    QsVolumeMore,
    QsBattery,
    QsBack,
    QsDetailToggle,
    QsDetailRow(u16),
    QsPasswordField,
    QsPromptReveal,
    QsPromptCancel,
    QsPromptConnect,
    QsNetForget,
    QsNetDisconnect,
    QsRescan,
    QsDetailSettings,
    Search,
    Privacy,
    Headset,
    HeadsetOutput,
    HeadsetDisconnect,
    HeadsetSettings,
    Phone,
    PhoneRing,
    PhoneAcceptPair,
    PhoneRejectPair,
    PhoneClearNotifs,
    PhoneOpenApp,
    Passwords,
    PwField,
    PwRow(u8),
    PwMenu(u8),
    PwGenerate,
    PwOpen,
    PwUnlock,
    PwCreate,
    PwConnect,
    CalMonth,
    CalPrev,
    CalNext,
    Tray(u16),
    Polkit(crate::polkit::Control),
    Keychain(crate::keychain_prompt::Control),
    StopRecording,
}

impl Slot {
    #[must_use]
    pub const fn is_slider(self) -> bool {
        matches!(self, Self::QsVolumeTrack | Self::QsMicTrack | Self::QsBrightnessTrack)
    }

    #[must_use]
    pub const fn on_notice(self) -> bool {
        matches!(
            self,
            Self::Notice
                | Self::NoticeClose
                | Self::NoticeButton(_)
                | Self::NoticeField
                | Self::NoticeSend
                | Self::Pill
        )
    }

    const fn in_status_cluster(self) -> bool {
        matches!(
            self,
            Self::Status
                | Self::Search
                | Self::Privacy
                | Self::Headset
                | Self::Phone
                | Self::Passwords
                | Self::StopRecording
        )
    }

    const fn in_headset_popup(self) -> bool {
        matches!(self, Self::HeadsetOutput | Self::HeadsetDisconnect | Self::HeadsetSettings)
    }

    const fn in_phone_popup(self) -> bool {
        matches!(
            self,
            Self::PhoneRing
                | Self::PhoneAcceptPair
                | Self::PhoneRejectPair
                | Self::PhoneClearNotifs
                | Self::PhoneOpenApp
        )
    }

    #[must_use]
    pub const fn in_passwords_popup(self) -> bool {
        matches!(
            self,
            Self::PwField
                | Self::PwRow(_)
                | Self::PwMenu(_)
                | Self::PwGenerate
                | Self::PwOpen
                | Self::PwUnlock
                | Self::PwCreate
                | Self::PwConnect
        )
    }

    const fn in_strip(self) -> bool {
        self.in_status_cluster()
            || self.on_notice()
            || matches!(
                self,
                Self::MenuTitle(_)
                    | Self::Clock
                    | Self::IslandMedia
                    | Self::IslandAlert
                    | Self::IslandAlertClose
                    | Self::LayoutCycle
                    | Self::LayoutPick(_)
                    | Self::LayoutDropdown
                    | Self::Caffeine
                    | Self::Tray(_)
                    | Self::Polkit(_)
                    | Self::Keychain(_)
            )
    }

    const fn in_media_popup(self) -> bool {
        matches!(
            self,
            Self::PopMediaPrev
                | Self::PopMediaPlay
                | Self::PopMediaNext
                | Self::PopMediaSeek
                | Self::IslandToNotifications
                | Self::IslandCollapse
        )
    }

    const fn in_layout(self) -> bool {
        matches!(self, Self::LayoutCycle | Self::LayoutPick(_) | Self::LayoutDropdown)
    }

    const fn in_lang_menu(self) -> bool {
        matches!(self, Self::LangMenuItem(_) | Self::LangMenuSettings)
    }

    const fn in_notification_center(self) -> bool {
        matches!(
            self,
            Self::NcClearAll
                | Self::NcDndToggle
                | Self::NcItem(_)
                | Self::NcItemDismiss(_)
                | Self::NcItemAction(_, _)
                | Self::CalMonth
                | Self::CalPrev
                | Self::CalNext
                | Self::IslandToMedia
                | Self::IslandCollapse
        )
    }

    const fn in_app_menu(self) -> bool {
        matches!(self, Self::MenuRow(_))
    }

    const fn in_quick_settings(self) -> bool {
        !self.in_strip()
            && !self.in_media_popup()
            && !self.in_lang_menu()
            && !self.in_notification_center()
            && !self.in_app_menu()
            && !self.in_headset_popup()
            && !self.in_phone_popup()
            && !self.in_passwords_popup()
    }

    const fn state_layer_radius(self) -> Option<f32> {
        match self {
            Self::NcDndToggle
            | Self::QsVolumeTrack
            | Self::QsMicTrack
            | Self::QsBrightnessTrack
            | Self::QsDetailToggle
            | Self::QsPasswordField
            | Self::IslandAlert
            | Self::Notice
            | Self::NoticeField
            | Self::Pill
            | Self::PopMediaSeek => None,
            Self::Clock
            | Self::IslandMedia
            | Self::IslandAlertClose
            | Self::NoticeClose
            | Self::NoticeSend
            | Self::IslandToMedia
            | Self::IslandToNotifications
            | Self::IslandCollapse
            | Self::NcClearAll
            | Self::CalMonth
            | Self::CalPrev
            | Self::CalNext
            | Self::QsPromptReveal => Some(glide_theme::shape::R1),
            Self::LayoutPick(_) | Self::QsProfile(_) => Some(SEG_RADIUS),
            Self::PopMediaPlay | Self::NcItemDismiss(_) => Some(glide_theme::shape::R_ROUND),
            Self::Polkit(control) => polkit_view::layer_radius(control),
            Self::Keychain(control) => keychain_view::layer_radius(control),
            _ => Some(glide_theme::shape::R2),
        }
    }

    const fn action(self) -> TopbarAction {
        match self {
            Self::MenuTitle(index) => TopbarAction::OpenAppMenu(index),
            Self::MenuRow(path) => TopbarAction::AppMenuRow(path),
            Self::Clock
            | Self::IslandAlert
            | Self::IslandToNotifications
            | Self::Notice
            | Self::NoticeClose
            | Self::NoticeButton(_)
            | Self::NoticeField
            | Self::NoticeSend
            | Self::Pill => TopbarAction::ToggleNotificationCenter,
            Self::IslandMedia | Self::IslandToMedia => TopbarAction::ToggleMediaPlayer,
            Self::IslandAlertClose => TopbarAction::DismissIslandAlert,
            Self::IslandCollapse => TopbarAction::ClosePopup,
            Self::PopMediaPlay => TopbarAction::MediaPlayPause,
            Self::PopMediaPrev => TopbarAction::MediaPrevious,
            Self::PopMediaNext => TopbarAction::MediaNext,
            Self::PopMediaSeek => TopbarAction::MediaSeek(0),
            Self::LayoutCycle => TopbarAction::CycleLayout,
            Self::LayoutPick(index) | Self::LangMenuItem(index) => TopbarAction::SelectLayout(index),
            Self::LayoutDropdown => TopbarAction::ToggleLanguageMenu,
            Self::Caffeine | Self::QsCaffeine => TopbarAction::ToggleCaffeine,
            Self::NcDndToggle | Self::QsDnd => TopbarAction::ToggleDoNotDisturb,
            Self::Status => TopbarAction::ToggleQuickSettings,
            Self::LangMenuSettings => TopbarAction::OpenInputSettings,
            Self::NcClearAll => TopbarAction::ClearAllNotifications,
            Self::NcItem(id) => TopbarAction::ActivateNotification(id),
            Self::NcItemDismiss(id) => TopbarAction::DismissNotification(id),
            Self::NcItemAction(id, index) => {
                TopbarAction::Notification(crate::notifications::NotificationAction::Button(id, index as usize))
            }
            Self::QsWifi => TopbarAction::QsToggleWifi,
            Self::QsBluetooth => TopbarAction::QsToggleBluetooth,
            Self::QsNetbird => TopbarAction::QsToggleVpn,
            Self::QsNightLight => TopbarAction::QsToggleNightLight,
            Self::QsScreenshot => TopbarAction::QsScreenshot,
            Self::QsAirplane => TopbarAction::QsToggleAirplane,
            Self::QsDarkTheme => TopbarAction::QsToggleDarkTheme,
            Self::QsVolumeMute => TopbarAction::QsVolumeMute,
            Self::QsVolumeTrack => TopbarAction::QsVolumeAdjust(50),
            Self::QsMicMute => TopbarAction::QsMicMute,
            Self::QsMicTrack => TopbarAction::QsMicAdjust(50),
            Self::QsDrawerItem(drawer, index) => TopbarAction::QsUseDevice(drawer, index),
            Self::QsBrightnessTrack => TopbarAction::QsBrightnessAdjust(50),
            Self::QsProfile(profile) => TopbarAction::QsSetPowerProfile(profile),
            Self::QsSaver => TopbarAction::QsSetPowerProfile(PowerProfile::PowerSaver),
            Self::QsSettings => TopbarAction::QsOpenSettings,
            Self::QsLock => TopbarAction::QsPower(PowerAction::Lock),
            Self::QsPowerMenu => TopbarAction::QsPanel(PanelAction::Open(Detail::Power)),
            Self::QsPowerButton(action) if is_destructive(action) => TopbarAction::QsArmPower(action),
            Self::QsPowerButton(action) | Self::QsPowerConfirm(action) => TopbarAction::QsPower(action),
            Self::QsPowerCancel => TopbarAction::QsDisarmPower,
            Self::QsWifiMore => TopbarAction::QsPanel(PanelAction::Open(Detail::Wifi)),
            Self::QsBluetoothMore => TopbarAction::QsPanel(PanelAction::Open(Detail::Bluetooth)),
            Self::QsNetbirdMore => TopbarAction::QsPanel(PanelAction::Open(Detail::Vpn)),
            Self::QsVolumeMore | Self::QsMicMore => TopbarAction::QsPanel(PanelAction::Open(Detail::Sound)),
            Self::QsBattery | Self::QsSaverMore => TopbarAction::QsPanel(PanelAction::Open(Detail::Battery)),
            Self::QsBack => TopbarAction::QsPanel(PanelAction::Back),
            Self::QsDetailToggle => TopbarAction::QsPanel(PanelAction::Toggle),
            Self::QsDetailRow(index) => TopbarAction::QsPanel(PanelAction::Row(index)),
            Self::QsPasswordField => TopbarAction::QsPanel(PanelAction::FocusPassword),
            Self::QsPromptReveal => TopbarAction::QsPanel(PanelAction::PromptReveal),
            Self::QsPromptCancel => TopbarAction::QsPanel(PanelAction::PromptCancel),
            Self::QsPromptConnect => TopbarAction::QsPanel(PanelAction::PromptSubmit),
            Self::QsNetForget => TopbarAction::QsPanel(PanelAction::Forget),
            Self::QsNetDisconnect => TopbarAction::QsPanel(PanelAction::Disconnect),
            Self::QsRescan => TopbarAction::QsPanel(PanelAction::Rescan),
            Self::QsDetailSettings => TopbarAction::QsPanel(PanelAction::Settings),
            Self::Search => TopbarAction::ToggleLauncher,
            Self::Privacy => TopbarAction::TogglePrivacy,
            Self::Headset => TopbarAction::ToggleHeadset,
            Self::HeadsetOutput => TopbarAction::HeadsetUseOutput,
            Self::HeadsetDisconnect => TopbarAction::HeadsetDisconnect,
            Self::HeadsetSettings => TopbarAction::HeadsetSettings,
            Self::Phone => TopbarAction::TogglePhone,
            Self::PhoneRing => TopbarAction::PhoneRing,
            Self::PhoneAcceptPair => TopbarAction::PhoneAcceptPair,
            Self::PhoneRejectPair => TopbarAction::PhoneRejectPair,
            Self::PhoneClearNotifs => TopbarAction::PhoneClearNotifications,
            Self::PhoneOpenApp => TopbarAction::PhoneOpenApp,
            Self::Passwords => TopbarAction::TogglePasswords,
            Self::PwField => TopbarAction::PasswordsField,
            Self::PwRow(index) => TopbarAction::PasswordsRow(index),
            Self::PwMenu(index) => TopbarAction::PasswordsMenu(index),
            Self::PwGenerate => TopbarAction::PasswordsGenerate,
            Self::PwOpen => TopbarAction::PasswordsOpen,
            Self::PwUnlock => TopbarAction::PasswordsUnlock,
            Self::PwCreate => TopbarAction::PasswordsCreate,
            Self::PwConnect => TopbarAction::PasswordsConnect,
            Self::CalMonth => TopbarAction::CalendarToday,
            Self::CalPrev => TopbarAction::CalendarShift(-1),
            Self::CalNext => TopbarAction::CalendarShift(1),
            Self::Tray(id) => TopbarAction::Tray(TrayClick { id, button: TrayButton::Primary }),
            Self::Polkit(control) => TopbarAction::Polkit(control),
            Self::Keychain(control) => TopbarAction::Keychain(control),
            Self::StopRecording => TopbarAction::StopRecording,
        }
    }

    #[must_use]
    pub const fn accessible_key(self) -> Option<Key> {
        match self {
            Self::StopRecording => Some(Key::TbScreenRecording),
            _ => None,
        }
    }
}

type ScaledBuffers = std::collections::HashMap<i32, MemoryRenderBuffer>;
type QsCacheKey = ((u64, u64, String), QsFlags, QsMotion, Option<Slot>, QsShape);
type LayerKey = (u8, i32, i32, u32, i32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layer {
    Hover,
    Press,
    Focus,
}

impl Layer {
    const fn tag(self) -> u8 {
        match self {
            Self::Hover => 0,
            Self::Press => 1,
            Self::Focus => 2,
        }
    }

    const fn colour(self, theme: &TopbarTheme) -> Color {
        match self {
            Self::Hover => theme.button_hover,
            Self::Press => theme.button_press,
            Self::Focus => theme.active_background,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Badge {
    count: usize,
    reserved: bool,
}

#[derive(Debug)]
struct Cache<K> {
    key: Option<K>,
    buffers: ScaledBuffers,
}

impl<K> Default for Cache<K> {
    fn default() -> Self {
        Self { key: None, buffers: ScaledBuffers::new() }
    }
}

impl<K: PartialEq> Cache<K> {
    fn clear(&mut self) {
        self.key = None;
        self.buffers.clear();
    }

    fn resolve(
        &mut self, key: K, scale: i32, draw: impl FnOnce() -> Option<MemoryRenderBuffer>,
    ) -> Option<&MemoryRenderBuffer> {
        if self.key.as_ref() != Some(&key) {
            self.key = Some(key);
            self.buffers.clear();
        }
        if let std::collections::hash_map::Entry::Vacant(entry) = self.buffers.entry(scale) {
            entry.insert(draw()?);
        }
        self.buffers.get(&scale)
    }
}

#[derive(Debug, Default)]
pub struct Topbar {
    theme: TopbarTheme,
    date: String,
    clock: String,
    unread: usize,
    layouts: Vec<String>,
    layout_index: usize,
    app_name: String,
    app_id: String,
    caffeinated: bool,
    dnd: bool,
    open_popup: Option<OpenPopup>,
    closing_popup: Option<OpenPopup>,
    popup_reveal: Reveal,
    nc_items: Vec<crate::notifications::NcItem>,
    media: Option<MediaWidget>,
    system_status: crate::system_info::SystemStatus,
    hovered: Option<Slot>,
    pressed: Option<Slot>,
    focused: Option<Slot>,
    focus_ring_last: Option<Rectangle<i32, Logical>>,
    qs_panel: panels::DetailPanel,

    hover_layers: HashMap<Slot, Anim>,
    press_layers: HashMap<Slot, Anim>,
    focus_anim: Anim,
    caffeine_anim: Anim,
    dnd_anim: Anim,
    volume_anim: Anim,
    mic_anim: Anim,
    brightness_anim: Anim,
    seek_anim: Anim,
    badge_anim: Anim,
    badge: Badge,
    segment_anim: Anim,
    profile_anim: Anim,
    tile_anims: HashMap<QsTile, Anim>,
    menu_births: HashMap<MenuPath, Anim>,
    slider_dragging: bool,
    scroll: crate::system_info::ScrollAccumulator,
    recording_started: Option<Instant>,
    recording_anim: Anim,
    recording_last_sec: Option<u64>,

    rev: u64,
    layouts_rev: u64,
    media_rev: u64,
    track_rev: u64,
    status_rev: u64,
    headsets: Vec<crate::headset::Headset>,
    phone: Option<crate::phone::PhoneState>,
    detail_rev: u64,
    nc_rev: u64,

    slots: Vec<(Slot, Rectangle<i32, Logical>)>,
    shown_popup_slots: Vec<(Slot, Rectangle<i32, Logical>)>,
    layout_key: Option<(u64, Rectangle<i32, Logical>)>,
    popup_rect: Option<Rectangle<i32, Logical>>,
    panel: std::collections::HashMap<PanelKey, MemoryRenderBuffer>,
    shadows: std::collections::HashMap<(i32, i32, i32, u32), MemoryRenderBuffer>,
    layers: HashMap<LayerKey, MemoryRenderBuffer>,
    separator_cache: Cache<()>,
    app_cache: Cache<(String, String)>,
    layout_cache: Cache<(u64, usize, Option<Slot>, bool, u8)>,
    coffee_cache: Cache<(bool, bool, (u8, u8))>,
    qs_cache: Cache<QsCacheKey>,
    qs_bg_cache: Cache<(i32, i32)>,
    qs_sliders: HashMap<SliderKey, MemoryRenderBuffer>,
    qs_detail_cache: Cache<panels::DetailKey>,
    headset_cache: Cache<headset_view::HeadsetKey>,
    phone_cache: Cache<phone_view::PhoneKey>,
    passwords: passwords_view::Applet,
    passwords_menu_rect: Option<Rectangle<i32, Logical>>,
    passwords_rect: Option<Rectangle<i32, Logical>>,
    passwords_cache: Cache<passwords_view::PasswordsKey>,
    passwords_menu_cache: Cache<passwords_view::PasswordsKey>,
    lang_menu_cache: Cache<(u64, usize, Option<Slot>)>,
    raster_rev: u64,

    menu_bar: Vec<glide_appmenu::MenuItem>,
    menu_open: MenuPath,
    menu_panels: Vec<crate::global_menu::Panel>,
    menu_titles: HashMap<(usize, bool, bool, bool, i32), MemoryRenderBuffer>,
    menu_rasters: HashMap<(MenuPath, Option<usize>, i32), MemoryRenderBuffer>,
    app_chip_w: i32,

    calendar: calendar::CalendarView,
    island: island::Island,
    indicators: status::Indicators,
    tray: tray::TrayStrip,
    face_tip: face_tip::FaceTip,
    slides: HashMap<(i32, i32), glide_ui::Tween>,
    home: Option<Rectangle<i32, Logical>>,
    placements: HashMap<(i32, i32), Placement>,
}

#[derive(Debug, Default)]
struct Placement {
    key: Option<(u64, Rectangle<i32, Logical>)>,
    slots: Vec<(Slot, Rectangle<i32, Logical>)>,
    tray: tray::Placement,
}

struct Interaction {
    hovered: Option<Slot>,
    pressed: Option<Slot>,
    focused: Option<Slot>,
    open: Option<OpenPopup>,
    closing: Option<OpenPopup>,
}

impl Interaction {
    const fn take(topbar: &mut Topbar) -> Self {
        Self {
            hovered: topbar.hovered.take(),
            pressed: topbar.pressed.take(),
            focused: topbar.focused.take(),
            open: topbar.open_popup.take(),
            closing: topbar.closing_popup.take(),
        }
    }

    const fn restore(self, topbar: &mut Topbar) {
        topbar.hovered = self.hovered;
        topbar.pressed = self.pressed;
        topbar.focused = self.focused;
        topbar.open_popup = self.open;
        topbar.closing_popup = self.closing;
    }
}

type Anchor = Option<Rectangle<i32, Logical>>;

type PanelKey = (i32, i32, u64, (i32, i32, u64));

struct MenuLayout {
    titles: Vec<(Slot, Rectangle<i32, Logical>)>,
    rows: Vec<(Slot, Rectangle<i32, Logical>)>,
    panels: Vec<crate::global_menu::Panel>,
    anchor: Option<Rectangle<i32, Logical>>,
}

impl Topbar {
    #[must_use]
    pub fn new(theme: TopbarTheme) -> Self {
        let mut bar = Self { theme, popup_reveal: Reveal::closed().with_eases(ease_out, ease_io), ..Self::default() };
        bar.volume_anim.jump(f64::from(bar.system_status.volume_pct));
        bar.mic_anim.jump(f64::from(bar.system_status.mic_pct));
        bar.brightness_anim.jump(f64::from(bar.system_status.brightness_pct));
        bar
    }

    pub fn reconfigure(&mut self, theme: TopbarTheme) {
        if self.theme == theme {
            return;
        }
        self.theme = theme;
        self.invalidate();
    }

    fn invalidate(&mut self) {
        self.touch();
        self.panel.clear();
        self.shadows.clear();
        self.layers.clear();
        self.separator_cache.clear();
        self.app_cache.clear();
        self.layout_cache.clear();
        self.coffee_cache.clear();
        self.qs_cache.clear();
        self.qs_bg_cache.clear();
        self.qs_sliders.clear();
        self.qs_detail_cache.clear();
        self.headset_cache.clear();
        self.phone_cache.clear();
        self.passwords_cache.clear();
        self.passwords_menu_cache.clear();
        self.lang_menu_cache.clear();
        self.menu_titles.clear();
        self.menu_rasters.clear();
        self.calendar.clear();
        self.island.clear();
        self.indicators.clear();
    }

    pub fn set_app_menu(&mut self, items: Vec<glide_appmenu::MenuItem>) -> bool {
        if self.menu_bar == items {
            return false;
        }
        let app_owned = self.tray_menu_id().is_none();
        let keep_open = self.open_popup == Some(OpenPopup::AppMenu)
            && crate::global_menu::item_at(&items, self.menu_open.truncated(1))
                .is_some_and(glide_appmenu::MenuItem::is_submenu);
        self.menu_bar = items;
        self.menu_titles.clear();
        self.menu_rasters.clear();
        if !app_owned {
            self.touch();
            return true;
        }
        if keep_open {
            while !self.menu_open.is_empty()
                && !crate::global_menu::item_at(&self.menu_bar, self.menu_open)
                    .is_some_and(glide_appmenu::MenuItem::is_submenu)
            {
                match self.menu_open.parent() {
                    Some(parent) => self.menu_open = parent,
                    None => break,
                }
            }
        } else if self.open_popup == Some(OpenPopup::AppMenu) {
            self.close_popup();
        }
        self.touch();
        true
    }

    #[must_use]
    pub fn app_menu(&self) -> &[glide_appmenu::MenuItem] {
        &self.menu_bar
    }

    #[must_use]
    pub fn app_menu_item(&self, path: MenuPath) -> Option<&glide_appmenu::MenuItem> {
        crate::global_menu::item_at(self.menu_items(), path)
    }

    pub fn open_app_menu(&mut self, index: usize) -> bool {
        if !crate::global_menu::item_at(&self.menu_bar, MenuPath::top(index))
            .is_some_and(glide_appmenu::MenuItem::is_submenu)
        {
            return false;
        }
        if self.tray_menu_id().is_some() {
            self.release_tray_menu();
            if self.open_popup == Some(OpenPopup::AppMenu) {
                self.menu_open = MenuPath::top(index);
                self.set_focused(None);
                self.touch();
                return true;
            }
        }
        if self.open_popup == Some(OpenPopup::AppMenu) {
            if self.menu_open.top_index() == index {
                return self.close_popup();
            }
            self.menu_open = MenuPath::top(index);
            self.set_focused(None);
            self.touch();
            return true;
        }
        self.menu_open = MenuPath::top(index);
        self.toggle_popup(OpenPopup::AppMenu)
    }

    pub fn app_menu_row(&mut self, path: MenuPath) -> MenuRowOutcome {
        let Some(item) = crate::global_menu::item_at(self.menu_items(), path) else {
            return MenuRowOutcome::Ignored;
        };
        if !item.enabled || item.separator {
            return MenuRowOutcome::Ignored;
        }
        if item.is_submenu() {
            if self.menu_open != path {
                self.menu_open = path;
                self.touch();
            }
            return MenuRowOutcome::Submenu;
        }
        let activation = item.activation.clone();
        self.close_popup();
        MenuRowOutcome::Activate(activation)
    }

    fn follow_menu_hover(&mut self, hovered: Option<Slot>) {
        if self.open_popup != Some(OpenPopup::AppMenu) {
            return;
        }
        let next = match hovered {
            Some(Slot::MenuTitle(index))
                if usize::from(index) != self.menu_open.top_index() && self.tray_menu_id().is_none() =>
            {
                self.set_focused(None);
                Some(MenuPath::top(usize::from(index)))
            }
            Some(Slot::MenuRow(path)) => {
                let submenu = crate::global_menu::item_at(self.menu_items(), path)
                    .is_some_and(glide_appmenu::MenuItem::is_submenu);
                if submenu { Some(path) } else { path.parent() }
            }
            _ => None,
        };
        if let Some(next) = next
            && next != self.menu_open
        {
            self.menu_open = next;
            self.touch();
        }
    }

    fn app_menu_nav(&mut self, dx: i32, dy: i32) -> bool {
        let focused = match self.focused {
            Some(Slot::MenuRow(path)) => Some(path),
            _ => None,
        };
        let panel_parent = focused.and_then(MenuPath::parent).unwrap_or_else(|| self.menu_open.truncated(1));
        if dy != 0 {
            let Some(panel) = self.menu_panels.iter().find(|p| p.parent == panel_parent) else {
                return false;
            };
            let Some(next) = crate::global_menu::step_enabled(panel, focused, dy > 0) else {
                return false;
            };
            self.menu_open = panel_parent;
            self.set_focused(Some(Slot::MenuRow(next)));
            self.touch();
            return true;
        }
        let submenu = focused.filter(|path| {
            crate::global_menu::item_at(self.menu_items(), *path).is_some_and(glide_appmenu::MenuItem::is_submenu)
        });
        if dx > 0
            && let Some(path) = submenu
        {
            self.menu_open = path;
            let first = crate::global_menu::item_at(self.menu_items(), path)
                .and_then(|item| item.children.iter().position(|c| c.enabled && !c.separator))
                .and_then(|index| path.child(index));
            self.set_focused(first.map(Slot::MenuRow));
            self.touch();
            return true;
        }
        if dx < 0
            && let Some(path) = focused.filter(|path| path.len() > 2)
            && let Some(parent) = path.parent()
        {
            self.menu_open = parent.parent().unwrap_or(parent);
            self.set_focused(Some(Slot::MenuRow(parent)));
            self.touch();
            return true;
        }
        let count = self.menu_items().len();
        if count == 0 {
            return false;
        }
        let step = if dx > 0 { 1 } else { count - 1 };
        let mut index = self.menu_open.top_index();
        for _ in 0..count {
            index = (index + step) % count;
            if self.menu_items()[index].is_submenu() {
                break;
            }
        }
        self.menu_open = MenuPath::top(index);
        self.set_focused(None);
        self.touch();
        true
    }

    const fn touch(&mut self) {
        self.rev = self.rev.wrapping_add(1);
    }

    fn hovered_in(&self, pred: fn(Slot) -> bool) -> Option<Slot> {
        self.hovered.filter(|slot| pred(*slot))
    }

    fn focused_in(&self, pred: fn(Slot) -> bool) -> Option<Slot> {
        self.focused.filter(|slot| pred(*slot))
    }

    fn focus_ring_in(&self, pred: fn(Slot) -> bool) -> Option<LocalRect> {
        let popup = self.popup_rect?;
        let rect = self.slot_rect(self.focused_in(pred)?)?;
        Some(Rectangle::new(rect.loc - popup.loc, rect.size))
    }

    fn focus_ring_rect(&self) -> Option<Rectangle<i32, Logical>> {
        let popup = self.popup_rect?;
        let local = self.focus_ring_in(popup_slots(self.open_popup?))?;
        Some(Rectangle::new(local.loc + popup.loc, local.size))
    }

    fn set_focused(&mut self, slot: Option<Slot>) {
        self.focused = slot;
        self.focus_anim.retarget(if slot.is_some() { 1.0 } else { 0.0 }, motion::t1(), E_OUT);
    }

    #[must_use]
    pub const fn has_focus(&self) -> bool {
        self.focused.is_some()
    }

    #[must_use]
    pub fn next_wake(&self) -> Option<std::time::Instant> {
        let mut wake = self.indicators.next_wake();
        if let Some(started) = self.recording_started
            && !self.is_animating()
        {
            let now = glide_ui::clock::now();
            let elapsed_secs = now.saturating_duration_since(started).as_secs();
            let next_tick = started + Duration::from_secs(elapsed_secs + 1);
            wake = match wake {
                Some(earliest) => Some(earliest.min(next_tick)),
                None => Some(next_tick),
            };
        }
        wake
    }

    #[must_use]
    pub fn is_animating(&self) -> bool {
        let popup = self.popup_reveal.is_animating(motion::t2());
        let layers = self.hover_layers.values().chain(self.press_layers.values()).any(Anim::is_animating);
        let controls = [
            &self.focus_anim,
            &self.caffeine_anim,
            &self.dnd_anim,
            &self.volume_anim,
            &self.mic_anim,
            &self.brightness_anim,
            &self.seek_anim,
            &self.badge_anim,
            &self.segment_anim,
            &self.profile_anim,
            &self.recording_anim,
        ]
        .into_iter()
        .any(Anim::is_animating);
        let controls = controls
            || self.tile_anims.values().any(Anim::is_animating)
            || self.menu_births.values().any(Anim::is_animating);
        let detail = self.qs_panel.is_animating();
        let extras = self.calendar.is_animating() || self.indicators.is_animating() || self.island.is_animating();
        let tray = self.tray_is_animating() || self.face_tip.is_animating();
        let sliding = self.slides.values().any(|slide| slide.is_animating(motion::t3()));
        popup || layers || controls || detail || extras || tray || sliding || self.pending_settle()
    }

    pub fn slide_away(&mut self, output: Rectangle<i32, Logical>, hidden: bool) -> f64 {
        let target = if hidden { 1.0 } else { 0.0 };
        let slide = self.slides.entry((output.loc.x, output.loc.y)).or_insert_with(|| glide_ui::Tween::at(target));
        slide.retarget(target, motion::t3(), motion::ease_out);
        slide.value(motion::t3(), motion::ease_out)
    }

    #[must_use]
    pub const fn home(&self) -> Option<Rectangle<i32, Logical>> {
        self.home
    }

    #[must_use]
    pub fn is_home(&self, output: Rectangle<i32, Logical>) -> bool {
        self.home.is_none_or(|home| home == output)
    }

    fn home_held(&self) -> bool {
        self.shown_popup().is_some() || self.island.view.dims() || self.slider_dragging
    }

    pub fn set_home(&mut self, output: Rectangle<i32, Logical>) -> bool {
        if self.home == Some(output) || (self.home.is_some() && self.home_held()) {
            return false;
        }
        self.move_home(output);
        true
    }

    pub fn reset_home(&mut self, output: Rectangle<i32, Logical>) {
        if self.home != Some(output) {
            self.move_home(output);
        }
    }

    fn move_home(&mut self, output: Rectangle<i32, Logical>) {
        self.set_hover(None);
        self.set_pressed(None);
        let mut placement = self.placements.remove(&(output.loc.x, output.loc.y)).unwrap_or_default();
        self.swap_placement(&mut placement);
        self.layout_key = None;
        self.shown_popup_slots.clear();
        self.popup_rect = None;
        self.menu_panels.clear();
        self.home = Some(output);
    }

    fn with_passive<T>(&mut self, output: Rectangle<i32, Logical>, f: impl FnOnce(&mut Self) -> T) -> T {
        let at = (output.loc.x, output.loc.y);
        let mut placement = self.placements.remove(&at).unwrap_or_default();
        let interaction = Interaction::take(self);
        let mut tooltip = None;
        self.tray.swap_tooltip(&mut tooltip);
        self.swap_placement(&mut placement);
        let result = f(self);
        self.swap_placement(&mut placement);
        self.tray.swap_tooltip(&mut tooltip);
        interaction.restore(self);
        self.placements.insert(at, placement);
        result
    }

    fn swap_placement(&mut self, placement: &mut Placement) {
        std::mem::swap(&mut self.layout_key, &mut placement.key);
        std::mem::swap(&mut self.slots, &mut placement.slots);
        self.tray.swap_placement(&mut placement.tray);
    }

    fn pending_settle(&self) -> bool {
        self.closing_popup.is_some()
            || (self.unread == 0 && self.badge.reserved)
            || (self.recording_started.is_none() && self.indicators.recording_reserved)
    }

    pub fn set_pressed(&mut self, slot: Option<Slot>) -> bool {
        if self.pressed == slot {
            return false;
        }
        if let Some(previous) = self.pressed.take()
            && let Some(layer) = self.press_layers.get_mut(&previous)
        {
            layer.retarget(0.0, motion::t1(), E_OUT);
        }
        if let Some(slot) = slot {
            self.press_layers.entry(slot).or_default().jump(1.0);
            if slot == Slot::StopRecording && self.open_popup.is_some() {
                self.close_popup();
            }
        }
        self.pressed = slot;
        true
    }

    pub const fn set_slider_dragging(&mut self, on: bool) {
        self.slider_dragging = on;
    }

    pub const fn keep_dragged_levels(&self, sample: &mut crate::system_info::SystemStatus) {
        if !self.slider_dragging {
            return;
        }
        let shown = &self.system_status;
        (sample.volume_pct, sample.volume_muted) = (shown.volume_pct, shown.volume_muted);
        (sample.mic_pct, sample.mic_muted) = (shown.mic_pct, shown.mic_muted);
        sample.brightness_pct = shown.brightness_pct;
    }

    fn slide(&mut self, which: fn(&mut Self) -> &mut Anim, pct: u8) {
        let target = f64::from(pct);
        let dragging = self.slider_dragging;
        let anim = which(self);
        if dragging {
            anim.jump(target);
        } else {
            anim.retarget(target, motion::t2(), E_OUT);
        }
    }

    fn prune_layers(&mut self) {
        let settled = |anim: &Anim| anim.target() <= 0.0 && !anim.is_animating();
        self.hover_layers.retain(|_, anim| !settled(anim));
        self.press_layers.retain(|_, anim| !settled(anim));
    }

    fn shown_popup(&self) -> Option<OpenPopup> {
        self.open_popup.or(self.closing_popup)
    }

    fn popup_progress(&self) -> f64 {
        self.popup_reveal.value(motion::t2())
    }

    fn tile_on(&self, tile: QsTile) -> bool {
        let status = &self.system_status;
        match tile {
            QsTile::Network if status.wifi_available => status.wifi_enabled && !status.airplane,
            QsTile::Network => status.ethernet_connected,
            QsTile::Bluetooth => status.bluetooth_on,
            QsTile::Vpn => {
                self.qs_panel.netbird.tile().0 || status.vpn_active || self.qs_panel.vpns().iter().any(|vpn| vpn.active)
            }
            QsTile::Saver => status.power_profile == Some(PowerProfile::PowerSaver),
            QsTile::NightLight => status.night_light_on,
            QsTile::Dnd => self.dnd,
            QsTile::Caffeine => self.caffeinated,
            QsTile::DarkTheme => self.theme.dark,
        }
    }

    fn tile_level(&self, tile: QsTile) -> f64 {
        self.tile_anims.get(&tile).map_or_else(|| f64::from(self.tile_on(tile)), Anim::value)
    }

    fn tile_levels(&self) -> [u8; QS_TILE_COUNT] {
        let mut levels = [0; QS_TILE_COUNT];
        for tile in QS_TILES {
            levels[tile.index()] = quantize(self.tile_level(tile), MOTION_QUANTUM);
        }
        levels
    }

    fn sync_tile_anims(&mut self) {
        for tile in QS_TILES {
            let target = f64::from(self.tile_on(tile));
            self.tile_anims.entry(tile).or_insert_with(|| Anim::at(target)).retarget(target, motion::t2(), E_OUT);
        }
    }

    fn settle_motion(&mut self) {
        self.sync_tile_anims();
        let duration = motion::t2();
        self.popup_reveal.settle(duration);
        if self.closing_popup.is_some() && !self.popup_reveal.is_visible(duration) {
            self.closing_popup = None;
            self.forget_power();
            self.touch();
        }
        let reserved = self.unread > 0 || self.badge_anim.value() > 0.0;
        if reserved != self.badge.reserved {
            self.badge.reserved = reserved;
            self.touch();
        }
        if let Some(rect) = self.focus_ring_rect() {
            self.focus_ring_last = Some(rect);
        }
        if let Some(started) = self.recording_started {
            let sec = glide_ui::clock::now().saturating_duration_since(started).as_secs();
            if self.recording_last_sec != Some(sec) {
                let hour_before = self.recording_last_sec.is_some_and(|s| s >= 3600);
                let hour_now = sec >= 3600;
                self.recording_last_sec = Some(sec);
                self.indicators.rev = self.indicators.rev.wrapping_add(1);
                if hour_before != hour_now {
                    self.touch();
                }
            }
        }
        self.settle_indicators();
        self.tray_settle();
        self.prune_layers();
    }

    #[must_use]
    pub fn focused_action(&self) -> Option<TopbarAction> {
        let slot = self.focused?;
        self.slot_rect(slot)?;
        Some(self.resolve(slot))
    }

    fn resolve(&self, slot: Slot) -> TopbarAction {
        if let Some(action) = self.notice_action(slot) {
            return action;
        }
        match (slot, self.alert()) {
            (Slot::IslandAlert, Some(alert)) => TopbarAction::ActivateNotification(alert.id),
            (Slot::StopRecording, _) => TopbarAction::StopRecording,
            (Slot::QsWifi, _) if !self.system_status.wifi_available => TopbarAction::QsToggleNetworking,
            (Slot::QsSaver, _) => {
                TopbarAction::QsSetPowerProfile(if self.system_status.power_profile == Some(PowerProfile::PowerSaver) {
                    PowerProfile::Balanced
                } else {
                    PowerProfile::PowerSaver
                })
            }
            _ => slot.action(),
        }
    }

    pub fn focus_adjust(&mut self, delta: i8) -> Option<TopbarAction> {
        let slot = self.focused?;
        let current = match slot {
            Slot::QsVolumeTrack => self.system_status.volume_pct,
            Slot::QsMicTrack => self.system_status.mic_pct,
            Slot::QsBrightnessTrack => self.system_status.brightness_pct,
            _ => return None,
        };
        let pct = u8::try_from((i32::from(current) + i32::from(delta)).clamp(0, 100)).ok()?;
        let action = match slot {
            Slot::QsVolumeTrack => {
                self.system_status.volume_pct = pct;
                self.slide(|bar| &mut bar.volume_anim, pct);
                TopbarAction::QsVolumeAdjust(pct)
            }
            Slot::QsMicTrack => {
                self.system_status.mic_pct = pct;
                self.slide(|bar| &mut bar.mic_anim, pct);
                TopbarAction::QsMicAdjust(pct)
            }
            _ => {
                self.system_status.brightness_pct = pct;
                self.slide(|bar| &mut bar.brightness_anim, pct);
                TopbarAction::QsBrightnessAdjust(pct)
            }
        };
        self.status_rev = self.status_rev.wrapping_add(1);
        self.touch();
        Some(action)
    }

    pub fn focus_nav(&mut self, dx: i32, dy: i32) -> bool {
        if dx == 0 && dy == 0 {
            return false;
        }
        if self.open_popup == Some(OpenPopup::AppMenu) {
            return self.app_menu_nav(dx, dy);
        }
        if dx != 0 && self.focused == Some(Slot::CalMonth) && self.open_popup == Some(OpenPopup::NotificationCenter) {
            return self.shift_calendar(dx.signum());
        }
        if dx == 0 && dy > 0 && self.open_popup == Some(OpenPopup::NotificationCenter) {
            let next = match self.focused {
                Some(Slot::NcClearAll) => Some(Slot::NcDndToggle),
                Some(Slot::NcDndToggle) => self.slots.iter().find_map(|(slot, _)| match slot {
                    Slot::NcItem(id) => Some(Slot::NcItem(*id)),
                    _ => None,
                }),
                _ => None,
            };
            if let Some(next) = next {
                self.set_focused(Some(next));
                return true;
            }
        }
        let Some(pred) = self.open_popup.map(popup_slots) else {
            return false;
        };
        let candidates: Vec<(Slot, Rectangle<i32, Logical>)> =
            self.slots.iter().copied().filter(|(slot, _)| pred(*slot)).collect();
        let current = self
            .focused
            .and_then(|focused| candidates.iter().find(|(slot, _)| *slot == focused).map(|(_, rect)| *rect));
        let next = match current {
            None => candidates
                .iter()
                .min_by_key(|(_, rect)| {
                    let centre = rect_centre(*rect);
                    (centre.y, centre.x)
                })
                .map(|(slot, _)| *slot),
            Some(from) => {
                let origin = rect_centre(from);
                candidates
                    .iter()
                    .filter(|(slot, _)| Some(*slot) != self.focused)
                    .filter_map(|(slot, rect)| {
                        let centre = rect_centre(*rect);
                        let (step_x, step_y) = (centre.x - origin.x, centre.y - origin.y);
                        let beneath = slot.is_slider() && spans_overlap(rect.loc.x, rect.size.w, origin.x, 1);
                        let (along, across, in_lane) = if dx == 0 {
                            (
                                step_y * dy.signum(),
                                if beneath { 0 } else { step_x },
                                spans_overlap(from.loc.x, from.size.w, rect.loc.x, rect.size.w),
                            )
                        } else {
                            (
                                step_x * dx.signum(),
                                step_y,
                                spans_overlap(from.loc.y, from.size.h, rect.loc.y, rect.size.h),
                            )
                        };
                        (along > 0 && in_lane).then_some((along + across.abs(), *slot))
                    })
                    .min_by_key(|(score, _)| *score)
                    .map(|(_, slot)| slot)
            }
        };
        let Some(next) = next else {
            return false;
        };
        if self.focused == Some(next) {
            return false;
        }
        self.set_focused(Some(next));
        true
    }

    fn invalidate_stale_buffers(&mut self) {
        let current = crate::paint::raster_revision(self.theme.revision());
        if current == self.raster_rev {
            return;
        }
        self.raster_rev = current;
        self.invalidate();
    }

    #[must_use]
    pub const fn caffeinated(&self) -> bool {
        self.caffeinated
    }

    #[must_use]
    pub const fn battery_low_pct(&self) -> u8 {
        self.theme.battery_low_pct
    }

    #[must_use]
    pub const fn battery_saver(&self) -> bool {
        self.theme.battery_saver
    }

    pub fn set_caffeinated(&mut self, on: bool) {
        if self.caffeinated == on {
            return;
        }
        self.caffeinated = on;
        self.caffeine_anim.retarget(if on { 1.0 } else { 0.0 }, motion::t2(), E_OVER);
        self.status_rev = self.status_rev.wrapping_add(1);
    }

    pub fn set_recording(&mut self, started: Option<Instant>) {
        if self.recording_started == started
            && (self.recording_anim.target() - if started.is_some() { 1.0 } else { 0.0 }).abs() < f64::EPSILON
        {
            return;
        }
        self.recording_started = started;
        if started.is_some() {
            self.recording_anim.retarget(1.0, motion::t2(), E_OUT);
        } else {
            self.recording_anim.retarget(0.0, motion::t2(), E_IO);
        }
        self.indicators.recording_reserved = self.recording_active();
        self.indicators.rev = self.indicators.rev.wrapping_add(1);
        self.touch();
    }

    #[must_use]
    pub fn is_recording(&self) -> bool {
        self.recording_started.is_some()
    }

    #[must_use]
    pub fn recording_elapsed(&self, now: Instant) -> Option<Duration> {
        self.recording_started.map(|started| now.saturating_duration_since(started))
    }

    pub(crate) fn recording_active(&self) -> bool {
        self.recording_started.is_some() || self.recording_anim.value() > 0.0 || self.recording_anim.is_animating()
    }

    #[must_use]
    pub const fn dnd(&self) -> bool {
        self.dnd
    }

    pub fn set_dnd(&mut self, on: bool) -> bool {
        if self.dnd == on {
            return false;
        }
        self.dnd = on;
        self.dnd_anim.retarget(if on { 1.0 } else { 0.0 }, motion::t2(), E_OVER);
        true
    }

    #[must_use]
    pub const fn unread(&self) -> usize {
        self.unread
    }

    pub fn set_unread(&mut self, unread: usize) -> bool {
        if self.unread == unread {
            return false;
        }
        self.unread = unread;
        if unread > 0 {
            self.badge = Badge { count: unread, reserved: true };
            self.badge_anim.retarget(1.0, motion::t2(), E_OVER);
        } else {
            self.badge_anim.retarget(0.0, motion::t1(), E_OUT);
            self.badge.reserved = self.badge_anim.value() > 0.0;
        }
        self.touch();
        true
    }

    #[must_use]
    pub const fn has_open_popup(&self) -> bool {
        self.popup_reveal.is_open()
    }

    #[must_use]
    pub const fn open_popup(&self) -> Option<OpenPopup> {
        self.open_popup
    }

    pub fn close_popup(&mut self) -> bool {
        let Some(popup) = self.open_popup.take() else {
            return false;
        };
        self.closing_popup = Some(popup);
        self.popup_reveal.close(motion::t2());
        self.set_focused(None);
        self.qs_panel.drop_prompt();
        self.sync_island();
        self.touch();
        true
    }

    pub fn toggle_popup(&mut self, popup: OpenPopup) -> bool {
        if self.open_popup == Some(popup) {
            return self.close_popup();
        }
        if popup == OpenPopup::MediaPlayer && self.media.is_none() {
            return false;
        }
        self.open_popup = Some(popup);
        self.closing_popup = None;
        self.popup_reveal.open(motion::t2());
        self.set_focused(None);
        self.qs_panel.reset();
        self.forget_power();
        if popup == OpenPopup::NotificationCenter {
            self.calendar.reset();
            self.reset_island_scroll();
            self.island_forget_alert();
        }
        self.sync_island();
        self.touch();
        true
    }

    fn forget_power(&mut self) {
        self.qs_panel.armed = None;
    }

    fn qs_shape(&self, painter: Option<&crate::paint::Painter>) -> QsShape {
        let status = &self.system_status;
        let chip = painter.map_or(QS_CHIP_DEFAULT, |painter| qs_chip_width(painter, status));
        QsShape {
            mic: status.mic_available,
            brightness: status.brightness_available,
            battery: status.battery_pct.is_some(),
            profile: status.power_profile.is_some(),
            vpn: self.qs_panel.netbird.installed || status.vpn_active || !self.qs_panel.vpns().is_empty(),
            bluetooth: status.bluetooth_available,
            network: status.wifi_available || status.ethernet_available,
            strip: qs_privacy(status).is_some(),
            chip,
        }
    }

    fn qs_main_key(&self, painter: &crate::paint::Painter) -> QsCacheKey {
        let main_view = self.qs_panel.view.is_none();
        let hovered = self.hovered_in(Slot::in_quick_settings).filter(|_| main_view);
        let dragging = self.pressed.filter(|slot| self.slider_dragging && slot.is_slider());
        let flags = QsFlags { caffeinated: self.caffeinated, dnd: self.dnd, dark: self.theme.dark, dragging };
        let motion = QsMotion {
            volume: percent(self.volume_anim.value()),
            mic: percent(self.mic_anim.value()),
            brightness: percent(self.brightness_anim.value()),
            tiles: self.tile_levels(),
        };
        let battery = self.system_status.battery_pct.map(|_| qs_chip_line(&self.system_status));
        let revs = (self.status_rev, self.qs_panel.rev, battery.unwrap_or_default());
        (revs, flags, motion, hovered, self.qs_shape(Some(painter)))
    }

    pub fn set_headset(&mut self, headset: crate::headset::Headset) {
        self.headsets.retain(|known| known.mac != headset.mac);
        self.headsets.push(headset);
        self.headsets_changed();
    }

    pub fn update_headset(&mut self, mac: &str, change: impl FnOnce(&mut crate::headset::Headset)) -> bool {
        let Some(headset) = self.headsets.iter_mut().find(|headset| headset.mac == mac) else {
            return false;
        };
        let before = headset.clone();
        change(headset);
        if *headset == before {
            return false;
        }
        self.headsets_changed();
        true
    }

    #[must_use]
    pub fn headset(&self) -> Option<&crate::headset::Headset> {
        self.headsets.last()
    }

    pub fn forget_headset(&mut self, mac: &str) -> bool {
        let before = self.headsets.len();
        self.headsets.retain(|headset| headset.mac != mac);
        let changed = self.headsets.len() != before;
        if changed {
            if self.headsets.is_empty() && self.open_popup == Some(OpenPopup::Headset) {
                self.close_popup();
            }
            self.headsets_changed();
        }
        changed
    }

    fn headsets_changed(&mut self) {
        self.status_rev = self.status_rev.wrapping_add(1);
        self.touch();
    }

    pub fn set_phone(&mut self, phone: Option<crate::phone::PhoneState>) -> bool {
        if self.phone == phone {
            return false;
        }
        let gone = phone.is_none();
        self.phone = phone;
        if gone && self.open_popup == Some(OpenPopup::Phone) {
            self.close_popup();
        }
        self.status_rev = self.status_rev.wrapping_add(1);
        self.touch();
        true
    }

    #[must_use]
    pub const fn phone(&self) -> Option<&crate::phone::PhoneState> {
        self.phone.as_ref()
    }

    #[must_use]
    pub const fn armed_power(&self) -> Option<PowerAction> {
        self.qs_panel.armed
    }

    pub fn arm_power(&mut self, action: PowerAction, apps: Vec<String>) -> bool {
        if self.open_popup != Some(OpenPopup::QuickSettings) {
            return false;
        }
        if self.qs_panel.armed == Some(action) {
            return self.disarm_power();
        }
        self.qs_panel.armed = Some(action);
        self.qs_panel.apps = apps;
        if self.focused.is_some() {
            self.set_focused(Some(Slot::QsPowerConfirm(action)));
        }
        self.qs_panel.bump();
        self.touch();
        true
    }

    pub fn disarm_power(&mut self) -> bool {
        let Some(action) = self.qs_panel.armed.take() else {
            return false;
        };
        if self.focused.is_some() {
            self.set_focused(Some(Slot::QsPowerButton(action)));
        }
        self.qs_panel.bump();
        self.touch();
        true
    }

    pub fn qs_escape(&mut self) -> bool {
        self.open_popup == Some(OpenPopup::QuickSettings)
            && (self.qs_panel.collapse() || self.disarm_power() || self.close_detail())
    }

    #[must_use]
    pub fn drawer_device(&self, drawer: Drawer, index: u16) -> Option<crate::qs_net::AudioOutput> {
        self.qs_panel.devices(drawer).get(usize::from(index)).cloned()
    }

    pub fn set_system_status(&mut self, status: crate::system_info::SystemStatus) -> bool {
        if self.system_status == status {
            return false;
        }
        let at_a_glance = !self.system_status.looks_like(&status);
        let (volume, mic, brightness) = (status.volume_pct, status.mic_pct, status.brightness_pct);
        match (self.system_status.power_profile, status.power_profile) {
            (Some(before), Some(after)) if before != after => {
                self.profile_anim.retarget(profile_index(after), motion::t2(), E_OUT);
            }
            (None, Some(after)) => self.profile_anim.jump(profile_index(after)),
            _ => {}
        }
        self.system_status = status;
        self.detail_rev = self.detail_rev.wrapping_add(1);
        if !at_a_glance {
            return self.open_popup.is_some();
        }
        self.slide(|bar| &mut bar.volume_anim, volume);
        self.slide(|bar| &mut bar.mic_anim, mic);
        self.slide(|bar| &mut bar.brightness_anim, brightness);
        self.status_rev = self.status_rev.wrapping_add(1);
        self.sync_indicators();
        self.touch();
        true
    }

    #[must_use]
    pub fn system_status(&self) -> &crate::system_info::SystemStatus {
        &self.system_status
    }

    #[must_use]
    pub const fn media(&self) -> Option<&MediaWidget> {
        self.media.as_ref()
    }

    pub fn set_nc_items(&mut self, items: Vec<crate::notifications::NcItem>) -> bool {
        if self.nc_items == items {
            return false;
        }
        self.nc_items = items;
        self.nc_rev = self.nc_rev.wrapping_add(1);
        self.touch();
        true
    }

    pub fn set_media(&mut self, media: Option<MediaWidget>) -> bool {
        if self.media == media {
            return false;
        }
        let progress = media.as_ref().and_then(MediaWidget::progress_permille).map_or(0.0, f64::from);
        let gone = media.is_none();
        self.media = media;
        self.seek_anim.jump(progress);
        self.media_rev = self.media_rev.wrapping_add(1);
        self.track_rev = self.track_rev.wrapping_add(1);
        if gone && self.open_popup == Some(OpenPopup::MediaPlayer) {
            self.close_popup();
        }
        self.sync_island();
        self.touch();
        true
    }

    fn seek_permille(&self) -> Option<u16> {
        self.media.as_ref()?.progress?;
        Some(self.seek_anim.value().clamp(0.0, 1000.0).round() as u16)
    }

    #[must_use]
    pub const fn popup_rect(&self) -> Option<Rectangle<i32, Logical>> {
        self.popup_rect
    }

    pub fn set_media_progress(&mut self, progress: Option<f32>) -> bool {
        let Some(media) = self.media.as_mut() else {
            return false;
        };
        if media.progress_permille() == progress_permille(progress) {
            return false;
        }
        media.progress = progress;
        let target = progress_permille(progress).map_or(0.0, f64::from);
        if self.slider_dragging {
            self.seek_anim.jump(target);
        } else {
            self.seek_anim.retarget(target, motion::t2(), E_OUT);
        }
        self.media_rev = self.media_rev.wrapping_add(1);
        true
    }

    #[must_use]
    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    pub fn scroll_steps(&mut self, delta: crate::system_info::ScrollDelta) -> i32 {
        self.scroll.feed(delta)
    }

    pub fn set_clock(&mut self, date: String, clock: String) -> bool {
        if self.date == date && self.clock == clock {
            return false;
        }
        self.date = date;
        self.clock = clock;
        self.touch();
        true
    }

    pub fn set_layouts(&mut self, layouts: Vec<String>, active: usize) -> bool {
        if self.layouts == layouts && self.layout_index == active {
            return false;
        }
        let pill = f64::from(u8::from(active >= 1));
        if self.layouts == layouts {
            self.segment_anim.retarget(pill, motion::t2(), E_OUT);
        } else {
            self.layouts_rev = self.layouts_rev.wrapping_add(1);
            self.segment_anim.jump(pill);
        }
        self.layouts = layouts;
        self.layout_index = active;
        self.touch();
        true
    }

    pub fn set_app(&mut self, app_id: String, name: String) -> bool {
        if self.app_id == app_id && self.app_name == name {
            return false;
        }
        self.app_id = app_id;
        self.app_name = name;
        self.touch();
        true
    }

    #[must_use]
    #[allow(clippy::unused_self)]
    pub fn rect(&self, output: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
        Rectangle::new(output.loc, Size::from((output.size.w, HEIGHT)))
    }

    #[must_use]
    pub fn contains(&self, output: Rectangle<i32, Logical>, pos: Point<f64, Logical>) -> bool {
        if self.covered() {
            return self.floats() && self.island_target(output).to_f64().contains(pos);
        }
        if self.rect(output).to_f64().contains(pos) || self.island_target(output).to_f64().contains(pos) {
            return true;
        }
        if self.popup_reveal.is_open()
            && let Some(popup) = self.popup_rect
            && popup.to_f64().contains(pos)
        {
            return true;
        }
        if self.popup_reveal.is_open()
            && self.open_popup == Some(OpenPopup::Passwords)
            && self.passwords_menu_rect.is_some_and(|menu| menu.to_f64().contains(pos))
        {
            return true;
        }
        self.popup_reveal.is_open()
            && self.open_popup == Some(OpenPopup::AppMenu)
            && self.menu_panels.iter().any(|panel| panel.rect.to_f64().contains(pos))
    }

    #[must_use]
    pub fn action_at(&self, pos: Point<f64, Logical>) -> Option<TopbarAction> {
        self.slots.iter().find(|(_, rect)| rect.to_f64().contains(pos)).map(|(slot, rect)| match slot {
            Slot::QsVolumeTrack => {
                let ratio = ((pos.x - f64::from(rect.loc.x)) / f64::from(rect.size.w)).clamp(0.0, 1.0);
                let pct = (ratio * 100.0).round() as u8;
                TopbarAction::QsVolumeAdjust(pct)
            }
            Slot::QsMicTrack => {
                let ratio = ((pos.x - f64::from(rect.loc.x)) / f64::from(rect.size.w)).clamp(0.0, 1.0);
                let pct = (ratio * 100.0).round() as u8;
                TopbarAction::QsMicAdjust(pct)
            }
            Slot::QsBrightnessTrack => {
                let ratio = ((pos.x - f64::from(rect.loc.x)) / f64::from(rect.size.w)).clamp(0.0, 1.0);
                let pct = (ratio * 100.0).round() as u8;
                TopbarAction::QsBrightnessAdjust(pct)
            }
            Slot::PopMediaSeek => {
                let ratio = ((pos.x - f64::from(rect.loc.x)) / f64::from(rect.size.w)).clamp(0.0, 1.0);
                let pct = (ratio * 100.0).round() as u8;
                TopbarAction::MediaSeek(pct)
            }
            other => self.resolve(*other),
        })
    }

    #[must_use]
    pub fn slot_at(&self, pos: Point<f64, Logical>) -> Option<Slot> {
        self.slots.iter().find(|(_, rect)| rect.to_f64().contains(pos)).map(|(slot, _)| *slot)
    }

    pub fn set_hover(&mut self, pos: Option<Point<f64, Logical>>) -> bool {
        let hovered =
            pos.and_then(|pos| self.slots.iter().find(|(_, rect)| rect.to_f64().contains(pos)).map(|(slot, _)| *slot));
        if self.hovered == hovered {
            return false;
        }
        if let Some(previous) = self.hovered.take()
            && let Some(layer) = self.hover_layers.get_mut(&previous)
        {
            layer.retarget(0.0, motion::t1(), E_IO);
        }
        if let Some(slot) = hovered.filter(|slot| slot.state_layer_radius().is_some()) {
            self.hover_layers.entry(slot).or_default().retarget(1.0, motion::t1(), E_IO);
        }
        self.hovered = hovered;
        self.tray_hover(hovered);
        self.follow_menu_hover(hovered);
        true
    }

    fn segmented(&self) -> bool {
        self.layouts.len() == 2
    }

    fn layout_control_width(&self, painter: &crate::paint::Painter) -> i32 {
        if self.segmented() {
            let track: i32 = self.layouts.iter().map(|name| segment_width(painter, name)).sum();
            SEG_PAD_X * 2 + SEG_INSET * 2 + track + SEG_GAP
        } else if self.layouts.len() >= 3 {
            let label = self.active_layout_name();
            lang_button_width(painter, label)
        } else {
            let label = self.active_layout_name();
            button_width(painter, label)
        }
    }

    fn active_layout_name(&self) -> &str {
        self.layouts.get(self.layout_index).map_or("", String::as_str)
    }

    const fn caffeine_width() -> i32 {
        COFFEE_PAD + COFFEE_GLYPH + COFFEE_GAP + SWITCH_WIDTH + COFFEE_PAD_END
    }

    fn layout_slots(&mut self, painter: &crate::paint::Painter, output: Rectangle<i32, Logical>) {
        self.arrange(painter, output, false);
    }

    fn layout_passive(&mut self, painter: &crate::paint::Painter, output: Rectangle<i32, Logical>) {
        self.arrange(painter, output, true);
    }

    fn arrange(&mut self, painter: &crate::paint::Painter, output: Rectangle<i32, Logical>, passive: bool) {
        let key = (self.rev, output);
        if self.layout_key == Some(key) {
            return;
        }
        self.layout_key = Some(key);
        let strip = self.rect(output);
        let y = strip.loc.y + (HEIGHT - BUTTON_HEIGHT) / 2;
        let mut right = strip.loc.x + strip.size.w - PAD_X;

        let mut slots = Vec::new();

        right = self.layout_status_cluster(painter, right, y, &mut slots);

        right -= STATUS_SEP_WIDTH;

        let coffee_width = Self::caffeine_width();
        right -= coffee_width;
        slots
            .push((Slot::Caffeine, Rectangle::new(Point::from((right, y)), Size::from((coffee_width, BUTTON_HEIGHT)))));

        let mut lang_rect = None;
        if !self.layouts.is_empty() {
            let width = self.layout_control_width(painter);
            right -= ITEM_GAP + width;
            let control_rect = Rectangle::new(Point::from((right, y)), Size::from((width, BUTTON_HEIGHT)));
            lang_rect = Some(control_rect);
            if self.segmented() {
                let mut x = right + SEG_PAD_X + SEG_INSET;
                let top = y + (BUTTON_HEIGHT - SEG_HEIGHT) / 2 + SEG_INSET;
                let height = SEG_HEIGHT - SEG_INSET * 2;
                for (index, name) in self.layouts.iter().enumerate() {
                    let seg = segment_width(painter, name);
                    slots.push((
                        Slot::LayoutPick(index),
                        Rectangle::new(Point::from((x, top)), Size::from((seg, height))),
                    ));
                    x += seg + SEG_GAP;
                }
            } else if self.layouts.len() >= 3 {
                slots.push((Slot::LayoutDropdown, control_rect));
            } else {
                slots.push((Slot::LayoutCycle, control_rect));
            }
        }

        let mut island_slots = Vec::new();
        let island = self.layout_island(painter, strip, &mut island_slots, passive);
        let tray_limit = island.loc.x + island.size.w + glide_theme::spacing::SP3;
        self.layout_tray(output, (right - ITEM_GAP, tray_limit, y), &mut slots);

        let menu_limit = island.loc.x - glide_theme::spacing::SP3;
        let mut menu = self.layout_app_menu(painter, output, strip, menu_limit);
        if passive {
            slots.append(&mut menu.titles);
            slots.extend(island_slots);
            self.slots = slots;
            return;
        }
        if self.covered() {
            slots.clear();
            menu.titles.clear();
        }
        slots.append(&mut menu.titles);
        if !(self.popup_reveal.is_open() && Self::island_owns(self.open_popup)) {
            slots.extend(island_slots);
        }

        let mut popup_slots = std::mem::take(&mut menu.rows);
        let mut popup_rect = self
            .layout_popup(painter, (output, strip), (lang_rect, menu.anchor), &mut popup_slots)
            .or_else(|| self.layout_status_popup(&slots, output, strip.loc.y))
            .or_else(|| self.layout_headset_popup(&slots, output, strip.loc.y, &mut popup_slots))
            .or_else(|| self.layout_phone_popup(&slots, output, strip.loc.y, &mut popup_slots));
        if popup_rect.is_none() {
            popup_rect = self.layout_passwords_popup(&slots, output, strip.loc.y, &mut popup_slots);
        } else {
            self.passwords_menu_rect = None;
        }
        if self.popup_reveal.is_open() {
            slots.extend(popup_slots.iter().copied());
        }

        self.slots = slots;
        self.shown_popup_slots = popup_slots;
        self.popup_rect = popup_rect;
        self.menu_panels = menu.panels;
    }

    fn layout_popup(
        &self, painter: &crate::paint::Painter, (output, strip): (Rectangle<i32, Logical>, Rectangle<i32, Logical>),
        anchors: (Anchor, Anchor), popup_slots: &mut Vec<(Slot, Rectangle<i32, Logical>)>,
    ) -> Option<Rectangle<i32, Logical>> {
        let (lang_rect, menu_anchor) = anchors;
        let strip_y = strip.loc.y;
        match self.shown_popup() {
            Some(OpenPopup::AppMenu) => menu_anchor,
            Some(OpenPopup::LanguageMenu) => {
                lang_rect.map(|btn| layout_lang_menu_slots(&self.layouts, btn, strip_y, popup_slots))
            }
            Some(popup @ (OpenPopup::NotificationCenter | OpenPopup::MediaPlayer)) => {
                self.layout_island_popup(painter, popup, strip, popup_slots)
            }
            Some(OpenPopup::QuickSettings) => Some(layout_qs_slots(
                output,
                strip_y,
                QsLayoutInput {
                    shape: self.qs_shape(Some(painter)),
                    volume: self.system_status.volume_available,
                    detail: self.detail_slots(painter),
                },
                popup_slots,
            )),
            Some(OpenPopup::Privacy | OpenPopup::Headset | OpenPopup::Phone | OpenPopup::Passwords) | None => None,
        }
    }

    fn desktop_menu_logo(&self, index: usize) -> bool {
        self.app_name.is_empty() && index == 0 && self.menu_bar.first().is_some_and(|item| item.label == "Glide")
    }

    fn layout_app_menu(
        &self, painter: &crate::paint::Painter, output: Rectangle<i32, Logical>, strip: Rectangle<i32, Logical>,
        limit: i32,
    ) -> MenuLayout {
        let y = strip.loc.y + (HEIGHT - BUTTON_HEIGHT) / 2;
        let left = if self.app_name.is_empty() {
            strip.loc.x + PAD_X
        } else {
            let chip = if self.app_chip_w > 0 {
                self.app_chip_w
            } else {
                app_chip_width(painter, &self.app_name, !self.app_id.is_empty())
            };
            strip.loc.x + PAD_X + chip + ITEM_GAP * 2
        };
        let title_rects = layout_menu_titles(painter, &self.menu_bar, left, limit, y);
        let titles = title_rects
            .iter()
            .filter_map(|(index, rect)| Some((Slot::MenuTitle(u16::try_from(*index).ok()?), *rect)))
            .collect();
        let anchor = self.menu_anchor(&title_rects);
        let panels = if self.shown_popup() == Some(OpenPopup::AppMenu) {
            anchor
                .map(|title| {
                    crate::global_menu::layout_panels(
                        painter,
                        self.menu_items(),
                        self.menu_open,
                        title,
                        strip.loc.y + HEIGHT + glide_theme::spacing::SP1,
                        output,
                    )
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let rows = panels
            .iter()
            .rev()
            .flat_map(|panel| panel.rows.iter())
            .filter(|row| row.enabled)
            .map(|row| (Slot::MenuRow(row.path), row.rect))
            .collect();
        MenuLayout { titles, rows, anchor: panels.first().map(|panel| panel.rect), panels }
    }

    fn badge_shown(&self) -> usize {
        if self.unread > 0 {
            self.unread
        } else if self.badge.reserved {
            self.badge.count
        } else {
            0
        }
    }

    #[must_use]
    pub fn slot_rect(&self, slot: Slot) -> Option<Rectangle<i32, Logical>> {
        self.slots.iter().find(|(candidate, _)| *candidate == slot).map(|(_, rect)| *rect)
    }

    fn layout_control_rect(&self) -> Option<Rectangle<i32, Logical>> {
        if self.segmented() {
            let first = self.slot_rect(Slot::LayoutPick(0))?;
            let last = self.slot_rect(Slot::LayoutPick(self.layouts.len() - 1))?;
            let x = first.loc.x - SEG_PAD_X - SEG_INSET;
            let width = (last.loc.x + last.size.w + SEG_INSET + SEG_PAD_X) - x;
            Some(Rectangle::new(
                Point::from((x, first.loc.y - SEG_INSET - (BUTTON_HEIGHT - SEG_HEIGHT) / 2)),
                Size::from((width, BUTTON_HEIGHT)),
            ))
        } else if self.layouts.len() >= 3 {
            self.slot_rect(Slot::LayoutDropdown)
        } else {
            self.slot_rect(Slot::LayoutCycle)
        }
    }
}

fn layout_menu_titles(
    painter: &crate::paint::Painter, bar: &[glide_appmenu::MenuItem], left: i32, limit: i32, y: i32,
) -> Vec<(usize, Rectangle<i32, Logical>)> {
    let mut x = left;
    let mut out = Vec::new();
    for (index, item) in bar.iter().enumerate() {
        if !item.is_submenu() {
            continue;
        }
        let width = crate::global_menu::title_width(painter, &item.label);
        if x + width > limit {
            break;
        }
        out.push((index, Rectangle::new(Point::from((x, y)), Size::from((width, BUTTON_HEIGHT)))));
        x += width;
    }
    out
}

fn layout_lang_menu_slots(
    layouts: &[String], btn: Rectangle<i32, Logical>, strip_y: i32, slots: &mut Vec<(Slot, Rectangle<i32, Logical>)>,
) -> Rectangle<i32, Logical> {
    let menu_w = LANG_MENU_WIDTH;
    let menu_x = btn.loc.x + btn.size.w - menu_w;
    let menu_y = strip_y + HEIGHT + POPUP_GAP;
    let count = i32::try_from(layouts.len()).unwrap_or(0);
    let menu_h = LANG_MENU_PAD * 2 + count * LANG_MENU_ITEM_HEIGHT + LANG_MENU_DIVIDER + LANG_MENU_ITEM_HEIGHT;
    let rect = Rectangle::new(Point::from((menu_x, menu_y)), Size::from((menu_w, menu_h)));

    let mut item_y = menu_y + LANG_MENU_PAD;
    for index in 0..layouts.len() {
        slots.push((
            Slot::LangMenuItem(index),
            Rectangle::new(
                Point::from((menu_x + LANG_MENU_PAD, item_y)),
                Size::from((menu_w - LANG_MENU_PAD * 2, LANG_MENU_ITEM_HEIGHT)),
            ),
        ));
        item_y += LANG_MENU_ITEM_HEIGHT;
    }
    item_y += LANG_MENU_DIVIDER;
    slots.push((
        Slot::LangMenuSettings,
        Rectangle::new(
            Point::from((menu_x + LANG_MENU_PAD, item_y)),
            Size::from((menu_w - LANG_MENU_PAD * 2, LANG_MENU_ITEM_HEIGHT)),
        ),
    ));
    rect
}

type LocalRect = Rectangle<i32, Logical>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum QsTile {
    Network,
    Bluetooth,
    Vpn,
    Saver,
    Dnd,
    NightLight,
    DarkTheme,
    Caffeine,
}

const QS_TILE_COUNT: usize = 8;

const QS_TILES: [QsTile; QS_TILE_COUNT] = [
    QsTile::Network,
    QsTile::Bluetooth,
    QsTile::Vpn,
    QsTile::Saver,
    QsTile::Dnd,
    QsTile::NightLight,
    QsTile::DarkTheme,
    QsTile::Caffeine,
];

fn qs_tiles(shape: QsShape) -> impl Iterator<Item = QsTile> {
    QS_TILES.into_iter().filter(move |tile| match tile {
        QsTile::Network => shape.network,
        QsTile::Bluetooth => shape.bluetooth,
        QsTile::Vpn => shape.vpn,
        QsTile::Saver => shape.profile,
        QsTile::Dnd | QsTile::NightLight | QsTile::DarkTheme | QsTile::Caffeine => true,
    })
}

impl QsTile {
    const fn index(self) -> usize {
        match self {
            Self::Network => 0,
            Self::Bluetooth => 1,
            Self::Vpn => 2,
            Self::Saver => 3,
            Self::Dnd => 4,
            Self::NightLight => 5,
            Self::DarkTheme => 6,
            Self::Caffeine => 7,
        }
    }

    const fn slot(self) -> Slot {
        match self {
            Self::Network => Slot::QsWifi,
            Self::Bluetooth => Slot::QsBluetooth,
            Self::Vpn => Slot::QsNetbird,
            Self::Saver => Slot::QsSaver,
            Self::Dnd => Slot::QsDnd,
            Self::NightLight => Slot::QsNightLight,
            Self::DarkTheme => Slot::QsDarkTheme,
            Self::Caffeine => Slot::QsCaffeine,
        }
    }

    const fn more(self) -> Option<Slot> {
        match self {
            Self::Network => Some(Slot::QsWifiMore),
            Self::Bluetooth => Some(Slot::QsBluetoothMore),
            Self::Vpn => Some(Slot::QsNetbirdMore),
            Self::Saver => Some(Slot::QsSaverMore),
            Self::Dnd | Self::NightLight | Self::DarkTheme | Self::Caffeine => None,
        }
    }
}

const QS_PROFILES: [PowerProfile; 3] = [PowerProfile::PowerSaver, PowerProfile::Balanced, PowerProfile::Performance];

fn profile_index(profile: PowerProfile) -> f64 {
    let index = QS_PROFILES.iter().position(|candidate| *candidate == profile).unwrap_or(1);
    f64::from(u8::try_from(index).unwrap_or(1))
}

#[allow(clippy::struct_excessive_bools)] // Independent facts about which optional rows and tiles the panel has
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
struct QsShape {
    mic: bool,
    brightness: bool,
    battery: bool,
    profile: bool,
    vpn: bool,
    bluetooth: bool,
    network: bool,
    strip: bool,
    chip: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QsSliderRow {
    icon: LocalRect,
    track: LocalRect,
    readout: LocalRect,
    drill: Option<LocalRect>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QsFooter {
    chip: Option<LocalRect>,
    screenshot: LocalRect,
    settings: LocalRect,
    lock: LocalRect,
    power: LocalRect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct QsGeometry {
    strip: Option<LocalRect>,
    volume: QsSliderRow,
    mic: Option<QsSliderRow>,
    brightness: Option<QsSliderRow>,
    dividers: [i32; 2],
    tiles: Vec<(QsTile, LocalRect)>,
    footer: QsFooter,
    height: i32,
}

use crate::raster::rect as local_rect;

const fn band(rect: LocalRect) -> (i32, i32, i32, i32) {
    (rect.loc.x, rect.loc.x + rect.size.w, rect.loc.y, rect.loc.y + rect.size.h)
}

fn qs_split(rect: LocalRect) -> (LocalRect, LocalRect) {
    let main = local_rect(rect.loc.x, rect.loc.y, rect.size.w - QS_MORE_W, rect.size.h);
    let more = local_rect(rect.loc.x + rect.size.w - QS_MORE_W, rect.loc.y, QS_MORE_W, rect.size.h);
    (main, more)
}

fn qs_slider_row(x: i32, y: i32, width: i32, drill: bool) -> QsSliderRow {
    let right = x + width;
    let readout = local_rect(right - QS_DRILL - QS_ROW_GAP - QS_READOUT_W, y, QS_READOUT_W, QS_ROW);
    let track_left = x + QS_ICON_BTN + QS_ROW_GAP + QS_SLIDER_MARGIN;
    let track_right = readout.loc.x - QS_ROW_GAP - QS_SLIDER_MARGIN;
    QsSliderRow {
        icon: local_rect(x, y, QS_ICON_BTN, QS_ROW),
        track: local_rect(track_left, y, track_right - track_left, QS_ROW),
        readout,
        drill: drill.then(|| local_rect(right - QS_DRILL, y + (QS_ROW - QS_DRILL) / 2, QS_DRILL, QS_DRILL)),
    }
}

fn qs_geometry(shape: &QsShape) -> QsGeometry {
    let row_w = QS_WIDTH - QS_PAD * 2;
    let x = QS_PAD;
    let mut y = QS_PAD;
    let strip = if shape.strip {
        let strip = local_rect(x, y, row_w, QS_ROW);
        y += QS_ROW + QS_STRIP_GAP;
        Some(strip)
    } else {
        None
    };
    let volume = qs_slider_row(x, y, row_w, true);
    y += QS_ROW;
    let mut next_row = |drill: bool| {
        y += QS_ROW_GAP;
        let row = qs_slider_row(x, y, row_w, drill);
        y += QS_ROW;
        row
    };
    let mic = shape.mic.then(|| next_row(true));
    let brightness = shape.brightness.then(|| next_row(false));
    let first = y + SP2;
    y += QS_DIVIDER;

    let col_w = (row_w - QS_GRID_GAP) / 2;
    let grid_y = y;
    let tiles: Vec<(QsTile, LocalRect)> = qs_tiles(*shape)
        .zip(0..)
        .map(|(tile, index)| {
            let (column, row) = (index % 2, index / 2);
            let left = x + column * (col_w + QS_GRID_GAP);
            (tile, local_rect(left, grid_y + row * (QS_TILE_HEIGHT + QS_GRID_GAP), col_w, QS_TILE_HEIGHT))
        })
        .collect();
    let rows = i32::try_from(tiles.len().div_ceil(2)).unwrap_or(1);
    y += rows * QS_TILE_HEIGHT + (rows - 1) * QS_GRID_GAP;
    let second = y + SP2;
    y += QS_DIVIDER;

    let button =
        |slot: i32| local_rect(x + row_w - slot * (QS_ICON_BTN + QS_ROW_GAP) + QS_ROW_GAP, y, QS_ICON_BTN, QS_ROW);
    let footer = QsFooter {
        chip: shape.battery.then(|| local_rect(x, y, shape.chip, QS_ROW)),
        screenshot: button(4),
        settings: button(3),
        lock: button(2),
        power: button(1),
    };
    QsGeometry { strip, volume, mic, brightness, dividers: [first, second], tiles, footer, height: y + QS_ROW + QS_PAD }
}

fn offset_rect(rect: LocalRect, origin: Point<i32, Logical>) -> LocalRect {
    Rectangle::new(rect.loc + origin, rect.size)
}

fn rect_centre(rect: LocalRect) -> Point<i32, Logical> {
    Point::from((rect.loc.x + rect.size.w / 2, rect.loc.y + rect.size.h / 2))
}

const fn spans_overlap(a_start: i32, a_len: i32, b_start: i32, b_len: i32) -> bool {
    a_start < b_start + b_len && b_start < a_start + a_len
}

fn popup_slots(popup: OpenPopup) -> fn(Slot) -> bool {
    match popup {
        OpenPopup::QuickSettings => Slot::in_quick_settings,
        OpenPopup::NotificationCenter => Slot::in_notification_center,
        OpenPopup::LanguageMenu => Slot::in_lang_menu,
        OpenPopup::MediaPlayer => Slot::in_media_popup,
        OpenPopup::AppMenu => Slot::in_app_menu,
        OpenPopup::Privacy => |_| false,
        OpenPopup::Headset => Slot::in_headset_popup,
        OpenPopup::Phone => Slot::in_phone_popup,
        OpenPopup::Passwords => Slot::in_passwords_popup,
    }
}

const FOCUS_RING: i32 = 2;
const FOCUS_RING_STROKE: f32 = 2.0;

fn focus_ring_bounds(rect: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
    Rectangle::new(
        Point::from((rect.loc.x - FOCUS_RING, rect.loc.y - FOCUS_RING)),
        Size::from((rect.size.w + FOCUS_RING * 2, rect.size.h + FOCUS_RING * 2)),
    )
}

fn draw_layer(
    layer: Layer, size: Size<i32, Logical>, radius: f32, theme: &TopbarTheme, scale: i32,
) -> Option<MemoryRenderBuffer> {
    let mut c = Canvas::new(size.w, size.h, scale)?;
    match layer {
        Layer::Focus => c.stroke(c.full(), FOCUS_RING_STROKE, radius, layer.colour(theme)),
        Layer::Hover | Layer::Press => c.fill(c.full(), layer.colour(theme), radius),
    }
    Some(c.finish())
}

#[derive(Debug, Clone, Copy)]
struct LayerJob {
    layer: Layer,
    rect: Rectangle<i32, Logical>,
    radius: f32,
    alpha: f32,
    in_popup: bool,
}

impl Topbar {
    fn layer_buffer(
        &mut self, layer: Layer, size: Size<i32, Logical>, radius: f32, scale: i32,
    ) -> Option<&MemoryRenderBuffer> {
        let key = (layer.tag(), size.w, size.h, radius.to_bits(), scale);
        if let std::collections::hash_map::Entry::Vacant(entry) = self.layers.entry(key) {
            entry.insert(draw_layer(layer, size, radius, &self.theme, scale)?);
        }
        self.layers.get(&key)
    }

    fn drawn_slot_rect(&self, slot: Slot) -> Option<Rectangle<i32, Logical>> {
        self.slot_rect(slot)
            .or_else(|| self.shown_popup_slots.iter().find(|(candidate, _)| *candidate == slot).map(|(_, rect)| *rect))
    }

    fn press_level(&self, slot: Slot) -> f64 {
        self.press_layers.get(&slot).map_or(0.0, |anim| anim.value().clamp(0.0, 1.0))
    }

    fn layer_jobs(&self) -> Vec<LayerJob> {
        let mut jobs = Vec::new();
        let pairs = [(Layer::Hover, &self.hover_layers), (Layer::Press, &self.press_layers)];
        for (layer, anims) in pairs {
            for (slot, anim) in anims {
                let mut level = anim.value().clamp(0.0, 1.0);
                if layer == Layer::Hover {
                    level *= 1.0 - self.press_level(*slot);
                }
                let alpha = level as f32;
                if alpha <= 0.0 {
                    continue;
                }
                let (Some(radius), Some(rect)) = (slot.state_layer_radius(), self.drawn_slot_rect(*slot)) else {
                    continue;
                };
                jobs.push(LayerJob { layer, rect, radius, alpha, in_popup: !slot.in_strip() });
            }
        }
        let focus = self.focus_anim.value().clamp(0.0, 1.0) as f32;
        if focus > 0.0
            && let Some(rect) = self.focus_ring_rect().or(self.focus_ring_last)
        {
            jobs.push(LayerJob {
                layer: Layer::Focus,
                rect: focus_ring_bounds(rect),
                radius: glide_theme::shape::R2 + FOCUS_RING_STROKE,
                alpha: focus,
                in_popup: true,
            });
        }
        jobs
    }
}

fn push_state_layers<R: ShellRenderer>(topbar: &mut Topbar, frame: &mut Frame<'_, R>) {
    let (renderer, _, _, scale, output_scale, elements) = frame.parts();
    let reveal = topbar.popup_progress();
    let popup = topbar.popup_rect;
    let island = Topbar::island_owns(topbar.shown_popup()).then(|| topbar.island_content_alpha());
    for job in topbar.layer_jobs() {
        let placed = match (job.in_popup, popup, island) {
            (true, Some(_), Some(fade)) => {
                Placed { popup: job.rect, element: job.rect, progress: 1.0, alpha: job.alpha * fade }
            }
            (true, Some(popup), None) => Placed { popup, element: job.rect, progress: reveal, alpha: job.alpha },
            (true, None, _) => continue,
            (false, _, _) => Placed { popup: job.rect, element: job.rect, progress: 1.0, alpha: job.alpha },
        };
        let Some(buffer) = topbar.layer_buffer(job.layer, job.rect.size, job.radius, output_scale) else {
            continue;
        };
        push_placed(renderer, buffer, placed, scale, elements);
    }
}

struct QsLayoutInput {
    shape: QsShape,
    volume: bool,
    detail: Option<(i32, Vec<(Slot, LocalRect)>)>,
}

fn layout_qs_slots(
    output: Rectangle<i32, Logical>, strip_y: i32, input: QsLayoutInput,
    slots: &mut Vec<(Slot, Rectangle<i32, Logical>)>,
) -> Rectangle<i32, Logical> {
    let geometry = qs_geometry(&input.shape);
    let origin = Point::from((output.loc.x + output.size.w - PAD_X - QS_WIDTH, strip_y + HEIGHT + POPUP_GAP));
    let height = input.detail.as_ref().map_or(geometry.height, |(height, _)| *height);
    let popup = Rectangle::new(origin, Size::from((QS_WIDTH, height)));
    let mut push = |slot: Slot, rect: LocalRect| slots.push((slot, offset_rect(rect, origin)));
    if let Some((_, detail)) = input.detail {
        for (slot, rect) in detail {
            push(slot, rect);
        }
        return popup;
    }

    if input.volume {
        push(Slot::QsVolumeMute, geometry.volume.icon);
        push(Slot::QsVolumeTrack, geometry.volume.track);
    }
    if let Some(drill) = geometry.volume.drill {
        push(Slot::QsVolumeMore, drill);
    }
    if let Some(mic) = geometry.mic {
        push(Slot::QsMicMute, mic.icon);
        push(Slot::QsMicTrack, mic.track);
        if let Some(drill) = mic.drill {
            push(Slot::QsMicMore, drill);
        }
    }
    if let Some(brightness) = geometry.brightness {
        push(Slot::QsBrightnessTrack, brightness.track);
    }
    for &(tile, rect) in &geometry.tiles {
        if let Some(more) = tile.more() {
            let (main, side) = qs_split(rect);
            push(tile.slot(), main);
            push(more, side);
        } else {
            push(tile.slot(), rect);
        }
    }
    let footer = geometry.footer;
    if let Some(chip) = footer.chip {
        push(Slot::QsBattery, chip);
    }
    push(Slot::QsScreenshot, footer.screenshot);
    push(Slot::QsSettings, footer.settings);
    push(Slot::QsLock, footer.lock);
    push(Slot::QsPowerMenu, footer.power);
    popup
}

#[must_use]
pub fn native_layout_name(code: &str) -> &'static str {
    match code.to_ascii_uppercase().as_str() {
        "UA" | "UKR" => "Українська",
        "US" | "ENG" | "EN" => "English",
        "PL" | "POL" => "Polski",
        "DE" | "DEU" | "GER" => "Deutsch",
        "FR" | "FRA" => "Français",
        "ES" | "ESP" => "Español",
        "IT" | "ITA" => "Italiano",
        "PT" | "POR" => "Português",
        "CZ" | "CZE" => "Čeština",
        "SK" | "SLK" => "Slovenčina",
        "NL" | "NLD" => "Nederlands",
        "SE" | "SWE" => "Svenska",
        "NO" | "NOR" => "Norsk",
        "FI" | "FIN" => "Suomi",
        "DK" | "DAN" => "Dansk",
        "TR" | "TUR" => "Türkçe",
        "RO" | "RON" => "Română",
        "HU" | "HUN" => "Magyar",
        "EL" | "GRE" => "Ελληνικά",
        "JA" | "JPN" => "日本語",
        "ZH" | "CHI" => "中文",
        "KO" | "KOR" => "한국어",
        _ => "",
    }
}

fn lang_button_width(painter: &crate::paint::Painter, label: &str) -> i32 {
    (text_w(painter, label, &TextStyle::TOPBAR) + 29).max(BUTTON_MIN_WIDTH)
}

#[must_use]
pub fn short_layout_name(name: &str) -> String {
    let inner = name.rsplit_once('(').and_then(|(_, tail)| tail.split_once(')')).map(|(inside, _)| inside.trim());

    let candidate = inner.filter(|inside| !inside.is_empty()).unwrap_or(name);
    let word = candidate.split_whitespace().next().unwrap_or(candidate);
    let short: String = word.chars().take(3).collect();
    short.to_uppercase()
}

#[must_use]
pub fn app_display_name(app_id: &str, title: &str) -> String {
    application_name(app_id, Some(title))
}

#[must_use]
pub fn application_name(app_id: &str, resolved_name: Option<&str>) -> String {
    if let Some(name) = resolved_name.map(str::trim).filter(|name| !name.is_empty()) {
        return truncate_name(name);
    }
    let trimmed = app_id.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let tail = trimmed.rsplit('.').next().unwrap_or(trimmed);
    let tail = tail.trim();
    if tail.is_empty() {
        return String::new();
    }
    let mut chars = tail.chars();
    let name = match chars.next() {
        Some(first) if first.is_lowercase() => first.to_uppercase().collect::<String>() + chars.as_str(),
        _ => tail.to_string(),
    };
    truncate_name(&name)
}

fn truncate_name(name: &str) -> String {
    if name.chars().count() <= APP_NAME_MAX {
        return name.to_string();
    }
    let kept: String = name.chars().take(APP_NAME_MAX - 1).collect();
    format!("{}…", kept.trim_end())
}

fn ensure_time_locale() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // SAFETY: `setlocale` is called once, before any thread reads the
        // locale, and the empty string asks libc to honour the environment.
        unsafe { libc::setlocale(libc::LC_TIME, c"".as_ptr()) };
    });
}

fn format_local(format: &std::ffi::CStr) -> Option<String> {
    ensure_time_locale();

    let mut raw: libc::time_t = 0;
    // SAFETY: `time` writes a single `time_t` through a pointer we own.
    let now = unsafe { libc::time(&raw mut raw) };
    if now == -1 {
        return None;
    }
    calendar::refresh_timezone(now);

    // SAFETY: `tm` is a plain C struct of integers and a pointer for which
    // all-zero bytes is a valid value; `localtime_r` overwrites it below.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers address locals that outlive the call, and
    // `localtime_r` writes only the `tm` we hand it.
    let filled = unsafe { libc::localtime_r(&raw const now, &raw mut tm) };
    if filled.is_null() {
        return None;
    }

    let mut out = [0u8; 96];
    // SAFETY: `out` is a live buffer of the length we pass, `format` is a
    // NUL-terminated string, and `tm` was filled by `localtime_r` above.
    let written = unsafe { libc::strftime(out.as_mut_ptr().cast(), out.len(), format.as_ptr(), &raw const tm) };
    if written == 0 {
        return None;
    }
    std::str::from_utf8(&out[..written]).ok().map(collapse_spaces)
}

fn collapse_spaces(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[must_use]
pub fn local_clock() -> Option<String> {
    format_local(c"%H:%M")
}

#[must_use]
pub fn local_minute_of_day() -> Option<u16> {
    let clock = local_clock()?;
    let (hours, minutes) = clock.split_once(':')?;
    let hours: u16 = hours.parse().ok()?;
    let minutes: u16 = minutes.parse().ok()?;
    Some(hours * 60 + minutes)
}

fn button_width(painter: &crate::paint::Painter, label: &str) -> i32 {
    (text_w(painter, label, &TextStyle::TOPBAR) + BUTTON_PAD_X * 2).max(BUTTON_MIN_WIDTH)
}

fn segment_width(painter: &crate::paint::Painter, label: &str) -> i32 {
    (text_w(painter, label, &SEG_STYLE) + SEG_TEXT_PAD * 2).max(SEG_MIN_WIDTH)
}

pub const MEDIA_ART: i32 = 17;
pub const MEDIA_ART_LARGE: i32 = 56;

#[derive(Debug, Clone, PartialEq)]
pub struct MediaWidget {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub length: Option<Duration>,
    pub source: String,
    pub art: Option<Vec<u8>>,
    pub art_large: Option<Vec<u8>>,
    pub app_icons: Vec<String>,
    pub accent: Color,
    pub playing: bool,
    pub can_next: bool,
    pub can_previous: bool,
    pub progress: Option<f32>,
}

impl MediaWidget {
    #[must_use]
    pub fn progress_permille(&self) -> Option<u16> {
        progress_permille(self.progress)
    }
}

fn progress_permille(progress: Option<f32>) -> Option<u16> {
    progress.map(|value| (value.clamp(0.0, 1.0) * 1000.0).round() as u16)
}

const fn transport_enabled(media: &MediaWidget, index: usize) -> bool {
    match index {
        0 => media.can_previous,
        2 => media.can_next,
        _ => true,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PopupPlacement {
    position: Point<f64, Logical>,
    size: Size<i32, Logical>,
}

fn popup_placement(popup: Rectangle<i32, Logical>, element: Rectangle<i32, Logical>, progress: f64) -> PopupPlacement {
    let p = progress.clamp(0.0, 1.0);
    let zoom = POPUP_SCALE_FROM + (1.0 - POPUP_SCALE_FROM) * p;
    let rise = -POPUP_RISE * (1.0 - p);
    let anchor_x = f64::from(popup.loc.x) + f64::from(popup.size.w) / 2.0;
    let anchor_y = f64::from(popup.loc.y);
    let x = anchor_x + (f64::from(element.loc.x) - anchor_x) * zoom;
    let y = anchor_y + (f64::from(element.loc.y) - anchor_y) * zoom + rise;
    let size = Size::from((
        (f64::from(element.size.w) * zoom).round() as i32,
        (f64::from(element.size.h) * zoom).round() as i32,
    ));
    PopupPlacement { position: Point::from((x, y)), size }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Placed {
    popup: Rectangle<i32, Logical>,
    element: Rectangle<i32, Logical>,
    progress: f64,
    alpha: f32,
}

fn push_placed<R>(
    renderer: &mut R, buffer: &MemoryRenderBuffer, placed: Placed, scale: Scale<f64>,
    elements: &mut Vec<MemoryRenderBufferRenderElement<R>>,
) where
    R: ShellRenderer,
{
    push_placed_from(renderer, buffer, placed, None, scale, elements);
}

fn push_placed_from<R>(
    renderer: &mut R, buffer: &MemoryRenderBuffer, placed: Placed, origin: Option<Point<i32, Logical>>,
    scale: Scale<f64>, elements: &mut Vec<MemoryRenderBufferRenderElement<R>>,
) where
    R: ShellRenderer,
{
    let progress = placed.progress.clamp(0.0, 1.0);
    if progress <= 0.0 || placed.alpha <= 0.0 || placed.element.size.w <= 0 || placed.element.size.h <= 0 {
        return;
    }
    let alpha = (placed.alpha * progress as f32).clamp(0.0, 1.0);
    let crop = origin.map(|at| Rectangle::new(at.to_f64(), placed.element.size.to_f64()));
    let (position, src, size) = if progress >= 1.0 {
        (placed.element.loc.to_f64(), crop, crop.map(|_| placed.element.size))
    } else {
        let placement = popup_placement(placed.popup, placed.element, progress);
        let src = crop.unwrap_or_else(|| Rectangle::from_size(placed.element.size.to_f64()));
        (placement.position, Some(src), Some(placement.size))
    };
    if let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
        renderer,
        position.to_physical(scale),
        buffer,
        Some(alpha),
        src,
        size,
        Kind::Unspecified,
    ) {
        elements.push(element);
    }
}

fn push_popup_buffer<R>(
    renderer: &mut R, buffer: &MemoryRenderBuffer, popup: Rectangle<i32, Logical>, element: Rectangle<i32, Logical>,
    progress: f64, scale: Scale<f64>, elements: &mut Vec<MemoryRenderBufferRenderElement<R>>,
) where
    R: ShellRenderer,
{
    push_placed(renderer, buffer, Placed { popup, element, progress, alpha: 1.0 }, scale, elements);
}

#[allow(clippy::too_many_arguments)]
fn push_popup_shadow<R>(
    topbar: &mut Topbar, renderer: &mut R, rect: Rectangle<i32, Logical>, progress: f64, scale: Scale<f64>,
    output_scale: i32, elements: &mut Vec<MemoryRenderBufferRenderElement<R>>,
) where
    R: ShellRenderer,
{
    push_rounded_popup_shadow(topbar, renderer, (rect, POPUP_RADIUS), progress, scale, output_scale, elements);
}

#[allow(clippy::too_many_arguments)]
fn push_rounded_popup_shadow<R>(
    topbar: &mut Topbar, renderer: &mut R, (rect, radius): (Rectangle<i32, Logical>, f32), progress: f64,
    scale: Scale<f64>, output_scale: i32, elements: &mut Vec<MemoryRenderBufferRenderElement<R>>,
) where
    R: ShellRenderer,
{
    let margin = popup_shadow_margin(topbar.theme.dark);
    let key = (rect.size.w, rect.size.h, output_scale, radius.to_bits());
    if topbar.shadows.len() >= SHADOW_CACHE_LIMIT && !topbar.shadows.contains_key(&key) {
        topbar.shadows.clear();
    }
    if let std::collections::hash_map::Entry::Vacant(entry) = topbar.shadows.entry(key)
        && let Some(buffer) = draw_popup_shadow(topbar.theme.dark, rect.size, (margin, radius), output_scale)
    {
        entry.insert(buffer);
    }
    let Some(buffer) = topbar.shadows.get(&key) else {
        return;
    };
    let shadow = Rectangle::new(
        Point::from((rect.loc.x - margin, rect.loc.y - margin)),
        Size::from((rect.size.w + margin * 2, rect.size.h + margin * 2)),
    );
    push_popup_buffer(renderer, buffer, rect, shadow, progress, scale, elements);
}

fn popup_shadow_layers(dark: bool) -> Vec<glide_theme::elevation::ShadowLayer> {
    let appearance = if dark { glide_theme::Appearance::Dark } else { glide_theme::Appearance::Light };
    glide_theme::elevation::layers(glide_theme::elevation::Level::El3, appearance)
}

fn popup_shadow_margin(dark: bool) -> i32 {
    let reach = popup_shadow_layers(dark).iter().map(|layer| layer.blur + layer.dy).fold(0.0_f32, f32::max);
    reach.ceil() as i32 + glide_theme::spacing::SP1
}

fn draw_popup_shadow(
    dark: bool, size: Size<i32, Logical>, (margin, radius): (i32, f32), scale: i32,
) -> Option<MemoryRenderBuffer> {
    Some(crate::raster::shadow((size.w, size.h), margin, radius, &popup_shadow_layers(dark), scale)?.finish())
}

fn draw_media_art(
    c: &mut Canvas<'_>, painter: &crate::paint::Painter, media: &MediaWidget,
    (art, (x, y, side)): (Option<&[u8]>, (i32, i32, i32)), radius: f32,
) {
    if let Some(art) = art {
        c.blit_square(art, side, (x, y));
        return;
    }
    if let Some(loader) = c.icons.as_deref_mut() {
        for candidate in &media.app_icons {
            if let Some(pix) = loader.load(candidate, side * c.scale) {
                crate::paint::blit_square_over(&mut c.buf, c.w, c.h, pix, side, x, y, c.scale);
                return;
            }
        }
    }
    c.fill(local_rect(x, y, side, side), media.accent, radius);
    let white = Color([1.0, 1.0, 1.0, 1.0]);
    let glyph = side * 3 / 5;
    let offset = (side - glyph) / 2;
    if c.icon("audio-x-generic-symbolic", glyph, (x + offset, y + offset), white) {
        return;
    }
    let style = if side >= MEDIA_ART_LARGE { &TextStyle::DISPLAY } else { &TextStyle::TOPBAR };
    c.text(painter, (x, x + side, y, y + side), TextAlign::Center, "♪", white, style);
}

fn widest_digits(text: &str) -> String {
    text.chars().map(|c| if c.is_ascii_digit() { '8' } else { c }).collect()
}

fn numeric_label_width(painter: &crate::paint::Painter, text: &str, style: &glide_theme::TextStyle) -> i32 {
    let actual = painter.text_width_styled(text, style);
    let widest = painter.text_width_styled(&widest_digits(text), style);
    actual.max(widest).ceil() as i32
}

fn app_chip_width(painter: &crate::paint::Painter, name: &str, has_icon: bool) -> i32 {
    let icon = if has_icon { APP_ICON + APP_GAP } else { 0 };
    APP_PAD_LEFT + icon + text_w(painter, name, &APP_STYLE) + APP_PAD_RIGHT
}

#[allow(clippy::struct_excessive_bools)] // Independent [topbar] switches, not a state machine
#[derive(Debug, Clone, PartialEq)]
pub struct TopbarTheme {
    pub dark: bool,
    pub background: Color,
    pub popover_bg: Color,
    pub border: Color,
    pub label: Color,
    pub muted: Color,
    pub faint: Color,
    pub button_hover: Color,
    pub button_press: Color,
    pub open_fill: Color,
    pub track: Color,
    pub track_soft: Color,
    pub segment_active: Color,
    pub thumb: Color,
    pub active_background: Color,
    pub active_label: Color,
    pub active_soft: Color,
    pub active_soft_label: Color,
    pub ok: Color,
    pub danger: Color,
    pub ok_soft: Color,
    pub danger_soft: Color,
    pub warn: Color,
    pub line: Color,
    pub line_2: Color,
    pub fill_2: Color,
    pub bezel_hi: Color,
    pub acc_ring: Color,
    pub island_bg: Color,
    pub island_line: Color,
    pub ghost: Color,
    pub card: Color,
    pub focus_line: Color,
    pub view_bg: Color,
    pub battery_low_pct: u8,
    pub battery_saver: bool,
    pub battery_percentage: bool,
    pub clock_date: bool,
    pub clock_24h: Option<bool>,
    pub privacy_mic: Color,
    pub privacy_camera: Color,
    pub privacy_screen: Color,
}

impl TopbarTheme {
    #[must_use]
    pub fn from_tokens(t: &glide_theme::Tokens) -> Self {
        let dark = t.appearance == glide_theme::Appearance::Dark;
        let ramp_step = if dark { 2 } else { 3 };
        Self {
            battery_percentage: true,
            clock_date: true,
            clock_24h: None,
            privacy_mic: glide_theme::Accent::Orange.ramp()[ramp_step],
            privacy_camera: t.ok,
            privacy_screen: glide_theme::Accent::Purple.ramp()[ramp_step],
            dark,
            background: t.shell_bg,
            popover_bg: t.popover_bg,
            border: t.shell_line,
            label: t.shell_fg,
            muted: t.fg_2,
            faint: t.fg_3,
            button_hover: t.state.with_alpha(glide_theme::motion::ST_HOVER),
            button_press: t.state.with_alpha(glide_theme::motion::ST_PRESS),
            open_fill: t.fill_2,
            track: t.fill_3,
            track_soft: t.fill_1,
            segment_active: t.fill_3,
            thumb: if dark {
                Color::from_hex("#cfccc7").expect("literal is valid")
            } else {
                Color([1.0, 1.0, 1.0, 1.0])
            },
            active_background: t.acc_bg,
            active_label: t.acc_fg,
            active_soft: t.acc_soft,
            active_soft_label: t.acc_soft_fg,
            ok: t.ok,
            danger: t.danger,
            ok_soft: t.ok_soft,
            danger_soft: t.danger_soft,
            warn: t.warn,
            line: t.line,
            line_2: t.line_2,
            fill_2: t.fill_2,
            bezel_hi: Color::from_rgb8(255, 255, 255, if dark { BEZEL_HI_DARK } else { BEZEL_HI_LIGHT }),
            acc_ring: t.acc_ring,
            island_bg: t.island_bg,
            island_line: t.island_line,
            ghost: t.fg_4,
            card: t.card_bg,
            focus_line: t.acc_text,
            view_bg: t.view_bg,
            battery_low_pct: BATTERY_LOW_PCT,
            battery_saver: false,
        }
    }

    #[must_use]
    pub fn revision(&self) -> u64 {
        glide_theme::Revision::new()
            .bool(self.dark)
            .color(self.background)
            .color(self.popover_bg)
            .color(self.border)
            .color(self.label)
            .color(self.muted)
            .color(self.faint)
            .color(self.button_hover)
            .color(self.button_press)
            .color(self.open_fill)
            .color(self.track)
            .color(self.track_soft)
            .color(self.segment_active)
            .color(self.thumb)
            .color(self.active_background)
            .color(self.active_label)
            .color(self.active_soft)
            .color(self.active_soft_label)
            .color(self.ok)
            .color(self.danger)
            .color(self.ok_soft)
            .color(self.danger_soft)
            .color(self.line)
            .color(self.line_2)
            .color(self.fill_2)
            .color(self.bezel_hi)
            .color(self.acc_ring)
            .color(self.island_bg)
            .color(self.island_line)
            .color(self.ghost)
            .color(self.card)
            .color(self.focus_line)
            .color(self.view_bg)
            .u64(u64::from(self.battery_low_pct))
            .bool(self.battery_percentage)
            .bool(self.clock_date)
            .u64(self.clock_24h.map_or(0, |h24| 1 + u64::from(h24)))
            .color(self.privacy_mic)
            .color(self.privacy_camera)
            .color(self.privacy_screen)
            .finish()
    }
}

impl Default for TopbarTheme {
    fn default() -> Self {
        let tokens = glide_theme::Tokens::new(glide_theme::Appearance::Dark, glide_theme::Accent::Blue);
        Self::from_tokens(&tokens)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn render<R>(
    topbar: &mut Topbar, renderer: &mut R, painter: &crate::paint::Painter, icons: &mut crate::icon::IconLoader,
    output: Rectangle<i32, Logical>, scale: Scale<f64>, output_scale: i32,
    (backdrop, bar_hidden): (Option<&glide_shell::raster::Backdrop<'_>>, bool),
) -> Vec<MemoryRenderBufferRenderElement<R>>
where
    R: ShellRenderer,
{
    topbar.settle_motion();
    topbar.invalidate_stale_buffers();
    if !topbar.is_home(output) {
        if bar_hidden {
            return Vec::new();
        }
        let mut frame = Frame { renderer, painter, icons, scale, output_scale, elements: Vec::new() };
        render_passive(topbar, &mut frame, output, backdrop);
        return frame.elements;
    }
    topbar.layout_slots(painter, output);

    let strip = topbar.rect(output);
    let mut frame = Frame { renderer, painter, icons, scale, output_scale, elements: Vec::new() };

    {
        let (renderer, painter, _, scale, output_scale, elements) = frame.parts();
        face_tip::push(topbar, renderer, painter, output, (scale, output_scale), elements);
    }

    if bar_hidden {
        if topbar.floats() {
            push_state_layers(topbar, &mut frame);
            island::push(topbar, &mut frame, output);
        }
        return frame.elements;
    }

    push_state_layers(topbar, &mut frame);
    push_quick_settings(topbar, &mut frame);
    island::push(topbar, &mut frame, output);
    push_global_menu(topbar, &mut frame);
    push_app_chip(topbar, &mut frame, strip);
    push_layout(topbar, &mut frame);
    push_caffeine(topbar, &mut frame);
    push_status(topbar, &mut frame);
    push_panel(topbar, &mut frame, strip, backdrop);

    frame.elements
}

fn render_passive<R: ShellRenderer>(
    topbar: &mut Topbar, frame: &mut Frame<'_, R>, output: Rectangle<i32, Logical>,
    backdrop: Option<&glide_shell::raster::Backdrop<'_>>,
) {
    topbar.with_passive(output, |topbar| {
        topbar.layout_passive(frame.painter, output);
        let strip = topbar.rect(output);
        island::push_compact_only(topbar, frame, output);
        push_global_menu(topbar, frame);
        push_app_chip(topbar, frame, strip);
        push_layout(topbar, frame);
        push_caffeine(topbar, frame);
        push_status(topbar, frame);
        push_panel(topbar, frame, strip, backdrop);
    });
}

struct Frame<'a, R: ShellRenderer> {
    renderer: &'a mut R,
    painter: &'a crate::paint::Painter,
    icons: &'a mut crate::icon::IconLoader,
    scale: Scale<f64>,
    output_scale: i32,
    elements: Vec<MemoryRenderBufferRenderElement<R>>,
}

type Parts<'a, 'f, R> = (
    &'f mut R,
    &'a crate::paint::Painter,
    &'f mut crate::icon::IconLoader,
    Scale<f64>,
    i32,
    &'f mut Vec<MemoryRenderBufferRenderElement<R>>,
);

impl<'a, R: ShellRenderer> Frame<'a, R> {
    fn parts(&mut self) -> Parts<'a, '_, R> {
        (&mut *self.renderer, self.painter, &mut *self.icons, self.scale, self.output_scale, &mut self.elements)
    }
}

fn push_buffer<R>(
    renderer: &mut R, buffer: &MemoryRenderBuffer, position: Point<i32, Logical>, scale: Scale<f64>,
    elements: &mut Vec<MemoryRenderBufferRenderElement<R>>,
) where
    R: ShellRenderer,
{
    if let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
        renderer,
        position.to_f64().to_physical(scale),
        buffer,
        Some(1.0),
        None,
        None,
        Kind::Unspecified,
    ) {
        elements.push(element);
    }
}

fn push_panel<R: ShellRenderer>(
    topbar: &mut Topbar, frame: &mut Frame<'_, R>, strip: Rectangle<i32, Logical>,
    backdrop: Option<&glide_shell::raster::Backdrop<'_>>,
) {
    let (renderer, _, _, scale, output_scale, elements) = frame.parts();
    let key = (
        strip.size.w,
        output_scale,
        backdrop.map_or(0, |source| source.generation),
        backdrop.map_or((0, 0, 0), |source| (source.width, source.height, source.fractional_scale.to_bits())),
    );
    if !topbar.panel.contains_key(&key)
        && let Some(buffer) = draw_panel(&topbar.theme, strip, output_scale, backdrop)
    {
        topbar.panel.insert(key, buffer);
    }
    let Some(buffer) = topbar.panel.get(&key) else {
        return;
    };
    push_buffer(renderer, buffer, strip.loc, scale, elements);
}

fn draw_panel(
    theme: &TopbarTheme, strip: Rectangle<i32, Logical>, scale: i32,
    backdrop: Option<&glide_shell::raster::Backdrop<'_>>,
) -> Option<MemoryRenderBuffer> {
    let (w, h) = (strip.size.w, strip.size.h);
    let mut c = Canvas::new(w, h, scale)?;
    if backdrop.is_some_and(|source| source.blit(&mut c.buf, w, h, 0.0, Rectangle::from_size(strip.size), scale)) {
        c.fill(local_rect(0, 0, w, h), theme.background, 0.0);
    } else {
        let background = theme.background;
        crate::paint::Painter::fill_panel_background(&mut c.buf, w, h, scale, background, 0.0, [false; 4]);
    }
    c.fill(local_rect(0, h - 1, w, 1), theme.border, 0.0);
    Some(c.finish())
}

fn push_global_menu<R: ShellRenderer>(topbar: &mut Topbar, frame: &mut Frame<'_, R>) {
    push_lang_menu(topbar, frame);
    push_app_menu(topbar, frame);
    push_menu_titles(topbar, frame);
    let (renderer, painter, _, scale, output_scale, elements) = frame.parts();
    tray::push_tray(topbar, renderer, painter, scale, output_scale, elements);
}

fn push_menu_titles<R: ShellRenderer>(topbar: &mut Topbar, frame: &mut Frame<'_, R>) {
    let (renderer, painter, _, scale, output_scale, elements) = frame.parts();
    let open = (topbar.open_popup == Some(OpenPopup::AppMenu) && topbar.tray_menu_id().is_none())
        .then(|| topbar.menu_open.top_index());
    let titles: Vec<(usize, Rectangle<i32, Logical>)> = topbar
        .slots
        .iter()
        .filter_map(|(slot, rect)| match slot {
            Slot::MenuTitle(index) => Some((usize::from(*index), *rect)),
            _ => None,
        })
        .collect();
    for (index, rect) in titles {
        let is_open = open == Some(index);
        let lit = is_open || topbar.hovered == u16::try_from(index).ok().map(Slot::MenuTitle);
        let logo = topbar.desktop_menu_logo(index);
        let key = (index, is_open, lit, logo, output_scale);
        if !topbar.menu_titles.contains_key(&key)
            && let Some(item) = topbar.menu_bar.get(index)
            && let Some(buffer) = if logo {
                crate::global_menu::draw_logo_title(
                    rect.size.w,
                    rect.size.h,
                    is_open.then_some(topbar.theme.open_fill),
                    output_scale,
                )
            } else {
                crate::global_menu::draw_title(
                    painter,
                    &item.label,
                    rect.size.w,
                    rect.size.h,
                    hover_colour(&topbar.theme, lit),
                    is_open.then_some(topbar.theme.open_fill),
                    output_scale,
                )
            }
        {
            topbar.menu_titles.insert(key, buffer);
        }
        if let Some(buffer) = topbar.menu_titles.get(&key) {
            push_buffer(renderer, buffer, rect.loc, scale, elements);
        }
    }
}

fn push_app_menu<R: ShellRenderer>(topbar: &mut Topbar, frame: &mut Frame<'_, R>) {
    let (renderer, painter, _, scale, output_scale, elements) = frame.parts();
    if topbar.shown_popup() != Some(OpenPopup::AppMenu) {
        return;
    }
    let Some(anchor) = topbar.popup_rect else {
        return;
    };
    let reveal = topbar.popup_progress();
    let theme = &topbar.theme;
    let colors = crate::global_menu::MenuColors {
        background: theme.popover_bg,
        border: theme.border,
        text: theme.label,
        accel: theme.faint,
        disabled: theme.faint,
        mark: theme.active_background,
        open_row: theme.button_hover,
    };
    let panels = topbar.menu_panels.clone();
    let open = topbar.menu_open;
    topbar.menu_births.retain(|path, _| panels.iter().any(|panel| panel.parent == *path));
    for panel in &panels {
        topbar.menu_births.entry(panel.parent).or_insert_with(|| Anim::at(0.0)).retarget(1.0, motion::t2(), E_OUT);
    }
    for panel in panels.iter().rev() {
        let depth = panel.parent.len();
        let birth = topbar.menu_births.get(&panel.parent).map_or(1.0, |birth| birth.value().clamp(0.0, 1.0));
        let (anchor, reveal) = if depth == 0 { (anchor, reveal.min(birth)) } else { (panel.rect, birth) };
        let open_row = (open.len() > depth && open.starts_with(panel.parent)).then(|| open.truncated(depth + 1).last());
        let key = (panel.parent, open_row, output_scale);
        if !topbar.menu_rasters.contains_key(&key)
            && let Some(item) = crate::global_menu::item_at(topbar.menu_items(), panel.parent)
            && let Some(buffer) = crate::global_menu::draw_panel(&crate::global_menu::PanelDraw {
                painter,
                colors: &colors,
                items: &item.children,
                marks: panel.marks,
                size: panel.rect.size,
                open_row,
                scale: output_scale,
            })
        {
            topbar.menu_rasters.insert(key, buffer);
        }
        if let Some(buffer) = topbar.menu_rasters.get(&key) {
            push_popup_buffer(renderer, buffer, anchor, panel.rect, reveal, scale, elements);
        }
        push_popup_shadow(topbar, renderer, panel.rect, reveal, scale, output_scale, elements);
    }
}

fn push_app_chip<R: ShellRenderer>(topbar: &mut Topbar, frame: &mut Frame<'_, R>, strip: Rectangle<i32, Logical>) {
    let (renderer, painter, icons, scale, output_scale, elements) = frame.parts();
    if topbar.app_name.is_empty() {
        return;
    }

    let theme = &topbar.theme;
    let name = &topbar.app_name;
    let app_id = &topbar.app_id;
    let mut chip_w = topbar.app_chip_w;
    let buffer = topbar
        .app_cache
        .resolve((app_id.clone(), name.clone()), output_scale, || {
            let icon = icons.load(app_id, APP_ICON * scale_or_one(output_scale));
            chip_w = app_chip_width(painter, name, icon.is_some());
            draw_app_chip(painter, theme, name, icon, chip_w, output_scale)
        })
        .cloned();
    if chip_w != topbar.app_chip_w {
        topbar.app_chip_w = chip_w;
        topbar.touch();
    }
    let Some(buffer) = buffer else {
        return;
    };
    let buffer = &buffer;

    let position = Point::from((strip.loc.x + PAD_X, strip.loc.y + (HEIGHT - BUTTON_HEIGHT) / 2));
    push_buffer(renderer, buffer, position, scale, elements);
}

fn draw_app_chip(
    painter: &crate::paint::Painter, theme: &TopbarTheme, name: &str, icon: Option<&[u8]>, width: i32, scale: i32,
) -> Option<MemoryRenderBuffer> {
    let mut c = Canvas::new(width, BUTTON_HEIGHT, scale)?;
    let mut text_left = APP_PAD_LEFT;
    if let Some(icon) = icon {
        let top = (BUTTON_HEIGHT - APP_ICON) / 2;
        c.blit(icon, local_rect(APP_PAD_LEFT, top, APP_ICON, APP_ICON));
        text_left += APP_ICON + APP_GAP;
    }
    let span = (text_left, width - APP_PAD_RIGHT);
    c.label(painter, span, TextAlign::Left, name, theme.label, &APP_STYLE);
    Some(c.finish())
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct BadgePop {
    count: usize,
    pop: f64,
}

impl BadgePop {
    fn key(self) -> u8 {
        quantize(self.pop, BADGE_QUANTUM)
    }

    fn scale(self) -> f64 {
        lerp(BADGE_SCALE_FROM, 1.0, self.pop.max(0.0))
    }

    fn visible(self) -> bool {
        self.count > 0 && self.pop > 0.0
    }
}

fn scaled_about_centre(rect: LocalRect, factor: f64) -> LocalRect {
    let w = (f64::from(rect.size.w) * factor).round() as i32;
    let h = (f64::from(rect.size.h) * factor).round() as i32;
    local_rect(rect.loc.x + (rect.size.w - w) / 2, rect.loc.y + (rect.size.h - h) / 2, w.max(0), h.max(0))
}

fn push_layout<R: ShellRenderer>(topbar: &mut Topbar, frame: &mut Frame<'_, R>) {
    let (renderer, painter, icons, scale, output_scale, elements) = frame.parts();
    let Some(rect) = topbar.layout_control_rect() else {
        return;
    };
    if topbar.layouts.is_empty() {
        return;
    }

    let theme = &topbar.theme;
    let layouts = &topbar.layouts;
    let active = topbar.layout_index;
    let hovered = topbar.hovered_in(Slot::in_layout);
    let open = topbar.open_popup == Some(OpenPopup::LanguageMenu);
    let segmented = topbar.segmented();
    let width = rect.size.w;
    let pill = topbar.segment_anim.value();

    let buffer = topbar.layout_cache.resolve(
        (topbar.layouts_rev, active, hovered, open, quantize(pill, MOTION_QUANTUM)),
        output_scale,
        || {
            let label = layouts.get(active).map_or("", String::as_str);
            if segmented {
                draw_segmented(painter, theme, layouts, SegmentState { active, hovered, pill }, width, output_scale)
            } else if layouts.len() >= 3 {
                let state = (hovered == Some(Slot::LayoutDropdown), open);
                draw_lang_dropdown_button(painter, theme, label, width, state, output_scale, Some(icons))
            } else {
                draw_button(painter, theme, label, width, hovered == Some(Slot::LayoutCycle), output_scale)
            }
        },
    );
    let Some(buffer) = buffer else {
        return;
    };
    push_buffer(renderer, buffer, rect.loc, scale, elements);
}

fn draw_lang_dropdown_button(
    painter: &crate::paint::Painter, theme: &TopbarTheme, text: &str, width: i32, (hovered, open): (bool, bool),
    scale: i32, icons: Option<&mut crate::icon::IconLoader>,
) -> Option<MemoryRenderBuffer> {
    let mut c = Canvas::new(width, BUTTON_HEIGHT, scale)?.with_icons(icons);
    if open {
        c.panel(theme.open_fill, BUTTON_RADIUS);
    }
    let style = &TextStyle::TOPBAR;
    let span = (9, 9 + text_w(painter, text, style));
    c.label(painter, span, TextAlign::Left, text, hover_colour(theme, open || hovered), style);

    let side = 10;
    let chevron = local_rect(width - 6 - side, (BUTTON_HEIGHT - side) / 2, side, side);
    if !glyphs::themed(&mut c, if open { "pan-up" } else { "pan-down" }, side, chevron, theme.faint) {
        c.glyph(chevron, |b, w, h, s| {
            crate::paint::draw_chevron_glyph_over(b, w, h, 9.0, 1.5, open, s, theme.faint);
        });
    }
    Some(c.finish())
}

fn push_lang_menu<R: ShellRenderer>(topbar: &mut Topbar, frame: &mut Frame<'_, R>) {
    let (renderer, painter, icons, scale, output_scale, elements) = frame.parts();
    if topbar.shown_popup() != Some(OpenPopup::LanguageMenu) {
        return;
    }
    let Some(rect) = topbar.popup_rect else {
        return;
    };
    let theme = &topbar.theme;
    let layouts = &topbar.layouts;
    let active = topbar.layout_index;
    let hovered = topbar.hovered_in(Slot::in_lang_menu);
    let width = rect.size.w;
    let height = rect.size.h;
    let reveal = topbar.popup_progress();

    let buffer = topbar.lang_menu_cache.resolve((topbar.layouts_rev, active, hovered), output_scale, || {
        draw_lang_menu(painter, theme, layouts, (active, hovered), (width, height), output_scale, Some(icons))
    });
    let Some(buffer) = buffer else {
        return;
    };
    push_popup_buffer(renderer, buffer, rect, rect, reveal, scale, elements);
    push_popup_shadow(topbar, renderer, rect, reveal, scale, output_scale, elements);
}

fn draw_lang_menu(
    painter: &crate::paint::Painter, theme: &TopbarTheme, layouts: &[String], (active, hovered): (usize, Option<Slot>),
    size: (i32, i32), scale: i32, icons: Option<&mut crate::icon::IconLoader>,
) -> Option<MemoryRenderBuffer> {
    let mut c = Canvas::popover(size, scale, R3, theme.popover_bg, theme.border)?.with_icons(icons);
    let width = size.0;
    let style = &TextStyle::TOPBAR;
    let text_left = LANG_MENU_PAD + 6 + 14 + 6;
    let mut y = LANG_MENU_PAD;
    for (index, code) in layouts.iter().enumerate() {
        let is_active = index == active;
        if is_active {
            let tick_side = 14;
            let top = y + (LANG_MENU_ITEM_HEIGHT - tick_side) / 2;
            let tick = local_rect(LANG_MENU_PAD + 6, top, tick_side, tick_side);
            if !glyphs::themed(&mut c, "object-select", tick_side, tick, theme.active_background) {
                c.glyph(tick, |b, w, h, s| {
                    crate::paint::draw_tick_glyph_over(b, w, h, 13.0, 1.8, s, theme.active_background);
                });
            }
        }
        let (top, bottom) = (y, y + LANG_MENU_ITEM_HEIGHT);
        let colour = hover_colour(theme, is_active || hovered == Some(Slot::LangMenuItem(index)));
        c.text(painter, (text_left, text_left + 40, top, bottom), TextAlign::Left, code, colour, style);
        let native = native_layout_name(code);
        if !native.is_empty() {
            let area = (width - 120, width - LANG_MENU_PAD - 8, top, bottom);
            c.text(painter, area, TextAlign::Right, native, theme.faint, style);
        }
        y += LANG_MENU_ITEM_HEIGHT;
    }

    let divider = local_rect(LANG_MENU_PAD + 4, y + 4, width - (LANG_MENU_PAD + 4) * 2, 1);
    c.fill(divider, theme.border, 0.0);
    y += LANG_MENU_DIVIDER;

    c.text(
        painter,
        (text_left, width - LANG_MENU_PAD - 8, y, y + LANG_MENU_ITEM_HEIGHT),
        TextAlign::Left,
        crate::i18n::t(crate::i18n::Key::InputSettings),
        hover_colour(theme, hovered == Some(Slot::LangMenuSettings)),
        style,
    );
    Some(c.finish())
}

const fn hover_colour(theme: &TopbarTheme, hovered: bool) -> Color {
    if hovered { theme.label } else { theme.muted }
}

fn draw_button(
    painter: &crate::paint::Painter, theme: &TopbarTheme, text: &str, width: i32, hovered: bool, scale: i32,
) -> Option<MemoryRenderBuffer> {
    let mut c = Canvas::new(width, BUTTON_HEIGHT, scale)?;
    let span = (BUTTON_PAD_X, width - BUTTON_PAD_X);
    let colour = hover_colour(theme, hovered);
    c.label(painter, span, TextAlign::Center, text, colour, &TextStyle::TOPBAR);
    Some(c.finish())
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct SegmentState {
    active: usize,
    hovered: Option<Slot>,
    pill: f64,
}

fn segment_spans(painter: &crate::paint::Painter, layouts: &[String]) -> Vec<(i32, i32)> {
    let mut x = SEG_PAD_X + SEG_INSET;
    layouts
        .iter()
        .map(|name| {
            let seg = segment_width(painter, name);
            let span = (x, seg);
            x += seg + SEG_GAP;
            span
        })
        .collect()
}

fn draw_segmented(
    painter: &crate::paint::Painter, theme: &TopbarTheme, layouts: &[String], state: SegmentState, width: i32,
    scale: i32,
) -> Option<MemoryRenderBuffer> {
    let mut c = Canvas::new(width, BUTTON_HEIGHT, scale)?;
    let track_top = (BUTTON_HEIGHT - SEG_HEIGHT) / 2;
    let track = local_rect(SEG_PAD_X, track_top, width - SEG_PAD_X * 2, SEG_HEIGHT);
    c.fill(track, theme.track_soft, BUTTON_RADIUS);

    let spans = segment_spans(painter, layouts);
    if let (Some(first), Some(second)) = (spans.first(), spans.get(1)) {
        let (pill_x, pill_w) = segment_pill(*first, *second, state.pill);
        let pill = local_rect(pill_x, track_top + SEG_INSET, pill_w, SEG_HEIGHT - SEG_INSET * 2);
        let lift = if theme.dark { SEG_LIFT_DARK } else { SEG_LIFT_LIGHT };
        c.fill(
            local_rect(pill.loc.x, pill.loc.y + 1, pill.size.w, pill.size.h),
            Color([0.0, 0.0, 0.0, lift]),
            SEG_RADIUS,
        );
        c.fill(pill, theme.card, SEG_RADIUS);
    }

    for (index, (name, (x, seg))) in layouts.iter().zip(spans).enumerate() {
        let colour = if index == state.active {
            theme.label
        } else if state.hovered == Some(Slot::LayoutPick(index)) {
            theme.muted
        } else {
            theme.faint
        };
        c.label(painter, (x, x + seg), TextAlign::Center, name, colour, &SEG_STYLE);
    }
    Some(c.finish())
}

fn push_caffeine<R: ShellRenderer>(topbar: &mut Topbar, frame: &mut Frame<'_, R>) {
    let (renderer, _, icons, scale, output_scale, elements) = frame.parts();
    let Some(rect) = topbar.slot_rect(Slot::Caffeine) else {
        return;
    };
    let hovered = topbar.hovered == Some(Slot::Caffeine);
    let on = topbar.caffeinated;
    let switch = topbar.caffeine_anim.switch();
    let theme = &topbar.theme;
    let width = rect.size.w;

    let buffer = topbar.coffee_cache.resolve((on, hovered, switch.key()), output_scale, || {
        draw_caffeine(theme, width, hovered, on, switch, output_scale, Some(icons))
    });
    let Some(buffer) = buffer else {
        return;
    };
    push_buffer(renderer, buffer, rect.loc, scale, elements);
}

fn draw_caffeine(
    theme: &TopbarTheme, width: i32, hovered: bool, on: bool, switch: SwitchMotion, scale: i32,
    icons: Option<&mut crate::icon::IconLoader>,
) -> Option<MemoryRenderBuffer> {
    let mut c = Canvas::new(width, BUTTON_HEIGHT, scale)?.with_icons(icons);
    let colour = hover_colour(theme, on || hovered);
    let cup = local_rect(COFFEE_PAD, 0, COFFEE_GLYPH, BUTTON_HEIGHT);
    let name = if on { "caffeine-cup-full" } else { "caffeine-cup-empty" };
    if !glyphs::themed_on_ink(&mut c, (name, "caffeine-cup-empty"), COFFEE_GLYPH, cup, colour) {
        glyphs::glyph(&mut c, glyphs::Glyph::Cup, COFFEE_GLYPH, cup, colour);
    }
    let track_y = (BUTTON_HEIGHT - SWITCH_HEIGHT) / 2;
    let track = local_rect(width - COFFEE_PAD_END - SWITCH_WIDTH, track_y, SWITCH_WIDTH, SWITCH_HEIGHT);
    draw_switch(&mut c, theme, track, (SWITCH_INSET, SWITCH_THUMB), switch);
    Some(c.finish())
}

fn push_status<R: ShellRenderer>(topbar: &mut Topbar, frame: &mut Frame<'_, R>) {
    let (renderer, painter, icons, scale, output_scale, elements) = frame.parts();
    status::push_popups(topbar, renderer, painter, scale, output_scale, elements);
    headset_view::push_popup(topbar, renderer, painter, icons, scale, output_scale, elements);
    phone_view::push_popup(topbar, renderer, painter, icons, scale, output_scale, elements);
    passwords_view::push_popup(topbar, renderer, painter, icons, scale, output_scale, elements);
    status::push_cluster(topbar, renderer, painter, icons, scale, output_scale, elements);
}

const BATTERY_LOW_PCT: u8 = 20;
const BEZEL_HI_DARK: f32 = 0.12;
const BEZEL_HI_LIGHT: f32 = 0.55;

fn push_quick_settings<R: ShellRenderer>(topbar: &mut Topbar, frame: &mut Frame<'_, R>) {
    if topbar.shown_popup() != Some(OpenPopup::QuickSettings) {
        return;
    }
    let Some(rect) = topbar.popup_rect else {
        return;
    };
    let reveal = topbar.popup_progress();
    {
        let (renderer, painter, icons, scale, output_scale, elements) = frame.parts();
        panels::push_detail(topbar, renderer, painter, icons, rect, reveal, scale, output_scale, elements);
    }
    let slide = topbar.qs_panel.slide_progress();
    if slide < 1.0 {
        push_qs_main(topbar, frame, rect, (reveal, slide));
    }
    let (renderer, _, _, scale, output_scale, elements) = frame.parts();
    let theme = &topbar.theme;
    let corner = QS_RADIUS.ceil() as i32 + 1;
    let sliced = reveal >= 1.0 && rect.size.w >= corner * 2 && rect.size.h >= corner * 2;
    let size = if sliced { (corner * 2 + 1, corner * 2 + 1) } else { (rect.size.w, rect.size.h) };
    if let Some(background) =
        topbar.qs_bg_cache.resolve(size, output_scale, || draw_qs_background(size, theme, output_scale))
    {
        if sliced {
            for (src, dst) in island::nine_slices(rect, corner) {
                if let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    dst.loc.to_f64().to_physical(scale),
                    background,
                    None,
                    Some(src),
                    Some(dst.size),
                    Kind::Unspecified,
                ) {
                    elements.push(element);
                }
            }
        } else {
            push_popup_buffer(renderer, background, rect, rect, reveal, scale, elements);
        }
    }
    push_rounded_popup_shadow(topbar, renderer, (rect, QS_RADIUS), reveal, scale, output_scale, elements);
}

fn draw_qs_background((w, h): (i32, i32), theme: &TopbarTheme, scale: i32) -> Option<MemoryRenderBuffer> {
    let c = Canvas::popover((w, h), scale, QS_RADIUS, theme.popover_bg, theme.line)?;
    Some(c.finish())
}

fn qs_main_motion(slide: f64) -> (i32, f32) {
    ((slide * f64::from(QS_PAD)).round() as i32, (1.0 - slide) as f32)
}

fn push_qs_main<R: ShellRenderer>(
    topbar: &mut Topbar, frame: &mut Frame<'_, R>, rect: Rectangle<i32, Logical>, (reveal, slide): (f64, f64),
) {
    let (renderer, painter, icons, scale, output_scale, elements) = frame.parts();
    let (revs, flags, live, hovered, shape) = topbar.qs_main_key(painter);
    let motion = QsMotion { volume: volume_icon_step(live.volume), mic: 0, brightness: 0, ..live };
    let key = (revs, flags, motion, hovered, shape);
    let (shift, alpha) = qs_main_motion(slide);
    let left = rect.loc.x - shift;
    let geometry = qs_geometry(&shape);
    for (buffer, bounds) in qs_slider_layers(topbar, painter, &geometry, (live, hovered, flags.dragging), output_scale)
    {
        let element = Rectangle::new(Point::from((left + bounds.loc.x, rect.loc.y + bounds.loc.y)), bounds.size);
        push_placed(renderer, &buffer, Placed { popup: rect, element, progress: reveal, alpha }, scale, elements);
    }
    let theme = &topbar.theme;
    let status = &topbar.system_status;
    let panel = &topbar.qs_panel;
    let Some(buffer) = topbar.qs_cache.resolve(key, output_scale, || {
        let ctx = QsRenderContext { painter, theme, hovered };
        let mut canvas = Canvas::new(rect.size.w, geometry.height, output_scale)?.with_icons(Some(icons));
        let draw = QsDraw { status, panel, geometry: geometry.clone(), flags, motion, sliders: false };
        draw_quick_settings(&mut canvas, &ctx, &draw);
        Some(canvas.finish())
    }) else {
        return;
    };
    let element = Rectangle::new(Point::from((left, rect.loc.y)), Size::from((rect.size.w, geometry.height)));
    push_placed(renderer, buffer, Placed { popup: rect, element, progress: reveal, alpha }, scale, elements);
}

struct QsRenderContext<'a> {
    painter: &'a crate::paint::Painter,
    theme: &'a TopbarTheme,
    hovered: Option<Slot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QsFlags {
    caffeinated: bool,
    dnd: bool,
    dark: bool,
    dragging: Option<Slot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QsMotion {
    volume: u8,
    mic: u8,
    brightness: u8,
    tiles: [u8; QS_TILE_COUNT],
}

impl QsMotion {
    fn tile(self, tile: QsTile) -> f32 {
        f32::from(self.tiles[tile.index()]) / MOTION_QUANTUM as f32
    }
}

struct QsDraw<'a> {
    status: &'a crate::system_info::SystemStatus,
    panel: &'a panels::DetailPanel,
    geometry: QsGeometry,
    flags: QsFlags,
    motion: QsMotion,
    sliders: bool,
}

fn percent(value: f64) -> u8 {
    value.clamp(0.0, 100.0).round() as u8
}

type GlyphPainter = fn(&mut [u8], i32, i32, f32, f32, i32, Color);

const QS_TITLE: TextStyle = TextStyle { size_px: 14.0, line_height_px: 20.0, weight: 630, ..TextStyle::HEAD };
const QS_TITLE_PAGE: TextStyle = QS_TITLE;
const QS_SUB: TextStyle = TextStyle::CAP;
const QS_NOTE: TextStyle = TextStyle::CAP;
const QS_STRONG: TextStyle = TextStyle { weight: 600, ..TextStyle::CAP };
const QS_PILL_TEXT: TextStyle = TextStyle { size_px: 12.5, line_height_px: 16.0, weight: 520, ..TextStyle::CAP };
const QS_READOUT: TextStyle = TextStyle { size_px: 12.0, line_height_px: 16.0, weight: 450, ..TextStyle::NUM };
const QS_CHIP_PCT: TextStyle = TextStyle { size_px: 12.5, line_height_px: 16.0, weight: 520, ..TextStyle::NUM };
const QS_BATTERY_PCT: TextStyle = TextStyle { size_px: 26.0, line_height_px: 33.0, weight: 650, ..TextStyle::NUM };
const QS_SLIDER_THUMB_DRAG: i32 = 16;
const QS_SLIDER_RING_HOVER: i32 = 4;
const QS_SLIDER_RING_DRAG: i32 = 6;
const QS_THUMB_SHADOW: Color = Color([0.0, 0.0, 0.0, 0.12]);
const QS_THUMB_FACE: Color = Color([1.0, 1.0, 1.0, 1.0]);
const QS_ICON: i32 = 18;
const QS_HAIRLINE_ALPHA: f32 = 0.14;
const QS_MORE_ALPHA: f32 = 0.7;
const QS_PRIVACY_DOT: i32 = 8;
const QS_CHIP_DEFAULT: i32 = 150;
const QS_CHIP_BATTERY: (i32, i32) = (23, 11);

fn qs_symbol(c: &mut Canvas<'_>, rect: LocalRect, (icon, glyph): (&str, GlyphPainter), side: i32, colour: Color) {
    let at = (rect.loc.x + (rect.size.w - side) / 2, rect.loc.y + (rect.size.h - side) / 2);
    if !c.icon(icon, side, at, colour) {
        let glyph_side = (side - 1) as f32;
        c.glyph(rect, |b, w, h, s| glyph(b, w, h, glyph_side, 1.5, s, colour));
    }
}

fn dimmed(colour: Color, factor: f32) -> Color {
    colour.with_alpha(colour.0[3] * factor)
}

fn qs_confirm_pill_width(painter: &crate::paint::Painter, label: &str) -> i32 {
    text_w(painter, label, &QS_PILL_TEXT) + QS_CONFIRM_PILL_PAD * 2
}

struct QsTileSpec<'a> {
    tile: QsTile,
    rect: LocalRect,
    level: f32,
    title: &'a str,
    subtitle: &'a str,
    icon_name: &'a str,
    glyph: GlyphPainter,
}

fn draw_qs_chevron(c: &mut Canvas<'_>, ctx: &QsRenderContext<'_>, rect: LocalRect, colour: Color) {
    let side = QS_MORE_ICON;
    let at = (rect.loc.x + (rect.size.w - side) / 2, rect.loc.y + (rect.size.h - side) / 2);
    if !c.icon("go-next", side, at, colour) {
        c.text_in(ctx.painter, rect, TextAlign::Center, "\u{203a}", colour, &TextStyle::HEAD);
    }
}

fn draw_qs_tile(c: &mut Canvas<'_>, ctx: &QsRenderContext<'_>, spec: &QsTileSpec<'_>) {
    let (rect, theme) = (spec.rect, ctx.theme);
    let lit = spec.level;
    c.fill(rect, theme.track_soft.lerp(theme.active_soft, lit), R2);
    let ink = theme.label.lerp(theme.active_soft_label, lit);
    let sub_ink = theme.muted.lerp(theme.active_soft_label, lit);
    let icon =
        local_rect(rect.loc.x + QS_TILE_PAD, rect.loc.y + (rect.size.h - QS_TILE_ICON) / 2, QS_TILE_ICON, QS_TILE_ICON);
    qs_symbol(c, icon, (spec.icon_name, spec.glyph), QS_TILE_ICON, ink);
    let text_right = if spec.tile.more().is_some() {
        let (_, side) = qs_split(rect);
        let line = local_rect(side.loc.x, rect.loc.y + QS_HAIRLINE_INSET, 1, rect.size.h - QS_HAIRLINE_INSET * 2);
        c.fill(line, dimmed(ink, QS_HAIRLINE_ALPHA), 0.0);
        draw_qs_chevron(c, ctx, side, dimmed(ink, QS_MORE_ALPHA));
        side.loc.x - QS_TILE_PAD_END
    } else {
        rect.loc.x + rect.size.w - QS_TILE_PAD_END
    };
    let text_left = icon.loc.x + QS_TILE_ICON + QS_TILE_PAD;
    let title_h = QS_TITLE.line_height_px as i32;
    let sub_h = QS_SUB.line_height_px as i32;
    let top = rect.loc.y + (rect.size.h - title_h - sub_h) / 2;
    c.text(ctx.painter, (text_left, text_right, top, top + title_h), TextAlign::Left, spec.title, ink, &QS_TITLE);
    let sub_band = (text_left, text_right, top + title_h, top + title_h + sub_h);
    c.text(ctx.painter, sub_band, TextAlign::Left, spec.subtitle, sub_ink, &QS_SUB);
}

fn draw_switch(
    c: &mut Canvas<'_>, theme: &TopbarTheme, track: LocalRect, (inset, thumb): (i32, i32), switch: SwitchMotion,
) {
    let colour = switch.track_colour(theme.track, theme.active_background);
    c.fill(track, colour, R_ROUND);
    let thumb_x = switch.thumb_x(track.loc.x, track.size.w, inset, thumb);
    let knob = local_rect(thumb_x, track.loc.y + inset, thumb, thumb);
    c.fill(knob, theme.thumb, R_ROUND);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum QsThumb {
    Rest,
    Hover,
    Drag,
}

impl QsThumb {
    const fn of(slot: Slot, hovered: Option<Slot>, dragging: Option<Slot>) -> Self {
        if matches!(dragging, Some(dragged) if same_slot(dragged, slot)) {
            Self::Drag
        } else if matches!(hovered, Some(hover) if same_slot(hover, slot)) {
            Self::Hover
        } else {
            Self::Rest
        }
    }

    const fn side(self) -> i32 {
        match self {
            Self::Rest | Self::Hover => QS_SLIDER_THUMB,
            Self::Drag => QS_SLIDER_THUMB_DRAG,
        }
    }

    const fn ring(self) -> i32 {
        match self {
            Self::Rest => 0,
            Self::Hover => QS_SLIDER_RING_HOVER,
            Self::Drag => QS_SLIDER_RING_DRAG,
        }
    }
}

const fn same_slot(a: Slot, b: Slot) -> bool {
    matches!(
        (a, b),
        (Slot::QsVolumeTrack, Slot::QsVolumeTrack)
            | (Slot::QsMicTrack, Slot::QsMicTrack)
            | (Slot::QsBrightnessTrack, Slot::QsBrightnessTrack)
    )
}

#[derive(Debug, Clone, Copy)]
struct QsTrack {
    pct: u8,
    muted: bool,
    enabled: bool,
    thumb: QsThumb,
}

fn draw_qs_track(c: &mut Canvas<'_>, theme: &TopbarTheme, rect: LocalRect, track: QsTrack) {
    let groove_y = rect.loc.y + (rect.size.h - QS_SLIDER_TRACK_H) / 2;
    let bar = |width| local_rect(rect.loc.x, groove_y, width, QS_SLIDER_TRACK_H);
    c.fill(bar(rect.size.w), theme.fill_2, R1);
    if !track.enabled {
        return;
    }
    let fill_w = ((f64::from(rect.size.w) * f64::from(track.pct.min(100))) / 100.0).round() as i32;
    if fill_w > 0 {
        let colour = if track.muted { theme.track } else { theme.active_background };
        c.fill(bar(fill_w), colour, R1);
    }
    let centre = (rect.loc.x + fill_w, rect.loc.y + rect.size.h / 2);
    let circle = |side: i32| local_rect(centre.0 - side / 2, centre.1 - side / 2, side, side);
    let side = track.thumb.side();
    if track.thumb.ring() > 0 {
        c.fill(circle(side + track.thumb.ring() * 2), theme.acc_ring, R_ROUND);
    }
    let shadow = circle(side + 2);
    c.fill(Rectangle::new(Point::from((shadow.loc.x, shadow.loc.y + 1)), shadow.size), QS_THUMB_SHADOW, R_ROUND);
    c.fill(circle(side + 2), theme.line_2, R_ROUND);
    c.fill(circle(side), QS_THUMB_FACE, R_ROUND);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum SliderKind {
    Volume,
    Mic,
    Brightness,
}

type SliderKey = (SliderKind, u8, bool, bool, QsThumb, i32, u64);

const SLIDER_HALO: i32 = 18;

const SLIDER_LAYERS: usize = 128;

const fn volume_icon_step(pct: u8) -> u8 {
    if pct == 0 {
        0
    } else if pct < 33 {
        1
    } else if pct < 66 {
        33
    } else {
        66
    }
}

fn qs_slider_layers(
    topbar: &mut Topbar, painter: &crate::paint::Painter, geometry: &QsGeometry,
    (live, hovered, dragging): (QsMotion, Option<Slot>, Option<Slot>), scale: i32,
) -> Vec<(MemoryRenderBuffer, LocalRect)> {
    let status = &topbar.system_status;
    let mut rows = vec![(
        SliderKind::Volume,
        Slot::QsVolumeTrack,
        geometry.volume,
        QsTrack {
            pct: live.volume,
            muted: status.volume_muted,
            enabled: status.volume_available,
            thumb: QsThumb::Rest,
        },
    )];
    if let Some(mic) = geometry.mic {
        let track = QsTrack { pct: live.mic, muted: status.mic_muted, enabled: true, thumb: QsThumb::Rest };
        rows.push((SliderKind::Mic, Slot::QsMicTrack, mic, track));
    }
    if let Some(row) = geometry.brightness {
        let track = QsTrack { pct: live.brightness, muted: false, enabled: true, thumb: QsThumb::Rest };
        rows.push((SliderKind::Brightness, Slot::QsBrightnessTrack, row, track));
    }

    let theme = &topbar.theme;
    let mut out = Vec::with_capacity(rows.len());
    for (kind, slot, row, track) in rows {
        let track = QsTrack { thumb: QsThumb::of(slot, hovered, dragging), ..track };
        let union = row.readout.merge(row.track);
        let bounds = local_rect(
            union.loc.x - SLIDER_HALO,
            union.loc.y - SLIDER_HALO,
            union.size.w + SLIDER_HALO * 2,
            union.size.h + SLIDER_HALO * 2,
        );
        let key: SliderKey =
            (kind, track.pct, track.muted, track.enabled, track.thumb, scale, crate::paint::painter_generation());
        if !topbar.qs_sliders.contains_key(&key) {
            if topbar.qs_sliders.len() >= SLIDER_LAYERS {
                topbar.qs_sliders.clear();
            }
            let Some(mut c) = Canvas::new(bounds.size.w, bounds.size.h, scale) else {
                continue;
            };
            let shift = |rect: LocalRect| Rectangle::new(rect.loc - bounds.loc, rect.size);
            let ctx = QsRenderContext { painter, theme, hovered };
            let readout = track.enabled.then_some((track.pct, track.muted));
            draw_qs_readout(&mut c, &ctx, shift(row.readout), readout);
            draw_qs_track(&mut c, theme, shift(row.track), track);
            topbar.qs_sliders.insert(key, c.finish());
        }
        if let Some(buffer) = topbar.qs_sliders.get(&key) {
            out.push((buffer.clone(), bounds));
        }
    }
    out
}

fn initial(name: &str) -> String {
    name.chars().next().unwrap_or('?').to_uppercase().to_string()
}

fn qs_percent(pct: u8) -> String {
    format!("{pct}\u{a0}%")
}

fn draw_qs_readout(c: &mut Canvas<'_>, ctx: &QsRenderContext<'_>, rect: LocalRect, value: Option<(u8, bool)>) {
    let text = match value {
        None => "\u{2014}".to_string(),
        Some((_, true)) => crate::i18n::t(crate::i18n::Key::QsMutedShort).to_string(),
        Some((pct, false)) => qs_percent(pct),
    };
    c.text_in(ctx.painter, rect, TextAlign::Right, &text, ctx.theme.muted, &QS_READOUT);
}

fn qs_device_kind(icon: &str) -> crate::i18n::Key {
    use crate::i18n::Key;
    match icon {
        "audio-headset" => Key::QsKindHeadset,
        "audio-headphones" => Key::QsKindHeadphones,
        "video-display" => Key::QsDisplay,
        "audio-input-microphone" => Key::TbMicrophone,
        "camera-web" => Key::QsKindWebcam,
        _ => Key::QsKindSpeakers,
    }
}

const fn wifi_icon_name(level: u8) -> &'static str {
    match level {
        0 => "network-wireless-signal-none",
        1 => "network-wireless-signal-weak",
        2 => "network-wireless-signal-ok",
        3 => "network-wireless-signal-good",
        _ => "network-wireless-signal-excellent",
    }
}

const fn volume_icon_name(muted: bool, pct: u8) -> &'static str {
    if muted || pct == 0 {
        "audio-volume-muted"
    } else if pct < 33 {
        "audio-volume-low"
    } else if pct < 66 {
        "audio-volume-medium"
    } else {
        "audio-volume-high"
    }
}

const fn power_profile_label(profile: PowerProfile) -> crate::i18n::Key {
    match profile {
        PowerProfile::PowerSaver => crate::i18n::Key::QsProfileSaver,
        PowerProfile::Balanced => crate::i18n::Key::QsProfileBalanced,
        PowerProfile::Performance => crate::i18n::Key::QsProfilePerformance,
    }
}

const fn power_profile_icon(profile: PowerProfile) -> &'static str {
    match profile {
        PowerProfile::PowerSaver => "power-profile-power-saver-symbolic",
        PowerProfile::Balanced => "power-profile-balanced-symbolic",
        PowerProfile::Performance => "power-profile-performance-symbolic",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrivacyKind {
    Mic,
    Camera,
    Screen,
}

fn qs_privacy(status: &crate::system_info::SystemStatus) -> Option<(PrivacyKind, &str)> {
    let privacy = &status.privacy;
    [
        (&privacy.microphone, PrivacyKind::Mic),
        (&privacy.camera, PrivacyKind::Camera),
        (&privacy.screen, PrivacyKind::Screen),
    ]
    .into_iter()
    .find_map(|(apps, kind)| apps.first().map(|app| (kind, app.as_str())))
}

fn qs_chip_line(status: &crate::system_info::SystemStatus) -> String {
    use crate::i18n::{Key, t};
    use crate::system_info::BatteryState;
    let Some((state, minutes)) = status.battery.as_ref().and_then(|info| Some((info.state, info.minutes?))) else {
        return String::new();
    };
    let key = match state {
        BatteryState::Discharging => Key::QsBatteryLeftShort,
        BatteryState::Charging => Key::QsBatteryFullShort,
        BatteryState::Full | BatteryState::NotCharging => return String::new(),
    };
    t(key).replace("{}", &crate::i18n::duration_label(minutes))
}

fn qs_chip_width(painter: &crate::paint::Painter, status: &crate::system_info::SystemStatus) -> i32 {
    let pct = status.battery_pct.map_or_else(String::new, qs_percent);
    let line = qs_chip_line(status);
    let tail = if line.is_empty() { 0 } else { SP2 + text_w(painter, &line, &QS_SUB) };
    let width = QS_CHIP_PAD * 2 + QS_CHIP_BATTERY.0 + SP2 + text_w(painter, &pct, &QS_CHIP_PCT) + tail;
    width.min(QS_WIDTH - QS_PAD * 2 - 4 * (QS_ICON_BTN + QS_ROW_GAP))
}

fn qs_network_subtitle(status: &crate::system_info::SystemStatus) -> String {
    use crate::i18n::{Key, t};
    let text = if status.airplane {
        t(Key::AirplaneMode)
    } else if !status.wifi_available {
        t(if status.ethernet_connected { Key::QsEthernetShort } else { Key::QsOffLong })
    } else if !status.wifi_enabled {
        t(Key::QsOffLong)
    } else if status.wifi_ssid.is_empty() {
        t(Key::QsNotConnected)
    } else {
        &status.wifi_ssid
    };
    text.to_string()
}

const fn qs_network_icon(status: &crate::system_info::SystemStatus) -> &'static str {
    if status.airplane {
        "airplane-mode"
    } else if !status.wifi_available {
        "network-wired"
    } else if !status.wifi_enabled {
        "network-wireless-offline"
    } else {
        wifi_icon_name(status.wifi_level)
    }
}

fn qs_bluetooth_subtitle(status: &crate::system_info::SystemStatus, panel: &panels::DetailPanel) -> String {
    use crate::i18n::{Key, t};
    if !status.bluetooth_on {
        return t(Key::QsOffLong).to_string();
    }
    let names = panel.connected_device_names();
    match names.as_slice() {
        [] if status.bluetooth_connected => t(Key::Connected).to_string(),
        [] => t(Key::QsOnLong).to_string(),
        [name] => (*name).to_string(),
        many => {
            let lang = crate::i18n::language();
            let count = u64::try_from(many.len()).unwrap_or(u64::MAX);
            let forms = [Key::QsDeviceOne, Key::QsDeviceFew, Key::QsDeviceMany];
            crate::i18n::plural_key(lang, count, forms).text(lang).replace("{n}", &count.to_string())
        }
    }
}

fn draw_qs_tiles(c: &mut Canvas<'_>, ctx: &QsRenderContext<'_>, draw: &QsDraw<'_>) {
    use crate::i18n::{Key, t};
    let (status, flags) = (draw.status, draw.flags);
    let off = || t(Key::QsOffLong).to_string();
    let on_off = |on: bool| t(if on { Key::QsOnLong } else { Key::QsOffLong }).to_string();
    let moon: GlyphPainter = |b, w, h, side, _, s, c| crate::paint::draw_moon_glyph_over(b, w, h, side, s, c);
    let coffee: GlyphPainter = |b, w, h, side, stroke, s, c| {
        let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
        crate::paint::draw_coffee_glyph_over(b, w, h, cx, cy, side, stroke, s, c, false);
    };
    for &(tile, rect) in &draw.geometry.tiles {
        let (title, subtitle, icon_name, glyph): (&str, String, &str, GlyphPainter) = match tile {
            QsTile::Network => (
                t(Key::QsNetwork),
                qs_network_subtitle(status),
                qs_network_icon(status),
                crate::paint::draw_wifi_waves_glyph_over,
            ),
            QsTile::Bluetooth => (
                t(Key::Bluetooth),
                qs_bluetooth_subtitle(status, draw.panel),
                if status.bluetooth_on { "bluetooth-active" } else { "bluetooth-disabled" },
                crate::paint::draw_bluetooth_glyph_over,
            ),
            QsTile::Vpn => {
                let (netbird_on, netbird_line) = draw.panel.netbird.tile();
                let active = draw.panel.vpns().iter().find(|vpn| vpn.active);
                let subtitle = if netbird_on || draw.panel.netbird.row().1 {
                    netbird_line.to_string()
                } else if let Some(vpn) = active {
                    vpn.name.clone()
                } else if status.vpn_active {
                    t(Key::QsOnLong).to_string()
                } else {
                    off()
                };
                (t(Key::QsVpn), subtitle, "network-vpn", crate::paint::draw_lock_glyph_over)
            }
            QsTile::Saver => {
                let saver = status.power_profile == Some(PowerProfile::PowerSaver);
                let subtitle = match status.power_profile {
                    Some(profile) if !saver => t(power_profile_label(profile)).to_string(),
                    _ => on_off(saver),
                };
                let icon = power_profile_icon(PowerProfile::PowerSaver);
                (t(Key::QsProfileSaver), subtitle, icon, crate::paint::draw_power_glyph_over)
            }
            QsTile::Dnd => {
                let subtitle = if flags.dnd { t(Key::QsDndOnSub).to_string() } else { off() };
                (t(Key::DoNotDisturb), subtitle, "notifications-disabled", moon)
            }
            QsTile::NightLight => (t(Key::NightLight), on_off(status.night_light_on), "night-light", moon),
            QsTile::DarkTheme => (t(Key::QsDarkTheme), on_off(flags.dark), "dark-mode", moon),
            QsTile::Caffeine => {
                let icon = if flags.caffeinated { "caffeine-cup-full" } else { "caffeine-cup-empty" };
                let subtitle = if flags.caffeinated { t(Key::QsAwakeSub).to_string() } else { off() };
                (t(Key::QsStayAwake), subtitle, icon, coffee)
            }
        };
        let level = draw.motion.tile(tile);
        draw_qs_tile(c, ctx, &QsTileSpec { tile, rect, level, title, subtitle: &subtitle, icon_name, glyph });
    }
}

fn draw_qs_mute_button(c: &mut Canvas<'_>, theme: &TopbarTheme, rect: LocalRect, muted: bool) -> Color {
    if muted {
        c.fill(rect, theme.danger_soft, R2);
        theme.danger
    } else {
        theme.muted
    }
}

fn draw_qs_sliders(c: &mut Canvas<'_>, ctx: &QsRenderContext<'_>, draw: &QsDraw<'_>) {
    let (theme, geometry, status) = (ctx.theme, &draw.geometry, draw.status);
    let volume = geometry.volume;
    let (muted, enabled, pct) = (status.volume_muted, status.volume_available, draw.motion.volume);
    let colour = if enabled { draw_qs_mute_button(c, theme, volume.icon, muted) } else { theme.faint };
    let speaker: GlyphPainter = if muted {
        |b, w, h, side, stroke, s, c| crate::paint::draw_speaker_glyph_over(b, w, h, side, stroke, true, s, c)
    } else {
        |b, w, h, side, stroke, s, c| crate::paint::draw_speaker_glyph_over(b, w, h, side, stroke, false, s, c)
    };
    qs_symbol(c, volume.icon, (volume_icon_name(muted, pct), speaker), QS_ICON, colour);
    if let Some(drill) = volume.drill {
        draw_qs_chevron(c, ctx, drill, theme.faint);
    }
    if draw.sliders {
        draw_qs_readout(c, ctx, volume.readout, enabled.then_some((pct, muted)));
        let thumb = QsThumb::of(Slot::QsVolumeTrack, ctx.hovered, draw.flags.dragging);
        draw_qs_track(c, theme, volume.track, QsTrack { pct, muted, enabled, thumb });
    }

    if let Some(mic) = geometry.mic {
        let (muted, pct) = (status.mic_muted, draw.motion.mic);
        let colour = draw_qs_mute_button(c, theme, mic.icon, muted);
        draw_qs_mic_glyph(c, mic.icon, (muted, colour));
        if let Some(drill) = mic.drill {
            draw_qs_chevron(c, ctx, drill, theme.faint);
        }
        if draw.sliders {
            draw_qs_readout(c, ctx, mic.readout, Some((pct, muted)));
            let thumb = QsThumb::of(Slot::QsMicTrack, ctx.hovered, draw.flags.dragging);
            draw_qs_track(c, theme, mic.track, QsTrack { pct, muted, enabled: true, thumb });
        }
    }

    if let Some(brightness) = geometry.brightness {
        let pct = draw.motion.brightness;
        let sun: GlyphPainter = crate::paint::draw_sun_glyph_over;
        qs_symbol(c, brightness.icon, ("weather-clear", sun), QS_ICON, theme.muted);
        if draw.sliders {
            draw_qs_readout(c, ctx, brightness.readout, Some((pct, false)));
            let thumb = QsThumb::of(Slot::QsBrightnessTrack, ctx.hovered, draw.flags.dragging);
            draw_qs_track(c, theme, brightness.track, QsTrack { pct, muted: false, enabled: true, thumb });
        }
    }
}

fn draw_qs_strip(
    c: &mut Canvas<'_>, ctx: &QsRenderContext<'_>, rect: LocalRect, status: &crate::system_info::SystemStatus,
) {
    use crate::i18n::{Key, t};
    let Some((kind, app)) = qs_privacy(status) else {
        return;
    };
    let theme = ctx.theme;
    c.fill(rect, theme.track_soft, R2);
    let (colour, label, tail) = match kind {
        PrivacyKind::Mic => (theme.privacy_mic, Key::QsStripMic, Key::QsStripMicBy),
        PrivacyKind::Camera => (theme.privacy_camera, Key::QsStripCamera, Key::QsStripCameraBy),
        PrivacyKind::Screen => (theme.privacy_screen, Key::QsStripScreen, Key::QsStripScreenBy),
    };
    let dot_x = rect.loc.x + QS_TILE_PAD;
    let dot = local_rect(dot_x, rect.loc.y + (rect.size.h - QS_PRIVACY_DOT) / 2, QS_PRIVACY_DOT, QS_PRIVACY_DOT);
    c.fill(dot, colour, R_ROUND);
    let (top, bottom) = (rect.loc.y, rect.loc.y + rect.size.h);
    let left = dot_x + QS_PRIVACY_DOT + SP2;
    let label = t(label);
    let label_w = text_w(ctx.painter, label, &QS_STRONG);
    c.text(ctx.painter, (left, left + label_w, top, bottom), TextAlign::Left, label, theme.label, &QS_STRONG);
    let tail = t(tail).replace("{app}", app);
    let right = rect.loc.x + rect.size.w - QS_TILE_PAD;
    c.text(ctx.painter, (left + label_w, right, top, bottom), TextAlign::Left, &tail, theme.label, &QS_SUB);
}

fn draw_qs_footer(
    c: &mut Canvas<'_>, ctx: &QsRenderContext<'_>, footer: &QsFooter, status: &crate::system_info::SystemStatus,
) {
    let theme = ctx.theme;
    if let (Some(chip), Some(pct)) = (footer.chip, status.battery_pct) {
        let charging = status.battery_charging;
        let low = !charging && pct <= theme.battery_low_pct;
        let (w, h) = QS_CHIP_BATTERY;
        let x = chip.loc.x + QS_CHIP_PAD;
        let cell = battery::Cell::new(x, chip.loc.y + (chip.size.h - h) / 2, w, h);
        let fill = if low {
            theme.danger
        } else if charging {
            theme.ok
        } else {
            theme.label
        };
        battery::draw(c, &cell, battery::Look { pct, charging, ink: theme.label, fill });
        let (top, bottom) = (chip.loc.y, chip.loc.y + chip.size.h);
        let pct_text = qs_percent(pct);
        let pct_left = x + w + SP2;
        let pct_w = text_w(ctx.painter, &pct_text, &QS_CHIP_PCT);
        let ink = if low { theme.danger } else { theme.label };
        c.text(ctx.painter, (pct_left, pct_left + pct_w, top, bottom), TextAlign::Left, &pct_text, ink, &QS_CHIP_PCT);
        let line_left = pct_left + pct_w + SP2;
        let right = chip.loc.x + chip.size.w - QS_CHIP_PAD;
        c.text(
            ctx.painter,
            (line_left, right, top, bottom),
            TextAlign::Left,
            &qs_chip_line(status),
            theme.muted,
            &QS_SUB,
        );
    }
    let buttons: [(LocalRect, &str, GlyphPainter); 4] = [
        (footer.screenshot, "accessories-screenshot", crate::paint::draw_camera_glyph_over),
        (footer.settings, "emblem-system", crate::paint::draw_gear_glyph_over),
        (footer.lock, "system-lock-screen", crate::paint::draw_lock_glyph_over),
        (footer.power, "system-shutdown", crate::paint::draw_power_glyph_over),
    ];
    for (rect, icon, glyph) in buttons {
        qs_symbol(c, rect, (icon, glyph), QS_ICON, theme.muted);
    }
}

fn draw_qs_mic_glyph(c: &mut Canvas<'_>, rect: LocalRect, (muted, colour): (bool, Color)) {
    let side = 15;
    let at = (rect.loc.x + (rect.size.w - side) / 2, rect.loc.y + (rect.size.h - side) / 2);
    let name = if muted { "microphone-sensitivity-muted" } else { "audio-input-microphone" };
    if c.icon(name, side, at, colour) {
        return;
    }
    let (x, y) = at;
    let centre = x + side / 2;
    c.stroke(local_rect(centre - 3, y + 1, 7, 9), 1.5, R_ROUND, colour);
    c.fill(local_rect(centre, y + 11, 1, 2), colour, 0.0);
    c.fill(local_rect(centre - 3, y + 13, 7, 1), colour, 0.0);
    if muted {
        c.fill(local_rect(x + 1, y + side / 2, side - 2, 1), colour, 0.0);
    }
}

fn draw_quick_settings(c: &mut Canvas<'_>, ctx: &QsRenderContext<'_>, draw: &QsDraw<'_>) {
    let geometry = &draw.geometry;
    if let Some(strip) = geometry.strip {
        draw_qs_strip(c, ctx, strip, draw.status);
    }
    draw_qs_sliders(c, ctx, draw);
    for y in geometry.dividers {
        c.fill(local_rect(QS_PAD, y, QS_WIDTH - QS_PAD * 2, 1), ctx.theme.line, 0.0);
    }
    draw_qs_tiles(c, ctx, draw);
    draw_qs_footer(c, ctx, &geometry.footer, draw.status);
}

const fn scale_or_one(scale: i32) -> i32 {
    if scale < 1 { 1 } else { scale }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::{buffer_from, canvas};

    #[test]
    fn bluetooth_tile_requires_an_adapter() {
        let mut shape = QsShape::default();
        assert!(!qs_tiles(shape).any(|tile| tile == QsTile::Bluetooth));
        shape.bluetooth = true;
        assert!(qs_tiles(shape).any(|tile| tile == QsTile::Bluetooth));
    }

    fn output() -> Rectangle<i32, Logical> {
        Rectangle::new(Point::from((0, 0)), Size::from((1920, 1080)))
    }

    fn painter() -> crate::paint::Painter {
        crate::paint::Painter::new(14.0, None)
    }

    fn topbar_with(layouts: &[&str]) -> Topbar {
        let mut bar = Topbar::new(TopbarTheme::default());
        bar.set_layouts(layouts.iter().map(|s| (*s).to_string()).collect(), 0);
        bar.set_clock("Mon 21 Sep".to_string(), "12:34".to_string());
        bar.set_app("org.gnome.Nautilus".to_string(), "Nautilus".to_string());
        bar.layout_slots(&painter(), output());
        bar
    }

    mod global_menu_bar {
        use super::*;
        use glide_appmenu::{Activation, MenuItem};

        fn leaf(label: &str, id: i32) -> MenuItem {
            MenuItem::action(label, Activation::DbusMenu(id))
        }

        fn sample() -> Vec<MenuItem> {
            vec![
                MenuItem::submenu(
                    "File",
                    vec![
                        leaf("New", 1),
                        MenuItem::separator(),
                        MenuItem::submenu("Recent", vec![leaf("a.txt", 2), leaf("b.txt", 3)]),
                        MenuItem { enabled: false, ..leaf("Print", 4) },
                    ],
                ),
                MenuItem::submenu("Edit", vec![leaf("Undo", 5)]),
            ]
        }

        fn centre(rect: Rectangle<i32, Logical>) -> Point<f64, Logical> {
            Point::from((f64::from(rect.loc.x + rect.size.w / 2), f64::from(rect.loc.y + rect.size.h / 2)))
        }

        fn bar() -> Topbar {
            let mut bar = topbar_with(&["US", "UA"]);
            assert!(bar.set_app_menu(sample()));
            bar.layout_slots(&painter(), output());
            bar
        }

        fn relayout(bar: &mut Topbar) {
            bar.layout_slots(&painter(), output());
        }

        fn row(path: &[usize]) -> Slot {
            let mut menu = MenuPath::top(path[0]);
            for index in &path[1..] {
                menu = menu.child(*index).expect("within depth");
            }
            Slot::MenuRow(menu)
        }

        #[test]
        fn titles_sit_between_the_app_chip_and_the_clock() {
            let bar = bar();
            let file = bar.slot_rect(Slot::MenuTitle(0)).expect("File title");
            let edit = bar.slot_rect(Slot::MenuTitle(1)).expect("Edit title");
            let clock = bar.slot_rect(Slot::Clock).expect("clock");
            assert!(file.loc.x > PAD_X, "the app chip keeps its place");
            assert!(edit.loc.x >= file.loc.x + file.size.w);
            assert!(edit.loc.x + edit.size.w < clock.loc.x);
        }

        #[test]
        fn titles_that_would_reach_the_clock_are_dropped() {
            let mut bar = topbar_with(&["US", "UA"]);
            let long: Vec<MenuItem> =
                (0..40).map(|i| MenuItem::submenu(&format!("Menu number {i}"), vec![leaf("x", i)])).collect();
            bar.set_app_menu(long);
            relayout(&mut bar);
            let clock = bar.slot_rect(Slot::Clock).expect("clock");
            let rightmost = bar
                .slots
                .iter()
                .filter(|(slot, _)| matches!(slot, Slot::MenuTitle(_)))
                .map(|(_, rect)| rect.loc.x + rect.size.w)
                .max()
                .expect("some titles fit");
            assert!(rightmost < clock.loc.x);
        }

        #[test]
        fn opening_a_title_lays_out_only_enabled_rows() {
            let mut bar = bar();
            assert!(bar.open_app_menu(0));
            relayout(&mut bar);
            assert_eq!(bar.open_popup(), Some(OpenPopup::AppMenu));
            assert!(bar.slot_rect(row(&[0, 0])).is_some());
            assert!(bar.slot_rect(row(&[0, 1])).is_none(), "separators are not targets");
            assert!(bar.slot_rect(row(&[0, 3])).is_none(), "disabled rows are not targets");
            let panel = bar.popup_rect.expect("panel");
            let title = bar.slot_rect(Slot::MenuTitle(0)).expect("title");
            assert!(panel.loc.y >= title.loc.y + title.size.h);
        }

        #[test]
        fn clicking_the_open_title_again_closes_it() {
            let mut bar = bar();
            bar.open_app_menu(0);
            assert!(bar.open_app_menu(0));
            assert!(!bar.has_open_popup());
        }

        #[test]
        fn hovering_another_title_switches_menus_while_one_is_open() {
            let mut bar = bar();
            bar.open_app_menu(0);
            relayout(&mut bar);
            let edit = bar.slot_rect(Slot::MenuTitle(1)).expect("Edit title");
            bar.set_hover(Some(centre(edit)));
            relayout(&mut bar);
            assert!(bar.slot_rect(row(&[1, 0])).is_some(), "Edit's rows are live");
            assert!(bar.slot_rect(row(&[0, 0])).is_none());
        }

        #[test]
        fn hovering_a_submenu_row_cascades_and_a_leaf_collapses_it() {
            let mut bar = bar();
            bar.open_app_menu(0);
            relayout(&mut bar);
            let recent = bar.slot_rect(row(&[0, 2])).expect("Recent row");
            bar.set_hover(Some(centre(recent)));
            relayout(&mut bar);
            assert_eq!(bar.menu_panels.len(), 2);
            let cascade = bar.slot_rect(row(&[0, 2, 1])).expect("b.txt row");
            assert!(cascade.loc.x > recent.loc.x);
            assert!(bar.contains(output(), centre(cascade)), "a click on the cascade must not count as outside");

            let new = bar.slot_rect(row(&[0, 0])).expect("New row");
            bar.set_hover(Some(centre(new)));
            relayout(&mut bar);
            assert_eq!(bar.menu_panels.len(), 1);
        }

        #[test]
        fn activating_a_leaf_closes_the_menu_and_reports_it() {
            let mut bar = bar();
            bar.open_app_menu(0);
            relayout(&mut bar);
            let MenuRowOutcome::Activate(activation) = bar.app_menu_row(MenuPath::top(0).child(0).expect("depth"))
            else {
                panic!("New is a plain item");
            };
            assert_eq!(activation, Activation::DbusMenu(1));
            assert!(!bar.has_open_popup());
        }

        #[test]
        fn disabled_and_submenu_rows_do_not_activate() {
            let mut bar = bar();
            bar.open_app_menu(0);
            let print = MenuPath::top(0).child(3).expect("depth");
            assert_eq!(bar.app_menu_row(print), MenuRowOutcome::Ignored);
            let recent = MenuPath::top(0).child(2).expect("depth");
            assert_eq!(bar.app_menu_row(recent), MenuRowOutcome::Submenu);
            assert!(bar.has_open_popup());
        }

        #[test]
        fn keyboard_walks_rows_cascades_and_neighbouring_menus() {
            let mut bar = bar();
            bar.open_app_menu(0);
            relayout(&mut bar);
            assert!(bar.focus_nav(0, 1));
            assert_eq!(bar.focused, Some(row(&[0, 0])));
            assert!(bar.focus_nav(0, 1));
            assert_eq!(bar.focused, Some(row(&[0, 2])), "skips the separator");
            assert!(bar.focus_nav(0, 1));
            assert_eq!(bar.focused, Some(row(&[0, 0])), "wraps past the disabled row");

            bar.set_focused(Some(row(&[0, 2])));
            assert!(bar.focus_nav(1, 0));
            relayout(&mut bar);
            assert_eq!(bar.focused, Some(row(&[0, 2, 0])));
            assert!(bar.focus_nav(-1, 0));
            relayout(&mut bar);
            assert_eq!(bar.focused, Some(row(&[0, 2])));

            bar.set_focused(Some(row(&[0, 0])));
            assert!(bar.focus_nav(1, 0));
            relayout(&mut bar);
            assert!(bar.slot_rect(row(&[1, 0])).is_some(), "Right on a leaf moves to Edit");
        }

        #[test]
        fn losing_the_menu_closes_an_open_dropdown() {
            let mut bar = bar();
            bar.open_app_menu(0);
            assert!(bar.set_app_menu(Vec::new()));
            assert!(!bar.has_open_popup());
            relayout(&mut bar);
            assert!(bar.slot_rect(Slot::MenuTitle(0)).is_none());
        }

        #[test]
        fn the_desktop_menu_is_a_valid_menu_bar() {
            let mut bar = topbar_with(&["US"]);
            bar.set_app(String::new(), String::new());
            bar.set_app_menu(crate::global_menu::desktop_menu(false, false));
            relayout(&mut bar);
            let first = bar.slot_rect(Slot::MenuTitle(0)).expect("Glide menu");
            assert_eq!(first.loc.x, PAD_X, "with no app the menus start at the edge");
            assert!(bar.desktop_menu_logo(0));
            assert!(!bar.desktop_menu_logo(1));
        }

        #[test]
        fn an_active_application_keeps_its_glide_menu_title() {
            let mut bar = topbar_with(&["US"]);
            bar.set_app_menu(crate::global_menu::desktop_menu(false, false));
            assert!(!bar.desktop_menu_logo(0));
            bar.set_app(String::new(), String::new());
            assert!(bar.desktop_menu_logo(0));
            bar.set_app("org.glide.Test".to_string(), "Glide".to_string());
            assert!(!bar.desktop_menu_logo(0));
        }
    }

    fn playing() -> MediaWidget {
        MediaWidget {
            title: "Пісня про рушник".to_string(),
            artist: "Хор".to_string(),
            album: "Золоті шедеври".to_string(),
            length: Some(Duration::from_secs(212)),
            source: "Rhythmbox".to_string(),
            art: None,
            art_large: None,
            app_icons: Vec::new(),
            accent: Color([0.9, 0.63, 0.29, 1.0]),
            playing: true,
            can_next: true,
            can_previous: true,
            progress: Some(0.38),
        }
    }

    fn centre(rect: Rectangle<i32, Logical>) -> Point<f64, Logical> {
        Point::from((f64::from(rect.loc.x + rect.size.w / 2), f64::from(rect.loc.y + rect.size.h / 2)))
    }

    fn click(bar: &Topbar, slot: Slot) -> Option<TopbarAction> {
        bar.action_at(centre(bar.slot_rect(slot).expect("slot")))
    }

    fn with_popup(mut bar: Topbar, popup: OpenPopup) -> Topbar {
        bar.toggle_popup(popup);
        bar.layout_slots(&painter(), output());
        bar
    }

    fn playing_bar() -> Topbar {
        let mut bar = topbar_with(&["US", "UA"]);
        bar.set_media(Some(playing()));
        bar.layout_slots(&painter(), output());
        bar
    }

    fn inside(outer: Rectangle<i32, Logical>, rect: Rectangle<i32, Logical>) -> bool {
        rect.loc.x >= outer.loc.x
            && rect.loc.y >= outer.loc.y
            && rect.loc.x + rect.size.w <= outer.loc.x + outer.size.w
            && rect.loc.y + rect.size.h <= outer.loc.y + outer.size.h
    }

    #[test]
    fn the_island_sits_in_the_middle_of_the_bar_with_the_player_left_of_the_clock() {
        let silent = topbar_with(&["US", "UA"]);
        assert!(silent.slot_rect(Slot::IslandMedia).is_none());
        let clock = silent.slot_rect(Slot::Clock).expect("the clock is always there");
        let island = silent.island_target(output());
        assert_eq!(silent.island.rect(silent.rect(output())), island, "the first layout is taken without a morph");
        assert!(inside(island, clock));
        assert_eq!(island.loc.y, island::TOP);
        assert_eq!(island.loc.x + island.size.w / 2, output().size.w / 2, "the island is centred");

        let bar = playing_bar();
        let island = bar.island_target(output());
        let media = bar.slot_rect(Slot::IslandMedia).expect("a player button while something plays");
        let clock = bar.slot_rect(Slot::Clock).expect("clock");
        assert!(inside(island, media) && inside(island, clock));
        assert!(media.loc.x + media.size.w < clock.loc.x);
        assert!(island.size.w > silent.island_target(output()).size.w);
        assert_eq!(click(&bar, Slot::IslandMedia), Some(TopbarAction::ToggleMediaPlayer));
        assert_eq!(click(&bar, Slot::Clock), Some(TopbarAction::ToggleNotificationCenter));
    }

    #[test]
    fn the_player_needs_a_track_and_seeks_where_the_pointer_lands() {
        let mut silent = topbar_with(&["US", "UA"]);
        assert!(!silent.toggle_popup(OpenPopup::MediaPlayer), "silence has no player to show");
        silent.layout_slots(&painter(), output());
        assert!(silent.popup_rect().is_none());

        let bar = with_popup(playing_bar(), OpenPopup::MediaPlayer);
        let popup = bar.popup_rect().expect("popup rect");
        assert_eq!((popup.loc.y, popup.size.w), (island::TOP, island::WIDE), "the island grows in place");
        assert_eq!(popup.loc.x + popup.size.w / 2, output().size.w / 2);
        for slot in [
            Slot::PopMediaPrev,
            Slot::PopMediaPlay,
            Slot::PopMediaNext,
            Slot::PopMediaSeek,
            Slot::IslandToNotifications,
            Slot::IslandCollapse,
        ] {
            assert!(inside(popup, bar.slot_rect(slot).expect("popup slot")), "{slot:?}");
        }
        assert!(bar.slot_rect(Slot::Clock).is_none(), "the compact controls give way to the player");
        let track = bar.slot_rect(Slot::PopMediaSeek).expect("seek slot");
        let y = centre(track).y;
        let middle = f64::from(track.loc.x) + f64::from(track.size.w) / 2.0;
        assert_eq!(bar.action_at(Point::from((middle, y))), Some(TopbarAction::MediaSeek(50)));
        let start = Point::from((f64::from(track.loc.x), y));
        assert_eq!(bar.action_at(start), Some(TopbarAction::MediaSeek(0)));
        assert_eq!(click(&bar, Slot::IslandCollapse), Some(TopbarAction::ClosePopup));
        assert_eq!(click(&bar, Slot::IslandToNotifications), Some(TopbarAction::ToggleNotificationCenter));
    }

    #[test]
    fn the_island_switches_between_its_views_without_closing() {
        let mut bar = with_popup(playing_bar(), OpenPopup::MediaPlayer);
        assert!(bar.toggle_popup(OpenPopup::NotificationCenter));
        bar.layout_slots(&painter(), output());
        assert!(bar.has_open_popup());
        assert!(bar.slot_rect(Slot::IslandToMedia).is_some(), "with a player the calendar offers it");
        assert!(bar.slot_rect(Slot::PopMediaPlay).is_none());
        assert!(bar.set_media(None));
        bar.layout_slots(&painter(), output());
        assert!(bar.slot_rect(Slot::IslandToMedia).is_none());

        let mut player = with_popup(playing_bar(), OpenPopup::MediaPlayer);
        assert!(player.set_media(None));
        assert!(!player.has_open_popup(), "the player closes when the track goes away");
    }

    #[test]
    fn an_interactive_notification_takes_the_island_and_each_part_answers_it() {
        use crate::notifications::{Notice, NoticeButton, NoticeReply, NotificationAction as Act};
        let mut bar = topbar_with(&["US", "UA"]);
        let notice = Notice {
            id: 9,
            app: "Glide".into(),
            origin_device: String::new(),
            title: "Pair with Buds?".into(),
            body: "Check that both show 123456".into(),
            icon: None,
            buttons: vec![
                NoticeButton::Reply("Reply".into()),
                NoticeButton::Action(0, "Pair".into()),
                NoticeButton::Settings("Settings".into()),
            ],
            reply: None,
            urgent: false,
            asks: true,
            waiting: 0,
        };
        assert!(bar.set_notice(Some(notice.clone())) && !bar.set_notice(Some(notice.clone())));
        let alert = IslandAlert {
            id: 1,
            heading: "Telegram".into(),
            origin_device: String::new(),
            summary: "hi".into(),
            icon: None,
        };
        bar.show_alert(alert, std::time::Instant::now());
        bar.layout_slots(&painter(), output());
        assert!(bar.slot_rect(Slot::IslandAlert).is_none(), "a question outranks a glance");
        let expect = |bar: &Topbar, slot, action| {
            assert_eq!(click(bar, slot), Some(TopbarAction::Notification(action)));
        };
        expect(&bar, Slot::NoticeButton(0), Act::OpenReply(9));
        expect(&bar, Slot::NoticeButton(1), Act::Button(9, 0));
        expect(&bar, Slot::NoticeButton(2), Act::Configure(9));
        expect(&bar, Slot::NoticeClose, Act::Dismiss(9));
        expect(&bar, Slot::Notice, Act::Activate(9));

        let pos = centre(bar.slot_rect(Slot::NoticeClose).expect("close"));
        assert!(bar.set_hover(Some(pos)));
        assert_eq!(bar.held_notice(), Some(9), "hovering any part holds its timeout");

        let composing = NoticeReply::Composing { shown: String::new(), placeholder: "…".into(), invalid: false };
        assert!(bar.set_notice(Some(Notice { reply: Some(composing), ..notice })));
        bar.layout_slots(&painter(), output());
        assert!(bar.slot_rect(Slot::NoticeButton(0)).is_none());
        expect(&bar, Slot::NoticeSend, Act::SendReply(9));
        expect(&bar, Slot::NoticeField, Act::OpenReply(9));

        assert!(bar.set_notice(None));
        bar.layout_slots(&painter(), output());
        assert!(bar.slot_rect(Slot::IslandAlert).is_some(), "the glance comes back once the question is gone");
    }

    #[test]
    fn over_fullscreen_only_a_pill_floats_and_its_click_fits_what_it_announces() {
        use crate::notifications::{Notice, NoticeButton, NotificationAction as Act};
        let mut bar = topbar_with(&["US", "UA"]);
        assert!(bar.set_fullscreen(true));
        bar.layout_slots(&painter(), output());
        assert!(!bar.floats(), "nothing to announce, nothing drawn over the window");
        assert!(!bar.contains(output(), Point::from((100.0, 4.0))));

        let alert = IslandAlert {
            id: 4,
            heading: "Telegram".into(),
            origin_device: String::new(),
            summary: "hi".into(),
            icon: None,
        };
        bar.show_alert(alert, std::time::Instant::now());
        bar.layout_slots(&painter(), output());
        assert!(bar.floats());
        assert!(bar.slot_rect(Slot::Clock).is_none() && bar.slot_rect(Slot::Status).is_none(), "the bar stays away");
        assert_eq!(click(&bar, Slot::Pill), Some(TopbarAction::ActivateNotification(4)), "one glance opens itself");

        let question = Notice {
            id: 9,
            app: "Glide".into(),
            origin_device: String::new(),
            title: "Allow?".into(),
            body: String::new(),
            icon: None,
            buttons: vec![NoticeButton::Action(0, "Allow".into())],
            reply: None,
            urgent: false,
            asks: true,
            waiting: 0,
        };
        assert!(bar.set_notice(Some(question.clone())));
        bar.layout_slots(&painter(), output());
        assert_eq!(click(&bar, Slot::Pill), Some(TopbarAction::PeekNotice), "a question is shown, not answered");
        assert!(bar.peek_notice());
        bar.layout_slots(&painter(), output());
        assert!(bar.slot_rect(Slot::Pill).is_none());
        assert_eq!(click(&bar, Slot::NoticeButton(0)), Some(TopbarAction::Notification(Act::Button(9, 0))));

        let plain = Notice { id: 10, buttons: Vec::new(), asks: false, ..question };
        assert!(bar.set_notice(Some(plain.clone())));
        bar.layout_slots(&painter(), output());
        assert_eq!(click(&bar, Slot::Pill), Some(TopbarAction::Notification(Act::Activate(10))), "peek ends with it");
        assert!(bar.set_notice(Some(Notice { waiting: 2, ..plain })));
        bar.layout_slots(&painter(), output());
        assert_eq!(click(&bar, Slot::Pill), Some(TopbarAction::PeekNotice), "several are walked through one by one");

        assert!(bar.set_fullscreen(false));
        bar.layout_slots(&painter(), output());
        assert!(bar.slot_rect(Slot::Notice).is_some() && bar.slot_rect(Slot::Clock).is_none());
        assert!(bar.slot_rect(Slot::Status).is_some(), "the bar is back");
    }

    #[test]
    fn a_new_notification_flashes_in_the_island_and_a_click_runs_its_own_action() {
        let mut bar = topbar_with(&["US", "UA"]);
        let start = std::time::Instant::now();
        let alert = IslandAlert {
            id: 1,
            heading: "Telegram".into(),
            origin_device: String::new(),
            summary: "Olena: hi".into(),
            icon: None,
        };
        assert!(bar.show_alert(alert, start));
        bar.layout_slots(&painter(), output());
        let close = bar.slot_rect(Slot::IslandAlertClose).expect("close button");
        let below = Point::from((centre(close).x, f64::from(HEIGHT) + 6.0));
        assert!(bar.contains(output(), below), "the alert is taller than the bar and still takes clicks");
        assert_eq!(click(&bar, Slot::IslandAlertClose), Some(TopbarAction::DismissIslandAlert));
        assert_eq!(click(&bar, Slot::IslandAlert), Some(TopbarAction::ActivateNotification(1)));
        assert!(bar.slot_rect(Slot::Clock).is_none());
        assert!(!bar.tick_island(start + island::ALERT_HOLD / 2));
        assert!(bar.tick_island(start + island::ALERT_HOLD));
        assert!(bar.alert().is_none());
        bar.layout_slots(&painter(), output());
        assert!(bar.slot_rect(Slot::Clock).is_some());

        let alert = IslandAlert {
            id: 2,
            heading: "Mail".into(),
            origin_device: String::new(),
            summary: String::new(),
            icon: None,
        };
        bar.show_alert(alert, start);
        assert!(bar.toggle_popup(OpenPopup::NotificationCenter));
        assert!(bar.alert().is_none(), "opening the centre answers the alert");
    }

    #[test]
    fn setters_only_dirty_the_bar_when_something_changes() {
        let mut bar = topbar_with(&["US", "UA"]);
        assert!(!bar.set_media_progress(Some(0.5)), "nothing is playing");
        assert!(bar.set_media(Some(playing())));
        assert!(!bar.set_media(Some(playing())));
        assert!(bar.set_media_progress(Some(0.5)));
        assert!(!bar.set_media_progress(Some(0.5)));
        assert!(bar.set_media_progress(None));
        let paused = MediaWidget { playing: false, ..playing() };
        assert!(bar.set_media(Some(paused)));
        assert!(bar.set_media(None) && !bar.set_media(None));

        assert!(!bar.set_clock("Mon 21 Sep".to_string(), "12:34".to_string()));
        assert!(bar.set_clock("Mon 21 Sep".to_string(), "12:35".to_string()));
        assert!(bar.set_clock("Tue 22 Sep".to_string(), "12:35".to_string()));
        let two = || vec!["US".to_string(), "UA".to_string()];
        assert!(!bar.set_layouts(two(), 0));
        assert!(bar.set_layouts(two(), 1));
        assert!(bar.set_layouts(vec!["US".to_string()], 0));
        assert!(!bar.set_app("org.gnome.Nautilus".to_string(), "Nautilus".to_string()));
        assert!(bar.set_app("kitty".to_string(), "Kitty".to_string()));
        assert!(!bar.set_dnd(false) && bar.set_dnd(true) && !bar.set_dnd(true) && bar.set_dnd(false));
        assert!(!bar.set_unread(0) && bar.set_unread(1) && !bar.set_unread(1) && bar.set_unread(0));
    }

    #[test]
    fn every_control_sits_inside_the_strip_and_reports_its_own_action() {
        let bar = topbar_with(&["US", "UA"]);
        let strip = bar.rect(output());
        assert_eq!(strip, Rectangle::new(output().loc, Size::from((1920, HEIGHT))));
        for (slot, rect) in &bar.slots {
            assert!(rect.loc.y >= strip.loc.y && rect.loc.y + rect.size.h <= HEIGHT);
            assert!(rect.loc.x + rect.size.w <= strip.size.w - PAD_X + 1);
            assert_eq!(bar.action_at(centre(*rect)), Some(slot.action()));
        }
        assert_eq!(bar.action_at(Point::from((600.0, 17.0))), None, "empty space");
        assert!(!bar.contains(output(), Point::from((100.0, f64::from(HEIGHT) + 1.0))));
        assert!(bar.contains(output(), Point::from((100.0, 4.0))));

        let painter = painter();
        let chip = PAD_X + app_chip_width(&painter, &bar.app_name, true);
        assert!(chip < bar.island_target(output()).loc.x, "the app chip never reaches the island");
    }

    #[test]
    fn layouts_are_a_segmented_control_a_dropdown_or_a_single_cycle_button() {
        let two = topbar_with(&["US", "UA"]);
        assert!(two.segmented());
        assert!(two.slot_rect(Slot::LayoutCycle).is_none());
        let control = two.layout_control_rect().expect("control rect");
        let seg = |index| two.slot_rect(Slot::LayoutPick(index)).expect("segment");
        assert!(inside(control, seg(0)) && inside(control, seg(1)));
        assert!(seg(0).loc.x + seg(0).size.w <= seg(1).loc.x);
        assert_eq!(click(&two, Slot::LayoutPick(1)), Some(TopbarAction::SelectLayout(1)));
        let coffee = two.slot_rect(Slot::Caffeine).expect("caffeine slot");
        assert_eq!(control.loc.x + control.size.w + ITEM_GAP, coffee.loc.x);

        let three = topbar_with(&["US", "UA", "PL"]);
        assert!(!three.segmented());
        assert!(three.slot_rect(Slot::LayoutDropdown).is_some());
        assert!(three.slot_rect(Slot::LayoutPick(0)).is_none());

        let one = topbar_with(&["US"]);
        let coffee = one.slot_rect(Slot::Caffeine).expect("caffeine slot");
        let cycle = one.slot_rect(Slot::LayoutCycle).expect("layout slot");
        assert!(cycle.loc.x + cycle.size.w <= coffee.loc.x);

        let mut none = Topbar::new(TopbarTheme::default());
        none.layout_slots(&painter(), output());
        assert!(none.slot_rect(Slot::LayoutCycle).is_none());
        assert!(none.slot_rect(Slot::LayoutPick(0)).is_none());
        assert!(none.slot_rect(Slot::Caffeine).is_some());

        let painter = painter();
        let (narrow, wide) = (segment_width(&painter, "US"), segment_width(&painter, "UKR"));
        assert!(wide >= narrow && narrow >= SEG_MIN_WIDTH);
        assert!(Topbar::caffeine_width() > SWITCH_WIDTH);
        assert!(Topbar::caffeine_width() >= BUTTON_MIN_WIDTH);
    }

    #[test]
    fn the_language_menu_and_notification_centre_open_report_and_close() {
        let mut bar = with_popup(topbar_with(&["US", "UA", "PL"]), OpenPopup::LanguageMenu);
        assert_eq!(bar.open_popup(), Some(OpenPopup::LanguageMenu));
        for index in 0..3 {
            assert!(bar.slot_rect(Slot::LangMenuItem(index)).is_some());
        }
        assert_eq!(click(&bar, Slot::LangMenuSettings), Some(TopbarAction::OpenInputSettings));
        let settings = centre(bar.slot_rect(Slot::LangMenuSettings).expect("settings"));
        assert!(bar.contains(output(), settings));
        assert!(bar.close_popup());
        bar.layout_slots(&painter(), output());
        assert!(!bar.has_open_popup());
        assert!(bar.slot_rect(Slot::LangMenuItem(0)).is_none());

        let mut bar = topbar_with(&["US", "UA"]);
        bar.set_nc_items(vec![crate::notifications::NcItem {
            id: 42,
            app_name: "Telegram".to_string(),
            origin_device: String::new(),
            title: "Hello".to_string(),
            body: "World".to_string(),
            actions: Vec::new(),
            time: "2 хв".to_string(),
            icon: None,
            count: 1,
        }]);
        let bar = with_popup(bar, OpenPopup::NotificationCenter);
        assert_eq!(bar.open_popup(), Some(OpenPopup::NotificationCenter));
        assert!(bar.slot_rect(Slot::NcClearAll).is_some());
        assert!(bar.slot_rect(Slot::NcDndToggle).is_some());
        assert_eq!(click(&bar, Slot::NcItem(42)), Some(TopbarAction::ActivateNotification(42)));
        assert_eq!(click(&bar, Slot::NcItemDismiss(42)), Some(TopbarAction::DismissNotification(42)));
    }

    #[test]
    fn hover_is_paint_state_that_changes_only_between_controls() {
        let mut bar = topbar_with(&["US", "UA"]);
        let before = bar.layout_key;
        let coffee = centre(bar.slot_rect(Slot::Caffeine).expect("caffeine slot"));
        assert!(bar.set_hover(Some(coffee)));
        assert!(!bar.set_hover(Some(coffee)), "same control, no repaint");
        assert_eq!(bar.hovered_in(Slot::in_layout), None);
        assert_eq!(bar.hovered_in(Slot::in_media_popup), None);
        assert_eq!(bar.hovered_in(Slot::in_quick_settings), None);
        assert_eq!(bar.hovered_in(Slot::in_strip), Some(Slot::Caffeine));
        bar.layout_slots(&painter(), output());
        assert_eq!(bar.layout_key, before, "hover is paint state, not geometry");
        assert!(bar.set_hover(None) && !bar.set_hover(None));
        assert!(bar.set_unread(4));
        bar.layout_slots(&painter(), output());
        assert_ne!(bar.layout_key, before, "a badge changes the clock width");
    }

    fn second_output() -> Rectangle<i32, Logical> {
        Rectangle::new(Point::from((1920, 0)), Size::from((2560, 1440)))
    }

    fn passive_slots(bar: &mut Topbar, output: Rectangle<i32, Logical>) -> Vec<(Slot, Rectangle<i32, Logical>)> {
        bar.with_passive(output, |bar| {
            bar.layout_passive(&painter(), output);
            bar.slots.clone()
        })
    }

    #[test]
    fn a_popup_stays_on_its_home_output_while_the_other_bar_is_laid_out() {
        let mut bar = topbar_with(&["US", "UA"]);
        bar.reset_home(output());
        let mut bar = with_popup(bar, OpenPopup::QuickSettings);
        let popup = bar.popup_rect.expect("quick settings is laid out");
        let home_status = bar.slot_rect(Slot::Status).expect("status slot");

        let other = passive_slots(&mut bar, second_output());
        assert!(other.iter().all(|(_, rect)| second_output().contains(rect.loc)), "{other:?}");
        assert!(other.iter().any(|(slot, _)| *slot == Slot::Status));
        assert!(other.iter().all(|(slot, _)| !Slot::in_quick_settings(*slot)));

        assert_eq!(bar.open_popup(), Some(OpenPopup::QuickSettings));
        assert_eq!(bar.popup_rect, Some(popup));
        assert_eq!(bar.slot_rect(Slot::Status), Some(home_status));
        assert!(!bar.set_home(second_output()), "an open popup holds its output");
        assert!(bar.is_home(output()) && !bar.is_home(second_output()));
    }

    #[test]
    fn the_home_moves_to_the_output_under_the_pointer_and_hit_tests_its_bar() {
        let mut bar = topbar_with(&["US", "UA"]);
        bar.reset_home(output());
        bar.layout_slots(&painter(), output());
        let other = passive_slots(&mut bar, second_output());
        let status = other.iter().find(|(slot, _)| *slot == Slot::Status).map(|(_, rect)| *rect).expect("status");

        assert!(bar.set_home(second_output()));
        let pos = centre(status);
        assert!(bar.contains(second_output(), pos));
        assert_eq!(bar.slot_at(pos), Some(Slot::Status));
        assert_eq!(bar.action_at(pos), Some(TopbarAction::ToggleQuickSettings));
        assert!(!bar.contains(output(), pos));
    }

    #[test]
    fn names_are_shortened_for_the_strip() {
        assert_eq!(short_layout_name("Ukrainian"), "UKR");
        assert_eq!(short_layout_name("English (US)"), "US");
        assert_eq!(short_layout_name("Polish (qwertz)"), "QWE");
        assert_eq!(application_name("org.gnome.Nautilus", Some("Files")), "Files");
        assert_eq!(application_name("org.gnome.Nautilus", None), "Nautilus");
        assert_eq!(application_name("kitty", None), "Kitty");
        assert_eq!(application_name("", None), "");
        let shown = application_name(&"a".repeat(80), None);
        assert!(shown.chars().count() <= APP_NAME_MAX && shown.ends_with('…'));
    }

    fn clock_box(bar: &mut Topbar) -> i32 {
        island::clock_width(bar.island_clock(&painter()))
    }

    #[test]
    fn the_clock_reserves_room_for_both_the_date_and_the_time() {
        let mut with_date = Topbar::new(TopbarTheme { clock_24h: Some(true), ..TopbarTheme::default() });
        with_date.set_now(at(21, 1, 8, 12));
        let mut without =
            Topbar::new(TopbarTheme { clock_date: false, clock_24h: Some(true), ..TopbarTheme::default() });
        without.set_now(at(21, 1, 8, 12));
        assert!(clock_box(&mut with_date) > clock_box(&mut without));
        assert_eq!(widest_digits("12:34"), "88:88");
    }

    fn at(day: u32, weekday: u32, month0: u32, hour: u32) -> calendar::LocalNow {
        calendar::LocalNow { year: 2026, month0, day, weekday, hour, minute: 5 }
    }

    #[test]
    fn the_clock_box_holds_its_width_across_days_months_and_hours() {
        let mut bar = topbar_with(&["US", "UA"]);
        let mut widths = Vec::new();
        for (day, weekday, month0) in [(24, 4, 8), (1, 3, 0), (31, 5, 4), (7, 0, 10)] {
            for hour in [0, 9, 13, 23] {
                bar.set_now(at(day, weekday, month0, hour));
                widths.push(clock_box(&mut bar));
            }
        }
        assert!(widths.windows(2).all(|pair| pair[0] == pair[1]), "{widths:?}");
    }

    #[test]
    fn turning_the_date_off_leaves_only_the_time() {
        let theme = TopbarTheme { clock_date: false, clock_24h: Some(true), ..TopbarTheme::default() };
        let mut bar = Topbar::new(theme);
        bar.set_now(at(24, 4, 8, 14));
        assert_eq!(bar.date, "");
        assert_eq!(bar.clock, "14:05");
        bar.reconfigure(TopbarTheme { clock_24h: Some(false), ..TopbarTheme::default() });
        bar.set_now(at(24, 4, 8, 15));
        assert_eq!(bar.clock, format!("3:05 {}", crate::i18n::t(crate::i18n::Key::ClockPm)));
        assert!(!bar.date.is_empty());
    }

    #[test]
    fn numbers_are_set_in_tabular_figures_without_uppercase_tracking() {
        for style in [island::TIME_STYLE, status::PCT_STYLE] {
            assert!(style.tabular);
            assert!(!style.uppercase);
            assert!(style.tracking_em.abs() < f32::EPSILON, "no letter-spacing between digits");
        }
        assert!((island::TIME_STYLE.size_px - island::DATE_STYLE.size_px).abs() < f32::EPSILON);
    }

    #[test]
    fn a_battery_reading_keeps_its_width_within_a_digit_count() {
        let painter = painter();
        let reference = numeric_label_width(&painter, "88%", &status::PCT_STYLE);
        for pct in ["11%", "47%", "90%"] {
            assert_eq!(numeric_label_width(&painter, pct, &status::PCT_STYLE), reference, "{pct}");
        }
    }

    #[test]
    fn an_unread_notification_count_expands_the_clock() {
        let mut bar = topbar_with(&["US", "UA"]);
        let without_unread = clock_box(&mut bar);
        bar.set_unread(3);
        assert!(clock_box(&mut bar) > without_unread);
    }

    #[test]
    fn local_clock_reads_as_a_wall_clock() {
        let clock = local_clock().expect("the system clock must be readable");
        assert_eq!(clock.len(), 5);
        assert_eq!(clock.as_bytes()[2], b':');
    }

    #[test]
    fn the_local_calendar_date_is_readable() {
        let now = calendar::LocalNow::now().expect("the system clock must be readable");
        assert!((1..=31).contains(&now.day));
        assert!(now.weekday < 7 && now.month0 < 12 && now.hour < 24 && now.minute < 60);
    }

    #[test]
    fn quick_settings_opens_with_correct_slots_and_can_be_closed() {
        let bar = topbar_with(&["US", "UA"]);
        assert!(bar.slot_rect(Slot::Status).is_some() && !bar.has_open_popup());
        let mut bar = with_popup(bar, OpenPopup::QuickSettings);
        assert_eq!(bar.open_popup(), Some(OpenPopup::QuickSettings));
        for slot in [
            Slot::QsBluetooth,
            Slot::QsNightLight,
            Slot::QsDnd,
            Slot::QsDarkTheme,
            Slot::QsCaffeine,
            Slot::QsVolumeMute,
            Slot::QsBrightnessTrack,
            Slot::QsSettings,
            Slot::QsWifiMore,
            Slot::QsBluetoothMore,
            Slot::QsVolumeMore,
            Slot::QsScreenshot,
            Slot::QsLock,
            Slot::QsPowerMenu,
        ] {
            assert!(bar.slot_rect(slot).is_some(), "{slot:?}");
        }
        for slot in [Slot::QsSaver, Slot::QsBattery, Slot::QsPowerButton(PowerAction::Lock)] {
            assert!(bar.slot_rect(slot).is_none(), "{slot:?} has no place on this machine's main view");
        }
        for (slot, action) in [
            (Slot::QsWifi, TopbarAction::QsToggleWifi),
            (Slot::QsWifiMore, TopbarAction::QsPanel(PanelAction::Open(Detail::Wifi))),
            (Slot::QsScreenshot, TopbarAction::QsScreenshot),
            (Slot::QsCaffeine, TopbarAction::ToggleCaffeine),
            (Slot::QsDnd, TopbarAction::ToggleDoNotDisturb),
            (Slot::QsDarkTheme, TopbarAction::QsToggleDarkTheme),
            (Slot::QsVolumeMore, TopbarAction::QsPanel(PanelAction::Open(Detail::Sound))),
            (Slot::QsSettings, TopbarAction::QsOpenSettings),
            (Slot::QsLock, TopbarAction::QsPower(PowerAction::Lock)),
            (Slot::QsPowerMenu, TopbarAction::QsPanel(PanelAction::Open(Detail::Power))),
        ] {
            assert_eq!(click(&bar, slot), Some(action), "{slot:?}");
        }
        let centre = centre(bar.slot_rect(Slot::QsWifi).expect("wifi slot"));

        let vol_slot = bar.slot_rect(Slot::QsVolumeTrack).expect("vol slot");
        let quarter = Point::from((
            f64::from(vol_slot.loc.x) + f64::from(vol_slot.size.w) * 0.25,
            f64::from(vol_slot.loc.y + vol_slot.size.h / 2),
        ));
        assert_eq!(bar.action_at(quarter), Some(TopbarAction::QsVolumeAdjust(25)));

        assert!(bar.contains(output(), centre));
        assert!(bar.close_popup());
        bar.layout_slots(&painter(), output());
        assert!(!bar.has_open_popup());
        assert!(bar.slot_rect(Slot::QsWifi).is_none());
    }

    #[test]
    fn a_control_the_machine_lacks_has_no_slider_to_drag() {
        let mut bar = topbar_with(&["US"]);
        bar.set_system_status(crate::system_info::SystemStatus {
            brightness_available: false,
            ..crate::system_info::SystemStatus::default()
        });
        let bar = with_popup(bar, OpenPopup::QuickSettings);
        assert!(bar.slot_rect(Slot::QsBrightnessTrack).is_none(), "a desktop without a backlight");
        assert!(bar.slot_rect(Slot::QsVolumeTrack).is_some());

        let mut bar = topbar_with(&["US"]);
        bar.set_system_status(crate::system_info::SystemStatus {
            volume_available: false,
            ..crate::system_info::SystemStatus::default()
        });
        let bar = with_popup(bar, OpenPopup::QuickSettings);
        for slot in [Slot::QsVolumeTrack, Slot::QsVolumeMute] {
            assert!(bar.slot_rect(slot).is_none(), "{slot:?} without wpctl or pactl");
        }
    }

    #[test]
    fn every_slider_row_shares_one_anatomy() {
        let shape = QsShape { mic: true, brightness: true, ..QsShape::default() };
        let geometry = qs_geometry(&shape);
        let rows: Vec<QsSliderRow> =
            std::iter::once(geometry.volume).chain(geometry.mic).chain(geometry.brightness).collect();
        assert_eq!(rows.len(), 3);
        for row in &rows {
            assert_eq!((row.icon.loc.x, row.icon.size.w, row.icon.size.h), (QS_PAD, QS_ICON_BTN, QS_ROW));
            assert_eq!(row.track.loc.x, row.icon.loc.x + QS_ICON_BTN + QS_ROW_GAP + QS_SLIDER_MARGIN);
            assert_eq!(row.track.loc.x + row.track.size.w + QS_SLIDER_MARGIN + QS_ROW_GAP, row.readout.loc.x);
            assert_eq!(row.readout.loc.x + QS_READOUT_W + QS_ROW_GAP + QS_DRILL, QS_WIDTH - QS_PAD, "aligned columns");
        }
        assert!(geometry.volume.drill.is_some() && rows[1].drill.is_some());
        assert!(rows[2].drill.is_none(), "brightness has no page to open");
        for pair in rows.windows(2) {
            assert_eq!(pair[1].icon.loc.y, pair[0].icon.loc.y + QS_ROW + QS_ROW_GAP);
        }
    }

    #[test]
    fn the_quick_settings_popup_is_exactly_as_tall_as_its_content() {
        let plain = QsShape { network: true, ..QsShape::default() };
        let full = QsShape {
            mic: true,
            brightness: true,
            battery: true,
            profile: true,
            vpn: true,
            bluetooth: true,
            network: true,
            strip: true,
            chip: QS_CHIP_DEFAULT,
        };
        for shape in [plain, full] {
            let geometry = qs_geometry(&shape);
            let footer = geometry.footer;
            assert_eq!(geometry.height, footer.power.loc.y + footer.power.size.h + QS_PAD, "{shape:?}");
            let rects = geometry
                .tiles
                .iter()
                .map(|(_, rect)| *rect)
                .chain([footer.screenshot, footer.settings, footer.lock, footer.power])
                .chain(footer.chip)
                .chain(geometry.strip);
            for rect in rects {
                assert!(rect.loc.x >= QS_PAD && rect.loc.x + rect.size.w <= QS_WIDTH - QS_PAD, "{rect:?}");
                assert!(rect.loc.y >= QS_PAD && rect.loc.y + rect.size.h <= geometry.height - QS_PAD, "{rect:?}");
            }
            if let Some(chip) = footer.chip {
                assert!(chip.loc.x + chip.size.w <= footer.screenshot.loc.x, "the chip stops before the buttons");
            }
        }
        assert_eq!(qs_geometry(&plain).tiles.len(), 5);
        assert_eq!(qs_geometry(&full).tiles.len(), QS_TILE_COUNT);

        let mut bar = topbar_with(&["US", "UA"]);
        bar.toggle_popup(OpenPopup::QuickSettings);
        bar.layout_slots(&painter(), output());
        let popup = bar.popup_rect().expect("popup rect");
        assert_eq!(popup.size, Size::from((QS_WIDTH, qs_geometry(&bar.qs_shape(None)).height)));
    }

    #[test]
    fn every_popup_hangs_the_same_distance_below_the_strip() {
        let mut bar = topbar_with(&["US", "UA", "PL"]);
        bar.set_media(Some(playing()));
        let mut tops = Vec::new();
        for popup in
            [OpenPopup::QuickSettings, OpenPopup::NotificationCenter, OpenPopup::LanguageMenu, OpenPopup::MediaPlayer]
        {
            bar.close_popup();
            bar.toggle_popup(popup);
            bar.layout_slots(&painter(), output());
            tops.push((popup, bar.popup_rect().expect("popup rect").loc.y));
        }
        for (popup, top) in tops {
            let want = if Topbar::island_owns(Some(popup)) { island::TOP } else { HEIGHT + POPUP_GAP };
            assert_eq!(top, want, "{popup:?}: the island grows from where it sits, the rest hang below");
        }
    }

    #[test]
    fn a_theme_built_from_light_tokens_is_not_dark() {
        let tokens = glide_theme::Tokens::new(glide_theme::Appearance::Light, glide_theme::Accent::Blue);
        let theme = TopbarTheme::from_tokens(&tokens);
        assert!(!theme.dark);
        assert_ne!(theme.revision(), TopbarTheme::default().revision());
    }

    #[test]
    fn the_status_block_grows_to_hold_the_battery_instead_of_adding_a_slot() {
        let mut bar = topbar_with(&["US", "UA"]);
        let before = bar.slot_rect(Slot::Status).expect("status block");
        let strip_slots = bar.slots.len();

        let status = crate::system_info::SystemStatus { battery_pct: Some(82), ..Default::default() };
        bar.set_system_status(status);
        bar.layout_slots(&painter(), output());
        let after = bar.slot_rect(Slot::Status).expect("status block");
        assert_eq!(bar.slots.len(), strip_slots, "no per-indicator slot appears");
        assert!(after.size.w > before.size.w);
        assert_eq!(after.loc.x + after.size.w, before.loc.x + before.size.w, "the block stays pinned to the edge");
        let search = bar.slot_rect(Slot::Search).expect("search slot");
        assert!(search.loc.x + search.size.w <= after.loc.x);
    }

    #[test]
    fn hiding_the_battery_percentage_narrows_the_status_block() {
        let status = crate::system_info::SystemStatus { battery_pct: Some(82), ..Default::default() };
        let mut with_pct = topbar_with(&["US"]);
        with_pct.set_system_status(status.clone());
        with_pct.layout_slots(&painter(), output());
        let mut bare = Topbar::new(TopbarTheme { battery_percentage: false, ..TopbarTheme::default() });
        bare.set_system_status(status);
        bare.layout_slots(&painter(), output());
        let wide = with_pct.slot_rect(Slot::Status).expect("block").size.w;
        let narrow = bare.slot_rect(Slot::Status).expect("block").size.w;
        assert!(narrow < wide);
    }

    #[test]
    fn the_right_cluster_runs_from_the_edge_and_ends_before_caffeine() {
        let mut bar = topbar_with(&["US", "UA"]);
        let status = crate::system_info::SystemStatus {
            battery_pct: Some(50),
            bluetooth_on: true,
            vpn_active: true,
            privacy: crate::system_info::Privacy { microphone: vec!["Firefox".into()], ..Default::default() },
            ..Default::default()
        };
        bar.set_system_status(status);
        bar.layout_slots(&painter(), output());
        let order = [Slot::Status, Slot::Search, Slot::Privacy, Slot::Caffeine];
        let rects: Vec<_> =
            order.iter().map(|slot| bar.slot_rect(*slot).unwrap_or_else(|| panic!("{slot:?}"))).collect();
        let right_edge = output().size.w - PAD_X;
        assert_eq!(rects[0].loc.x + rects[0].size.w, right_edge);
        for pair in rects.windows(2) {
            assert!(pair[1].loc.x + pair[1].size.w <= pair[0].loc.x, "{pair:?}");
        }
    }

    #[test]
    fn clicking_the_indicators_reports_their_actions() {
        let mut bar = topbar_with(&["US"]);
        bar.set_system_status(crate::system_info::SystemStatus {
            battery_pct: Some(50),
            bluetooth_on: true,
            ..Default::default()
        });
        bar.layout_slots(&painter(), output());
        let click = |bar: &Topbar, slot: Slot| {
            let rect = bar.slot_rect(slot).expect("slot");
            bar.action_at(Point::from((
                f64::from(rect.loc.x + rect.size.w / 2),
                f64::from(rect.loc.y + rect.size.h / 2),
            )))
        };
        assert_eq!(click(&bar, Slot::Search), Some(TopbarAction::ToggleLauncher));
        assert_eq!(click(&bar, Slot::Status), Some(TopbarAction::ToggleQuickSettings));
        let block = bar.slot_rect(Slot::Status).expect("status block");
        let y = f64::from(block.loc.y + block.size.h / 2);
        for x in [block.loc.x + 1, block.loc.x + block.size.w - 1] {
            let pos = Point::from((f64::from(x), y));
            assert_eq!(bar.slot_at(pos), Some(Slot::Status), "one hit target");
        }
        assert!(bar.set_hover(Some(Point::from((f64::from(block.loc.x + 1), y)))));
        assert!(
            !bar.set_hover(Some(Point::from((f64::from(block.loc.x + block.size.w - 1), y)))),
            "the whole block shares one hover state"
        );
    }

    #[test]
    fn the_battery_chip_sits_in_the_footer_and_opens_its_view() {
        let mut bar = topbar_with(&["US"]);
        bar.toggle_popup(OpenPopup::QuickSettings);
        bar.layout_slots(&painter(), output());
        assert_eq!(bar.slot_rect(Slot::QsBattery), None, "no battery, no chip");

        bar.set_system_status(crate::system_info::SystemStatus { battery_pct: Some(50), ..Default::default() });
        bar.layout_slots(&painter(), output());
        let chip = bar.slot_rect(Slot::QsBattery).expect("battery chip");
        let lock = bar.slot_rect(Slot::QsLock).expect("lock");
        let tile = bar.slot_rect(Slot::QsDnd).expect("tile");
        assert_eq!(chip.loc.y, lock.loc.y, "one footer row");
        assert!(tile.loc.y + tile.size.h < chip.loc.y);
        assert!(bar.popup_rect().expect("qs").contains(chip.loc));
        assert_eq!(click(&bar, Slot::QsBattery), Some(TopbarAction::QsPanel(PanelAction::Open(Detail::Battery))));

        bar.set_focused(Some(Slot::QsBattery));
        assert!(bar.open_detail(Detail::Battery));
        bar.layout_slots(&painter(), output());
        assert_eq!(bar.focused, Some(Slot::QsBack));
        assert!(bar.slot_rect(Slot::QsDetailToggle).is_none(), "no radio");
        assert!(bar.focus_nav(0, 1));
        assert_eq!(bar.focused_action(), Some(TopbarAction::QsPanel(PanelAction::Settings)));
        assert!(bar.close_detail());
        assert_eq!(bar.focused, Some(Slot::QsBattery));
    }

    #[test]
    fn privacy_dots_appear_when_capture_starts_and_leave_after_fading() {
        let mut bar = topbar_with(&["US"]);
        assert_eq!(bar.privacy_dots(), [false; 3]);
        assert!(bar.set_screen_capture(vec!["obs".into()]));
        assert_eq!(bar.privacy_dots(), [false, false, true]);
        bar.layout_slots(&painter(), output());
        assert!(bar.slot_rect(Slot::Privacy).is_some());
        assert!(bar.set_screen_capture(Vec::new()));
        assert_eq!(bar.privacy_dots(), [false, false, true], "held while fading");
        std::thread::sleep(glide_theme::motion::t2() + Duration::from_millis(20));
        bar.settle_motion();
        assert_eq!(bar.privacy_dots(), [false; 3]);
        bar.layout_slots(&painter(), output());
        assert_eq!(bar.slot_rect(Slot::Privacy), None);
    }

    #[test]
    fn the_clock_popover_places_the_calendar_beside_the_notifications() {
        let mut bar = topbar_with(&["US"]);
        bar.set_now(at(24, 4, 8, 12));
        bar.toggle_popup(OpenPopup::NotificationCenter);
        bar.layout_slots(&painter(), output());
        let popup = bar.popup_rect().expect("clock popover");
        let month = bar.slot_rect(Slot::CalMonth).expect("month title");
        let next = bar.slot_rect(Slot::CalNext).expect("next month");
        let quiet = bar.slot_rect(Slot::NcDndToggle).expect("do not disturb");
        let collapse = bar.slot_rect(Slot::IslandCollapse).expect("collapse");
        assert!(popup.contains(month.loc) && popup.contains(next.loc));
        assert!(collapse.loc.y + collapse.size.h <= month.loc.y, "the header sits above the calendar");
        assert!(month.loc.x + month.size.w < quiet.loc.x, "the calendar is left of notifications");
        assert!(bar.slot_rect(Slot::NcClearAll).is_none(), "nothing to clear");
        assert!(bar.slot_rect(Slot::IslandToMedia).is_none(), "nothing playing");
    }

    #[test]
    fn notification_card_actions_target_their_original_button() {
        let mut bar = topbar_with(&["US"]);
        bar.set_nc_items(vec![crate::notifications::NcItem {
            id: 42,
            app_name: "Mail".into(),
            origin_device: String::new(),
            title: "Message".into(),
            body: "Hello".into(),
            actions: vec!["Open".into(), "Later".into(), "Archive".into()],
            time: "now".into(),
            icon: None,
            count: 1,
        }]);
        let bar = with_popup(bar, OpenPopup::NotificationCenter);
        assert_eq!(
            click(&bar, Slot::NcItemAction(42, 1)),
            Some(TopbarAction::Notification(crate::notifications::NotificationAction::Button(42, 1)))
        );
        assert_eq!(
            click(&bar, Slot::NcItemAction(42, 2)),
            Some(TopbarAction::Notification(crate::notifications::NotificationAction::Button(42, 2)))
        );
    }

    #[test]
    fn arrows_on_the_month_title_page_months_and_enter_returns_to_today() {
        let mut bar = topbar_with(&["US"]);
        bar.set_now(at(24, 4, 8, 12));
        bar.toggle_popup(OpenPopup::NotificationCenter);
        bar.layout_slots(&painter(), output());
        let today = bar.calendar.month();
        assert!(bar.focus_nav(0, 1));
        assert_eq!(bar.focused, Some(Slot::IslandCollapse), "the header comes first");
        bar.set_focused(Some(Slot::CalMonth));
        assert_eq!(bar.focused_action(), Some(TopbarAction::CalendarToday));
        assert!(bar.focus_nav(1, 0));
        assert_eq!(bar.calendar.month(), today.shifted(1));
        assert!(bar.focus_nav(-1, 0));
        assert!(bar.focus_nav(-1, 0));
        assert_eq!(bar.calendar.month(), today.shifted(-1));
        assert!(bar.calendar_today());
        assert_eq!(bar.calendar.month(), today);
        assert!(bar.close_popup());
        bar.shift_calendar(3);
        bar.toggle_popup(OpenPopup::NotificationCenter);
        assert_eq!(bar.calendar.month(), today, "reopening starts on today");
    }

    fn quick_settings() -> Topbar {
        with_popup(topbar_with(&["US", "UA"]), OpenPopup::QuickSettings)
    }

    #[test]
    fn arrows_walk_the_quick_settings_tiles_and_enter_reports_them() {
        let mut none = topbar_with(&["US", "UA"]);
        assert!(!none.focus_nav(1, 0) && !none.has_focus());
        assert_eq!(none.focus_adjust(5), None, "no popup, no focus");

        let mut bar = quick_settings();
        assert!(!bar.focus_nav(0, 0), "a zero step never moves focus");
        assert!(!bar.has_focus());
        assert_eq!(bar.focused_action(), None);
        for (dx, dy, slot, action) in [
            (0, 1, Slot::QsVolumeMute, Some(TopbarAction::QsVolumeMute)),
            (1, 0, Slot::QsVolumeTrack, None),
            (1, 0, Slot::QsVolumeMore, Some(TopbarAction::QsPanel(PanelAction::Open(Detail::Sound)))),
            (0, 1, Slot::QsBluetoothMore, Some(TopbarAction::QsPanel(PanelAction::Open(Detail::Bluetooth)))),
            (-1, 0, Slot::QsBluetooth, Some(TopbarAction::QsToggleBluetooth)),
            (-1, 0, Slot::QsWifiMore, Some(TopbarAction::QsPanel(PanelAction::Open(Detail::Wifi)))),
            (-1, 0, Slot::QsWifi, Some(TopbarAction::QsToggleWifi)),
        ] {
            assert!(bar.focus_nav(dx, dy));
            assert_eq!(bar.focused, Some(slot));
            if let Some(action) = action {
                assert_eq!(bar.focused_action(), Some(action));
            }
        }
        assert!(!bar.focus_nav(-1, 0), "nothing lies left of the first tile");
        assert!(bar.focus_nav(0, 1));
        assert_eq!(bar.focused, Some(Slot::QsDnd));
        assert!(bar.focus_nav(1, 0));
        assert_eq!(bar.focused, Some(Slot::QsNightLight));
        assert!(!bar.focus_nav(1, 0), "the grid has two columns");
        assert!(bar.close_popup());
        assert!(!bar.has_focus() && bar.focused_action().is_none());

        let mut bar = quick_settings();
        bar.focus_nav(1, 0);
        bar.toggle_popup(OpenPopup::QuickSettings);
        assert!(!bar.has_focus(), "toggling the popup shut drops focus");
    }

    #[test]
    fn the_vpn_tile_joins_the_grid_only_when_there_is_a_vpn() {
        let mut bar = quick_settings();
        let plain = qs_geometry(&bar.qs_shape(None)).height;
        assert!(bar.slot_rect(Slot::QsNetbird).is_none() && bar.slot_rect(Slot::QsNetbirdMore).is_none());
        bar.set_netbird_installed(true);
        bar.layout_slots(&painter(), output());
        let taller = qs_geometry(&bar.qs_shape(None)).height;
        assert_eq!(taller - plain, QS_TILE_HEIGHT + QS_GRID_GAP, "a seventh tile opens a fourth row");
        assert!(bar.slot_rect(Slot::QsNetbird).is_some() && bar.slot_rect(Slot::QsNetbirdMore).is_some());
        assert_eq!(Slot::QsNetbird.action(), TopbarAction::QsToggleVpn);
        assert_eq!(Slot::QsNetbirdMore.action(), TopbarAction::QsPanel(PanelAction::Open(Detail::Vpn)));
        let (on, subtitle) = bar.qs_panel.netbird.tile();
        assert!(!on && subtitle == crate::i18n::t(crate::i18n::Key::QsOffLong), "no status yet reads as off");

        let mut bar = quick_settings();
        let vpn = crate::qs_net::VpnConnection {
            name: "Office".into(),
            kind: crate::qs_net::VpnKind::WireGuard,
            active: false,
        };
        bar.apply_net_event(crate::qs_net::NetEvent::Vpn(vec![vpn]));
        bar.layout_slots(&painter(), output());
        assert!(bar.slot_rect(Slot::QsNetbird).is_some(), "a NetworkManager VPN is enough");
    }

    #[test]
    fn arrows_reach_every_row_of_quick_settings_from_top_to_bottom() {
        let mut bar = quick_settings();
        bar.set_focused(Some(Slot::QsVolumeMore));
        let mut visited = vec![Slot::QsVolumeMore];
        while bar.focus_nav(0, 1) {
            visited.push(bar.focused.expect("focus"));
        }
        assert_eq!(
            visited,
            [Slot::QsVolumeMore, Slot::QsBluetoothMore, Slot::QsNightLight, Slot::QsCaffeine, Slot::QsSettings]
        );
        let mut sideways = Vec::new();
        while bar.focus_nav(-1, 0) {
            sideways.push(bar.focused.expect("focus"));
        }
        while bar.focus_nav(1, 0) {
            sideways.push(bar.focused.expect("focus"));
        }
        assert_eq!(sideways, [Slot::QsScreenshot, Slot::QsSettings, Slot::QsLock, Slot::QsPowerMenu]);
        assert!(bar.focus_nav(0, -1));
        assert_eq!(bar.focused, Some(Slot::QsCaffeine));
    }

    #[test]
    fn adjusting_a_focused_slider_clamps_to_the_percent_range() {
        let mut bar = quick_settings();
        bar.set_system_status(crate::system_info::SystemStatus {
            volume_pct: 98,
            brightness_pct: 3,
            ..Default::default()
        });

        bar.focused = Some(Slot::QsVolumeTrack);
        assert_eq!(bar.focus_adjust(5), Some(TopbarAction::QsVolumeAdjust(100)));
        assert_eq!(bar.focus_adjust(5), Some(TopbarAction::QsVolumeAdjust(100)));
        assert_eq!(bar.focus_adjust(-5), Some(TopbarAction::QsVolumeAdjust(95)));

        bar.focused = Some(Slot::QsBrightnessTrack);
        assert_eq!(bar.focus_adjust(-5), Some(TopbarAction::QsBrightnessAdjust(0)));
        assert_eq!(bar.focus_adjust(5), Some(TopbarAction::QsBrightnessAdjust(5)));

        bar.focused = Some(Slot::QsWifi);
        assert_eq!(bar.focus_adjust(5), None, "tiles are not sliders");
    }

    #[test]
    fn a_slider_adjustment_repaints_quick_settings() {
        let mut bar = quick_settings();
        let before = bar.status_rev;
        bar.focused = Some(Slot::QsVolumeTrack);
        bar.focus_adjust(5);
        assert_ne!(bar.status_rev, before);
        assert_eq!(bar.system_status().volume_pct, 55);
    }

    #[test]
    fn arrows_walk_the_language_menu_top_to_bottom() {
        let mut bar = with_popup(topbar_with(&["US", "UA", "PL"]), OpenPopup::LanguageMenu);
        bar.focus_nav(0, 1);
        assert_eq!(bar.focused, Some(Slot::LangMenuItem(0)));
        bar.focus_nav(0, 1);
        bar.focus_nav(0, 1);
        assert_eq!(bar.focused, Some(Slot::LangMenuItem(2)));
        bar.focus_nav(0, 1);
        assert_eq!(bar.focused, Some(Slot::LangMenuSettings));
        assert!(!bar.focus_nav(0, 1));
        assert_eq!(bar.focused_action(), Some(TopbarAction::OpenInputSettings));
    }

    #[test]
    fn arrows_stay_in_their_lane_across_the_notification_center() {
        let mut bar = topbar_with(&["US", "UA"]);
        let item = |id: u32| crate::notifications::NcItem {
            id,
            app_name: "Telegram".to_string(),
            origin_device: String::new(),
            title: "Hello".to_string(),
            body: "World".to_string(),
            actions: Vec::new(),
            time: "2 хв".to_string(),
            icon: None,
            count: 1,
        };
        bar.set_nc_items(vec![item(1), item(2)]);
        let mut bar = with_popup(bar, OpenPopup::NotificationCenter);
        bar.focus_nav(0, 1);
        assert_eq!(bar.focused, Some(Slot::IslandCollapse), "the header heads the island");
        assert!(bar.focus_nav(0, 1));
        assert_eq!(bar.focused, Some(Slot::CalNext), "Down leaves the header for the calendar");
        bar.set_focused(Some(Slot::NcClearAll));
        assert!(bar.focus_nav(0, 1));
        assert_eq!(bar.focused, Some(Slot::NcDndToggle));
        assert!(bar.focus_nav(0, 1));
        assert_eq!(bar.focused, Some(Slot::NcItem(1)));
        assert!(bar.focus_nav(1, 0));
        assert_eq!(bar.focused, Some(Slot::NcItemDismiss(1)));
        assert!(bar.focus_nav(0, 1));
        assert_eq!(bar.focused, Some(Slot::NcItemDismiss(2)));
        assert!(bar.focus_nav(-1, 0));
        assert_eq!(bar.focused, Some(Slot::NcItem(2)));
        assert_eq!(bar.focused_action(), Some(TopbarAction::ActivateNotification(2)));
    }

    #[test]
    fn the_focus_ring_is_reported_in_popup_local_coordinates() {
        let mut bar = quick_settings();
        bar.focus_nav(1, 0);
        let popup = bar.popup_rect().expect("popup rect");
        let slot = bar.slot_rect(Slot::QsVolumeMute).expect("mute slot");
        let ring = bar.focus_ring_in(Slot::in_quick_settings).expect("ring for the focused button");
        assert_eq!(ring.loc, slot.loc - popup.loc);
        assert_eq!(ring.size, slot.size);
        assert!(ring.loc.x >= FOCUS_RING);
        assert!(ring.loc.y >= FOCUS_RING);
        assert!(ring.loc.x + ring.size.w + FOCUS_RING <= popup.size.w);
        assert!(ring.loc.y + ring.size.h + FOCUS_RING <= popup.size.h);
        assert_eq!(bar.focus_ring_in(Slot::in_notification_center), None);
    }

    #[test]
    fn hover_and_focus_are_independent_states() {
        let mut bar = quick_settings();
        bar.focus_nav(1, 0);
        let tile = centre(bar.slot_rect(Slot::QsBluetooth).expect("bluetooth tile"));
        assert!(bar.set_hover(Some(tile)));
        assert_eq!(bar.hovered_in(Slot::in_quick_settings), Some(Slot::QsBluetooth));
        assert_eq!(bar.focused_in(Slot::in_quick_settings), Some(Slot::QsVolumeMute));
    }

    const _: () = assert!(FOCUS_RING == 2 && FOCUS_RING_STROKE == 2.0);

    #[test]
    fn popup_lifecycle_executes_full_opening_and_closing_animations() {
        for popup in [OpenPopup::LanguageMenu, OpenPopup::NotificationCenter, OpenPopup::QuickSettings] {
            let mut bar = Topbar::new(TopbarTheme::default());
            assert_eq!(bar.open_popup(), None);
            assert_eq!(bar.shown_popup(), None);

            bar.toggle_popup(popup);
            assert_eq!(bar.open_popup(), Some(popup));
            assert_eq!(bar.shown_popup(), Some(popup));
            assert_eq!(bar.closing_popup, None);
            let p_open = bar.popup_progress();
            assert!((0.0..=1.0).contains(&p_open));

            let closed = bar.close_popup();
            assert!(closed);
            assert_eq!(bar.open_popup(), None);
            assert_eq!(bar.closing_popup, Some(popup));
            assert_eq!(bar.shown_popup(), Some(popup));
            let p_close = bar.popup_progress();
            assert!((0.0..=1.0).contains(&p_close));

            bar.settle_motion();
            assert_eq!(bar.closing_popup, Some(popup));
            assert_eq!(bar.shown_popup(), Some(popup));

            std::thread::sleep(motion::t2() + Duration::from_millis(20));
            bar.settle_motion();
            assert_eq!(bar.closing_popup, None);
            assert_eq!(bar.shown_popup(), None);
            assert_eq!(bar.open_popup(), None);
        }
    }

    #[test]
    fn cache_key_resolution_properly_invalidates_stale_entries() {
        let mut cache = Cache::default();
        let draws = std::cell::Cell::new(0usize);
        let draw = || {
            draws.set(draws.get() + 1);
            let buf = canvas(10, 10, 1)?;
            Some(buffer_from(&buf, 10, 10, 1))
        };
        for (key, total) in [
            ((1u64, 0usize, None, false), 1),
            ((1, 0, None, false), 1),
            ((2, 0, None, false), 2),
            ((2, 1, None, false), 3),
            ((2, 1, Some(Slot::LayoutDropdown), false), 4),
            ((2, 1, Some(Slot::LayoutDropdown), true), 5),
        ] {
            assert!(cache.resolve(key, 1, draw).is_some());
            assert_eq!(draws.get(), total, "{key:?}");
        }

        let mut by_scale = Cache::default();
        draws.set(0);
        for (key, scale, total) in [(10u64, 1, 1), (10, 2, 2), (10, 1, 2), (10, 2, 2), (11, 1, 3)] {
            let _ = by_scale.resolve(key, scale, draw);
            assert_eq!(draws.get(), total, "{key} @ {scale}");
        }
    }

    #[test]
    fn system_status_and_layout_changes_advance_revision_counters() {
        let mut bar = Topbar::new(TopbarTheme::default());
        let s_rev0 = bar.status_rev;
        let status = crate::system_info::SystemStatus { volume_pct: 75, ..Default::default() };
        assert!(bar.set_system_status(status));
        assert_eq!(bar.status_rev, s_rev0.wrapping_add(1));

        let l_rev0 = bar.layouts_rev;
        assert!(bar.set_layouts(vec!["US".to_string(), "UA".to_string()], 0));
        assert_eq!(bar.layouts_rev, l_rev0.wrapping_add(1));
    }

    #[test]
    fn a_closing_popup_keeps_the_pump_alive_until_settle_retires_it() {
        let mut bar = quick_settings();
        assert!(bar.close_popup());
        assert!(bar.is_animating());
        std::thread::sleep(motion::t2() + Duration::from_millis(20));
        assert!(bar.is_animating(), "the faded popup still needs one settle pass to be retired");
        bar.settle_motion();
        assert_eq!(bar.closing_popup, None);
        assert!(!bar.is_animating());
    }

    #[test]
    fn leaving_a_control_early_fades_the_hover_layer_from_where_it_got_to() {
        let mut bar = topbar_with(&["US", "UA"]);
        bar.set_hover(Some(centre(bar.slot_rect(Slot::Caffeine).expect("caffeine"))));
        std::thread::sleep(Duration::from_millis(40));
        let partway = bar.hover_layers[&Slot::Caffeine].value();
        assert!(partway > 0.0 && partway < 1.0, "partway was {partway}");

        bar.set_hover(None);
        let reversed = bar.hover_layers[&Slot::Caffeine].value();
        assert!((reversed - partway).abs() < 0.1, "no flash: {partway} vs {reversed}");
        assert!(bar.is_animating());

        std::thread::sleep(motion::t1() + Duration::from_millis(20));
        bar.prune_layers();
        assert!(bar.hover_layers.is_empty());
    }

    #[test]
    fn a_press_layer_lands_at_once_and_fades_on_release() {
        let mut bar = topbar_with(&["US", "UA"]);
        assert!(bar.set_pressed(Some(Slot::Caffeine)));
        assert!((bar.press_layers[&Slot::Caffeine].value() - 1.0).abs() < f64::EPSILON);
        assert!(bar.set_pressed(None));
        assert!(bar.press_layers[&Slot::Caffeine].is_animating());
        assert!((bar.press_layers[&Slot::Caffeine].target()).abs() < f64::EPSILON);
    }

    #[test]
    fn the_badge_pops_in_from_six_tenths_and_holds_its_room_while_fading_out() {
        let mut bar = topbar_with(&["US", "UA"]);
        assert!(bar.set_unread(3));
        assert!(bar.badge_anim.is_animating());
        assert!((bar.badge_anim.target() - 1.0).abs() < f64::EPSILON);
        assert_eq!(bar.badge_shown(), 3);
        let start = BadgePop { count: 3, pop: 0.0 };
        assert!((start.scale() - BADGE_SCALE_FROM).abs() < f64::EPSILON);
        assert!(!start.visible());
        assert!((BadgePop { count: 3, pop: 1.0 }.scale() - 1.0).abs() < f64::EPSILON);

        std::thread::sleep(motion::t2() + Duration::from_millis(20));
        assert!(bar.set_unread(0));
        assert_eq!(bar.badge_shown(), 3, "the pill keeps its width while it shrinks");
        assert!(bar.is_animating());

        std::thread::sleep(motion::t1() + Duration::from_millis(20));
        bar.settle_motion();
        assert_eq!(bar.badge_shown(), 0);
        assert!(!bar.is_animating());
    }

    #[test]
    fn picking_the_other_segment_slides_the_pill_instead_of_jumping() {
        let mut bar = topbar_with(&["US", "UA"]);
        assert!(bar.set_layouts(vec!["US".to_string(), "UA".to_string()], 1));
        assert!(bar.segment_anim.is_animating());
        assert!((bar.segment_anim.target() - 1.0).abs() < f64::EPSILON);

        assert!(bar.set_layouts(vec!["US".to_string()], 0));
        assert!(!bar.segment_anim.is_animating(), "a new layout set jumps");

        assert_eq!(segment_pill((10, 20), (40, 30), 0.0), (10, 20));
        assert_eq!(segment_pill((10, 20), (40, 30), 0.5), (25, 25));
        assert_eq!(segment_pill((10, 20), (40, 30), 1.0), (40, 30));
        assert_eq!(segment_pill((10, 20), (40, 30), 1.4), (40, 30));
    }

    #[test]
    fn the_switch_thumb_travels_between_the_insets_and_never_leaves_the_track() {
        let off = switch_thumb_x(100, SWITCH_WIDTH, SWITCH_INSET, SWITCH_THUMB, 0.0);
        let on = switch_thumb_x(100, SWITCH_WIDTH, SWITCH_INSET, SWITCH_THUMB, 1.0);
        assert_eq!(off, 100 + SWITCH_INSET);
        assert_eq!(on, 100 + SWITCH_WIDTH - SWITCH_INSET - SWITCH_THUMB);
        let overshoot = switch_thumb_x(100, SWITCH_WIDTH, SWITCH_INSET, SWITCH_THUMB, 1.3);
        assert!(overshoot >= on);
        assert!(overshoot <= 100 + SWITCH_WIDTH - SWITCH_THUMB);
        let undershoot = switch_thumb_x(100, SWITCH_WIDTH, SWITCH_INSET, SWITCH_THUMB, -0.3);
        assert!(undershoot >= 100);
    }

    #[test]
    fn toggling_caffeine_drives_thumb_and_track_from_the_same_anim() {
        let mut bar = topbar_with(&["US", "UA"]);
        let rest = bar.caffeine_anim.switch();
        assert!(rest.thumb.abs() < f64::EPSILON);
        assert!(rest.track.abs() < f64::EPSILON);
        bar.set_caffeinated(true);
        assert!(bar.caffeine_anim.is_animating());
        std::thread::sleep(motion::t2() + Duration::from_millis(20));
        let settled = bar.caffeine_anim.switch();
        assert!((settled.thumb - 1.0).abs() < f64::EPSILON);
        assert!((settled.track - 1.0).abs() < f64::EPSILON);
        let on = settled.track_colour(bar.theme.track, bar.theme.active_background);
        for (got, want) in on.0.iter().zip(bar.theme.active_background.0) {
            assert!((got - want).abs() < 1e-5, "{on:?}");
        }
    }

    #[test]
    fn a_quick_settings_tile_fades_between_off_and_on_instead_of_snapping() {
        let mut bar = topbar_with(&["US", "UA"]);
        bar.settle_motion();
        assert!(bar.tile_level(QsTile::Dnd).abs() < f64::EPSILON, "a settled tile starts off");
        bar.dnd = true;
        bar.settle_motion();
        assert!(bar.is_animating());
        assert!(bar.tile_level(QsTile::Dnd) < 1.0, "the tile has not jumped to on");
        assert!(bar.tile_levels()[QsTile::Network.index()] <= bar.tile_levels()[QsTile::Dnd.index()]);
        std::thread::sleep(motion::t2() + Duration::from_millis(20));
        assert!((bar.tile_level(QsTile::Dnd) - 1.0).abs() < f64::EPSILON);
        assert_eq!(bar.tile_levels()[QsTile::Dnd.index()], quantize(1.0, MOTION_QUANTUM));
    }

    #[test]
    fn quantised_motion_keys_reuse_one_buffer_across_nearby_frames() {
        assert_eq!(quantize(0.51, BADGE_QUANTUM), quantize(0.53, BADGE_QUANTUM));
        assert_ne!(quantize(0.0, BADGE_QUANTUM), quantize(1.0, BADGE_QUANTUM));
        assert_eq!(quantize(9.0, BADGE_QUANTUM), quantize(2.0, BADGE_QUANTUM));

        let mut cache = Cache::default();
        let draws = std::cell::Cell::new(0usize);
        let draw = || {
            draws.set(draws.get() + 1);
            let buf = canvas(4, 4, 1)?;
            Some(buffer_from(&buf, 4, 4, 1))
        };
        for pop in [0.50, 0.51, 0.52, 0.53] {
            let _ = cache.resolve(quantize(pop, BADGE_QUANTUM), 1, draw);
        }
        assert_eq!(draws.get(), 1);
        let _ = cache.resolve(quantize(0.75, BADGE_QUANTUM), 1, draw);
        assert_eq!(draws.get(), 2);
    }

    #[test]
    fn state_layer_buffers_are_keyed_by_kind_size_radius_and_scale() {
        let mut bar = topbar_with(&["US", "UA"]);
        let size = Size::from((40, 24));
        assert!(bar.layer_buffer(Layer::Hover, size, glide_theme::shape::R2, 1).is_some());
        assert_eq!(bar.layers.len(), 1);
        let _ = bar.layer_buffer(Layer::Hover, size, glide_theme::shape::R2, 1);
        assert_eq!(bar.layers.len(), 1);
        let _ = bar.layer_buffer(Layer::Press, size, glide_theme::shape::R2, 1);
        let _ = bar.layer_buffer(Layer::Hover, size, glide_theme::shape::R1, 1);
        let _ = bar.layer_buffer(Layer::Hover, size, glide_theme::shape::R2, 2);
        let _ = bar.layer_buffer(Layer::Focus, size, glide_theme::shape::R2, 1);
        assert_eq!(bar.layers.len(), 5);
        bar.reconfigure(TopbarTheme { dark: false, ..TopbarTheme::default() });
        assert!(bar.layers.is_empty(), "a theme change drops every layer");
    }

    #[test]
    fn hovering_a_tile_raises_a_layer_job_over_that_tile_only() {
        let mut bar = quick_settings();
        let tile = bar.slot_rect(Slot::QsNightLight).expect("night light tile");
        bar.set_hover(Some(centre(tile)));
        std::thread::sleep(motion::t1() + Duration::from_millis(20));
        let jobs = bar.layer_jobs();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].layer, Layer::Hover);
        assert_eq!(jobs[0].rect, tile);
        assert!(jobs[0].in_popup);
        assert!((jobs[0].alpha - 1.0).abs() < f32::EPSILON);

        bar.set_focused(Some(Slot::QsNightLight));
        std::thread::sleep(motion::t1() + Duration::from_millis(20));
        let jobs = bar.layer_jobs();
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[1].layer, Layer::Focus);
        assert_eq!(jobs[1].rect, focus_ring_bounds(tile));
    }

    #[test]
    fn the_focus_ring_fades_out_where_it_last_was() {
        let mut bar = quick_settings();
        bar.focus_nav(1, 0);
        bar.settle_motion();
        let ring = bar.focus_ring_last.expect("ring recorded");
        assert_eq!(Some(ring), bar.slot_rect(Slot::QsVolumeMute));
        bar.set_focused(None);
        assert!(bar.focus_anim.is_animating());
        assert_eq!(bar.focus_ring_rect(), None);
        let jobs = bar.layer_jobs();
        assert!(jobs.iter().any(|job| job.layer == Layer::Focus && job.rect == focus_ring_bounds(ring)));
    }

    #[test]
    fn slots_without_a_state_layer_never_get_a_hover_anim() {
        let mut bar = quick_settings();
        let track = centre(bar.slot_rect(Slot::QsVolumeTrack).expect("volume track"));
        assert!(bar.set_hover(Some(track)));
        assert_eq!(bar.hovered, Some(Slot::QsVolumeTrack));
        assert!(bar.hover_layers.is_empty());
    }

    #[test]
    fn the_power_page_confirms_session_endings_inline_and_names_the_apps() {
        let mut bar = quick_settings();
        assert!(bar.open_detail(Detail::Power));
        bar.layout_slots(&painter(), output());
        assert!(bar.slot_rect(Slot::QsDetailSettings).is_none(), "the power page has no settings link");
        for action in panels::POWER_ROWS {
            assert!(bar.slot_rect(Slot::QsPowerButton(action)).is_some(), "{action:?}");
        }
        assert_eq!(click(&bar, Slot::QsPowerButton(PowerAction::Lock)), Some(TopbarAction::QsPower(PowerAction::Lock)));
        assert_eq!(
            click(&bar, Slot::QsPowerButton(PowerAction::Sleep)),
            Some(TopbarAction::QsPower(PowerAction::Sleep))
        );
        assert_eq!(
            click(&bar, Slot::QsPowerButton(PowerAction::Restart)),
            Some(TopbarAction::QsArmPower(PowerAction::Restart))
        );

        let shut_down = bar.slot_rect(Slot::QsPowerButton(PowerAction::ShutDown)).expect("shut down");
        assert!(bar.arm_power(PowerAction::Restart, vec!["Firefox".into(), "Files".into()]));
        bar.layout_slots(&painter(), output());
        let restart = bar.slot_rect(Slot::QsPowerButton(PowerAction::Restart)).expect("the row stays");
        let confirm = bar.slot_rect(Slot::QsPowerConfirm(PowerAction::Restart)).expect("confirm");
        let cancel = bar.slot_rect(Slot::QsPowerCancel).expect("cancel");
        assert!(cancel.loc.x + cancel.size.w < confirm.loc.x);
        assert!(confirm.loc.y > restart.loc.y);
        let moved = bar.slot_rect(Slot::QsPowerButton(PowerAction::ShutDown)).expect("shut down");
        assert!(moved.loc.y > shut_down.loc.y, "the accordion pushes the rows below it down");
        assert_eq!(
            click(&bar, Slot::QsPowerConfirm(PowerAction::Restart)),
            Some(TopbarAction::QsPower(PowerAction::Restart))
        );
        assert_eq!(click(&bar, Slot::QsPowerCancel), Some(TopbarAction::QsDisarmPower));
        assert!(bar.qs_escape(), "Esc folds the confirmation first");
        assert_eq!(bar.armed_power(), None);
        assert!(bar.qs_escape(), "then leaves the page");
        assert!(bar.has_open_popup());
    }

    #[test]
    fn the_apps_note_names_two_apps_and_counts_the_rest() {
        let names = |list: &[&str]| list.iter().map(|name| (*name).to_string()).collect::<Vec<_>>();
        assert!(panels::apps_note(&[]).contains(crate::i18n::t(crate::i18n::Key::QsNoAppsOpen)));
        let two = panels::apps_note(&names(&["Firefox", "Files"]));
        assert!(two.contains("Firefox") && two.contains("Files"));
        let four = panels::apps_note(&names(&["Firefox", "Files", "Terminal", "Text"]));
        assert!(four.contains('2') && !four.contains("Terminal"));
    }

    #[test]
    fn leaving_the_power_page_forgets_an_armed_action() {
        let mut bar = quick_settings();
        bar.open_detail(Detail::Power);
        bar.layout_slots(&painter(), output());
        bar.set_focused(Some(Slot::QsPowerButton(PowerAction::LogOut)));
        bar.arm_power(PowerAction::LogOut, Vec::new());
        assert_eq!(bar.focused, Some(Slot::QsPowerConfirm(PowerAction::LogOut)), "Enter on the row lands on Confirm");
        assert!(bar.disarm_power());
        assert_eq!(bar.focused, Some(Slot::QsPowerButton(PowerAction::LogOut)));

        bar.arm_power(PowerAction::LogOut, Vec::new());
        bar.close_popup();
        bar.toggle_popup(OpenPopup::QuickSettings);
        assert_eq!(bar.armed_power(), None);

        bar.open_detail(Detail::Wifi);
        bar.layout_slots(&painter(), output());
        let popup = bar.popup_rect().expect("popup");
        let page = bar.page_height(Some(&painter()), Detail::Wifi);
        assert_eq!(popup.size.h, page, "the page is laid out for the popup it is drawn in");
        let link = bar.slot_rect(Slot::QsDetailSettings).expect("settings link");
        assert_eq!(link.loc.y + link.size.h, popup.loc.y + popup.size.h - QS_PAD);
        bar.close_detail();
        bar.open_detail(Detail::Power);
        bar.layout_slots(&painter(), output());
        let short = bar.popup_rect().expect("popup").size.h;
        bar.arm_power(PowerAction::Restart, vec!["Firefox".into()]);
        bar.layout_slots(&painter(), output());
        assert!(bar.popup_rect().expect("popup").size.h > short, "the page grows with its accordion");
        bar.close_detail();

        bar.open_detail(Detail::Power);
        bar.arm_power(PowerAction::Restart, vec!["Firefox".into()]);
        assert!(bar.close_detail());
        assert_eq!(bar.armed_power(), None, "Back folds the confirmation too");
        bar.open_detail(Detail::Power);
        bar.layout_slots(&painter(), output());
        assert!(bar.slot_rect(Slot::QsPowerConfirm(PowerAction::Restart)).is_none());
    }

    #[test]
    fn the_power_profile_is_a_saver_tile_and_a_segmented_control_on_the_battery_page() {
        let mut bar = topbar_with(&["US"]);
        bar.set_system_status(crate::system_info::SystemStatus {
            battery_pct: Some(70),
            power_profile: Some(PowerProfile::Balanced),
            ..Default::default()
        });
        assert!((bar.profile_anim.value() - 1.0).abs() < f64::EPSILON, "the first reading jumps");
        let mut bar = with_popup(bar, OpenPopup::QuickSettings);
        assert_eq!(click(&bar, Slot::QsSaver), Some(TopbarAction::QsSetPowerProfile(PowerProfile::PowerSaver)));
        assert_eq!(click(&bar, Slot::QsSaverMore), Some(TopbarAction::QsPanel(PanelAction::Open(Detail::Battery))));
        assert!(bar.open_detail(Detail::Battery));
        bar.layout_slots(&painter(), output());
        let back = bar.slot_rect(Slot::QsBack).expect("back");
        let mut previous: Option<Rectangle<i32, Logical>> = None;
        for profile in QS_PROFILES {
            let rect = bar.slot_rect(Slot::QsProfile(profile)).expect("segment");
            assert!(rect.loc.y > back.loc.y + back.size.h);
            if let Some(previous) = previous {
                assert!(previous.loc.x + previous.size.w <= rect.loc.x);
                assert!((previous.size.w - rect.size.w).abs() <= 1);
            }
            previous = Some(rect);
            assert_eq!(click(&bar, Slot::QsProfile(profile)), Some(TopbarAction::QsSetPowerProfile(profile)));
        }

        let mut status = bar.system_status().clone();
        status.power_profile = Some(PowerProfile::Performance);
        bar.set_system_status(status);
        assert!(bar.profile_anim.is_animating());
        assert!((bar.profile_anim.target() - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_microphone_adds_its_row_between_volume_and_brightness() {
        let bar = quick_settings();
        assert!(bar.slot_rect(Slot::QsMicTrack).is_none(), "no source, no row");
        let without = bar.popup_rect().expect("popup").size.h;

        let mut bar = topbar_with(&["US"]);
        bar.set_system_status(crate::system_info::SystemStatus {
            mic_available: true,
            mic_pct: 30,
            ..Default::default()
        });
        let mut bar = with_popup(bar, OpenPopup::QuickSettings);
        let volume = bar.slot_rect(Slot::QsVolumeTrack).expect("volume");
        let mic = bar.slot_rect(Slot::QsMicTrack).expect("mic");
        let brightness = bar.slot_rect(Slot::QsBrightnessTrack).expect("brightness");
        assert!(volume.loc.y < mic.loc.y && mic.loc.y < brightness.loc.y);
        assert!(bar.popup_rect().expect("popup").size.h > without);
        assert_eq!(click(&bar, Slot::QsMicMute), Some(TopbarAction::QsMicMute));
        let quarter =
            Point::from((f64::from(mic.loc.x) + f64::from(mic.size.w) * 0.25, f64::from(mic.loc.y + mic.size.h / 2)));
        assert_eq!(bar.action_at(quarter), Some(TopbarAction::QsMicAdjust(25)));
        bar.set_focused(Some(Slot::QsMicTrack));
        assert_eq!(bar.focus_adjust(-5), Some(TopbarAction::QsMicAdjust(25)));
        assert_eq!(bar.system_status().mic_pct, 25);
        bar.set_focused(Some(Slot::QsVolumeTrack));
        assert!(bar.focus_nav(0, 1));
        assert_eq!(bar.focused, Some(Slot::QsMicTrack));
        assert!(Slot::QsMicTrack.is_slider() && !Slot::QsMicMute.is_slider());
    }

    fn device(name: &str, active: bool, icon: &'static str) -> crate::qs_net::AudioOutput {
        crate::qs_net::AudioOutput { name: name.to_string(), description: name.to_uppercase(), icon, active }
    }

    fn with_devices() -> Topbar {
        let mut bar = topbar_with(&["US"]);
        bar.set_system_status(crate::system_info::SystemStatus { mic_available: true, ..Default::default() });
        let mut bar = with_popup(bar, OpenPopup::QuickSettings);
        bar.apply_net_event(crate::qs_net::NetEvent::Outputs(vec![
            device("speakers", true, "audio-speakers"),
            device("hdmi", false, "video-display"),
        ]));
        bar.apply_net_event(crate::qs_net::NetEvent::Inputs(vec![device("mic", true, "audio-input-microphone")]));
        bar.layout_slots(&painter(), output());
        bar
    }

    #[test]
    fn a_connected_headset_gets_an_applet_that_opens_its_own_popover() {
        let mut bar = topbar_with(&["US"]);
        bar.apply_net_event(crate::qs_net::NetEvent::Outputs(vec![
            device("speakers", true, "audio-speakers"),
            device("bluez_output.AA_BB_CC_DD_EE_FF.1", false, "audio-headset"),
        ]));
        bar.layout_slots(&painter(), output());
        assert!(bar.slot_rect(Slot::Headset).is_none(), "no headset, no applet");
        bar.set_headset(crate::headset::Headset {
            mac: "AA_BB_CC_DD_EE_FF".into(),
            name: "Buds".into(),
            icon: "audio-headset".into(),
            battery: Some(64),
            address: "AA:BB:CC:DD:EE:FF".into(),
            volume: None,
            profile: crate::headset::Profile::default(),
        });
        bar.layout_slots(&painter(), output());
        let applet = bar.slot_rect(Slot::Headset).expect("the applet");
        let status = bar.slot_rect(Slot::Status).expect("the status block");
        assert!(applet.loc.x + applet.size.w <= status.loc.x, "the applet sits left of the status block");
        assert_eq!(click(&bar, Slot::Headset), Some(TopbarAction::ToggleHeadset));

        assert!(bar.toggle_popup(OpenPopup::Headset));
        bar.layout_slots(&painter(), output());
        let popup = bar.popup_rect.expect("the popover");
        assert!(popup.loc.y > applet.loc.y + applet.size.h, "the popover opens under the bar");
        assert_eq!(click(&bar, Slot::HeadsetOutput), Some(TopbarAction::HeadsetUseOutput));
        assert_eq!(click(&bar, Slot::HeadsetDisconnect), Some(TopbarAction::HeadsetDisconnect));
        assert_eq!(click(&bar, Slot::HeadsetSettings), Some(TopbarAction::HeadsetSettings));
        assert_eq!(bar.headset_output().map(|output| output.name.as_str()), Some("bluez_output.AA_BB_CC_DD_EE_FF.1"));

        assert!(bar.forget_headset("AA_BB_CC_DD_EE_FF"));
        assert_eq!(bar.open_popup(), None, "the popover goes with the headset");
        bar.layout_slots(&painter(), output());
        assert!(bar.slot_rect(Slot::Headset).is_none());
    }

    fn rich_status() -> crate::system_info::SystemStatus {
        use crate::system_info::{BatteryHealth, BatteryInfo, BatteryState};
        crate::system_info::SystemStatus {
            battery_pct: Some(64),
            battery: Some(BatteryInfo {
                pct: 64,
                state: BatteryState::Discharging,
                minutes: Some(212),
                health: BatteryHealth::default(),
                profile: Some(PowerProfile::Balanced),
            }),
            power_profile: Some(PowerProfile::Balanced),
            mic_available: true,
            mic_pct: 40,
            volume_pct: 64,
            brightness_pct: 70,
            wifi_enabled: true,
            wifi_available: true,
            wifi_ssid: "Home".into(),
            wifi_level: 3,
            bluetooth_available: true,
            bluetooth_on: true,
            ..Default::default()
        }
    }

    #[test]
    fn every_quick_settings_surface_draws_without_icons_at_both_scales() {
        let mut bar = with_devices();
        let mut status = rich_status();
        status.privacy.microphone = vec!["Firefox".into()];
        status.volume_muted = true;
        bar.set_system_status(status);
        bar.layout_slots(&painter(), output());

        let painter = painter();
        let theme = bar.theme.clone();
        let (_, flags, motion, _, shape) = bar.qs_main_key(&painter);
        let geometry = qs_geometry(&shape);
        assert!(geometry.strip.is_some() && geometry.mic.is_some() && geometry.footer.chip.is_some());
        let hovers = [None, Some(Slot::QsVolumeTrack), Some(Slot::QsWifiMore)];
        for scale in [1, 2] {
            for hovered in hovers {
                let ctx = QsRenderContext { painter: &painter, theme: &theme, hovered };
                let mut canvas = Canvas::new(QS_WIDTH, geometry.height, scale).expect("main canvas");
                let flags = QsFlags { dragging: Some(Slot::QsMicTrack), ..flags };
                let draw = QsDraw {
                    status: bar.system_status(),
                    panel: &bar.qs_panel,
                    geometry: geometry.clone(),
                    flags,
                    motion,
                    sliders: true,
                };
                draw_quick_settings(&mut canvas, &ctx, &draw);
            }
            assert!(draw_qs_background((QS_WIDTH, geometry.height), &theme, scale).is_some());
        }
    }

    type PreviewAdjust = fn(&mut crate::system_info::SystemStatus, &mut Topbar);

    fn preview_variant(bar: &mut Topbar, variant: &str) {
        match variant {
            "-info" => {
                let info = crate::qs_net::WifiInfo { ip: "192.168.1.24".into(), freq_mhz: 5180, rate_mbps: 866 };
                bar.apply_net_event(crate::qs_net::NetEvent::WifiInfo(Some(info)));
                bar.toggle_info("Home");
            }
            "-password" => bar.toggle_prompt("Cafe Mriia"),
            _ => {}
        }
    }

    fn preview_lists(bar: &mut Topbar) {
        use crate::qs_net::{BtDevice, BtKind, NetEvent, Security, VpnConnection, VpnKind, WifiNetwork};
        let net = |ssid: &str, security, signal, connected, saved| WifiNetwork {
            ssid: ssid.into(),
            security,
            signal,
            connected,
            saved,
        };
        bar.apply_net_event(NetEvent::Wifi(vec![
            net("Home", Security::Passphrase, 90, true, true),
            net("Cafe Mriia", Security::Open, 60, false, false),
            net("Neighbour", Security::Passphrase, 40, false, false),
            net("eduroam", Security::Enterprise, 30, false, false),
        ]));
        let bt = |mac: &str, name: &str, kind, connected, battery| BtDevice {
            mac: mac.into(),
            name: name.into(),
            kind,
            connected,
            battery,
        };
        bar.apply_net_event(NetEvent::Bluetooth(vec![
            bt("aa", "WH-1000XM4", BtKind::Headphones, true, Some(74)),
            bt("bb", "MX Master 3", BtKind::Mouse, false, None),
        ]));
        bar.apply_net_event(NetEvent::Outputs(vec![
            device("speakers", true, "audio-speakers"),
            device("hdmi", false, "video-display"),
        ]));
        bar.apply_net_event(NetEvent::Inputs(vec![device("mic", true, "audio-input-microphone")]));
        bar.apply_net_event(NetEvent::Vpn(vec![
            VpnConnection { name: "Office".into(), kind: VpnKind::WireGuard, active: true },
            VpnConnection { name: "Proton".into(), kind: VpnKind::Vpn, active: false },
        ]));
        bar.begin_connect("Neighbour");
    }

    #[test]
    #[ignore = "writes preview PNGs to $GLIDE_QS_PREVIEW"]
    fn quick_settings_preview() {
        use crate::system_info::BatteryState;
        let Some(dir) = std::env::var_os("GLIDE_QS_PREVIEW") else {
            return;
        };
        let painter = painter();
        let mut icons = crate::icon::IconLoader::new(std::env::var("GLIDE_QS_PREVIEW_ICONS").ok());
        let save = |canvas: &Canvas<'_>, file: String| {
            let (w, h) = ((canvas.w * canvas.scale) as u32, (canvas.h * canvas.scale) as u32);
            let image = glide_shell::image::Image::from_bgra(w, h, canvas.buf.clone()).expect("image");
            std::fs::write(std::path::Path::new(&dir).join(file.to_ascii_lowercase()), image.png().expect("png"))
                .expect("write");
        };
        for appearance in [glide_theme::Appearance::Dark, glide_theme::Appearance::Light] {
            let theme = TopbarTheme::from_tokens(&glide_theme::Tokens::new(appearance, glide_theme::Accent::Blue));
            let cases: [(&str, PreviewAdjust); 4] = [
                ("rest", |_, _| {}),
                ("busy", |status, bar| {
                    status.privacy.microphone = vec!["Firefox".into()];
                    status.mic_muted = true;
                    status.power_profile = Some(PowerProfile::PowerSaver);
                    status.night_light_on = true;
                    bar.dnd = true;
                }),
                ("charging", |status, _| {
                    status.battery_charging = true;
                    if let Some(info) = status.battery.as_mut() {
                        info.state = BatteryState::Charging;
                        info.minutes = Some(48);
                    }
                }),
                ("desktop", |status, _| {
                    status.battery_pct = None;
                    status.battery = None;
                    status.power_profile = None;
                    status.brightness_available = false;
                    status.mic_available = false;
                }),
            ];
            for (name, adjust) in cases {
                let mut bar = Topbar::new(theme.clone());
                let mut status = rich_status();
                adjust(&mut status, &mut bar);
                bar.set_system_status(status);
                bar.toggle_popup(OpenPopup::QuickSettings);
                bar.settle_motion();
                std::thread::sleep(motion::t2() + Duration::from_millis(20));
                bar.layout_slots(&painter, output());
                let (_, flags, motion, _, shape) = bar.qs_main_key(&painter);
                let geometry = qs_geometry(&shape);
                let ctx = QsRenderContext { painter: &painter, theme: &theme, hovered: None };
                let mut canvas = Canvas::popover((QS_WIDTH, geometry.height), 2, R3, theme.popover_bg, theme.line)
                    .expect("canvas")
                    .with_icons(Some(&mut icons));
                let draw = QsDraw {
                    status: bar.system_status(),
                    panel: &bar.qs_panel,
                    geometry,
                    flags,
                    motion,
                    sliders: true,
                };
                draw_quick_settings(&mut canvas, &ctx, &draw);
                save(&canvas, format!("qs-{name}-{appearance:?}.png"));

                if name != "rest" {
                    continue;
                }
                preview_lists(&mut bar);
                for (page, armed, variant) in [
                    (Detail::Battery, None, ""),
                    (Detail::Power, None, ""),
                    (Detail::Power, Some(PowerAction::Restart), "-armed"),
                    (Detail::Sound, None, ""),
                    (Detail::Wifi, None, ""),
                    (Detail::Wifi, None, "-info"),
                    (Detail::Wifi, None, "-password"),
                    (Detail::Bluetooth, None, ""),
                    (Detail::Vpn, None, ""),
                ] {
                    bar.open_detail(page);
                    if let Some(action) = armed {
                        bar.arm_power(action, vec!["Firefox".into(), "Files".into(), "Terminal".into()]);
                    }
                    preview_variant(&mut bar, variant);
                    let height = bar.page_height(Some(&painter), page);
                    let mut canvas = Canvas::popover((QS_WIDTH, height), 2, R3, theme.popover_bg, theme.line)
                        .expect("canvas")
                        .with_icons(Some(&mut icons));
                    let status = bar.system_status().clone();
                    panels::draw_detail(&mut canvas, &ctx, &bar.qs_panel, page, &status, 1.0);
                    save(&canvas, format!("qs-page-{page:?}{variant}-{appearance:?}.png"));
                    bar.close_detail();
                }
            }
        }
    }

    #[test]
    fn picking_a_device_keeps_the_focus_on_its_row() {
        let mut bar = with_devices();
        assert!(bar.open_detail(Detail::Sound));
        bar.set_focused(Some(Slot::QsDrawerItem(Drawer::Input, 0)));
        bar.begin_connect("mic");
        assert_eq!(bar.focused, Some(Slot::QsDrawerItem(Drawer::Input, 0)), "a microphone card in the Sound view");
        bar.set_focused(Some(Slot::QsDetailRow(1)));
        bar.begin_connect("hdmi");
        assert_eq!(bar.focused, Some(Slot::QsDetailRow(1)));
        bar.begin_connect("nothing-like-it");
        assert_eq!(bar.focused, Some(Slot::QsDetailRow(1)), "an unknown key leaves the focus where it was");
    }

    #[test]
    fn a_new_battery_estimate_repaints_the_main_view() {
        let mut bar = quick_settings();
        let battery = crate::system_info::BatteryInfo {
            pct: 60,
            state: crate::system_info::BatteryState::Discharging,
            minutes: Some(90),
            health: crate::system_info::BatteryHealth::default(),
            profile: None,
        };
        bar.set_system_status(crate::system_info::SystemStatus {
            battery_pct: Some(60),
            battery: Some(battery),
            ..Default::default()
        });
        let before = bar.qs_main_key(&painter());
        let mut status = bar.system_status().clone();
        status.battery = Some(crate::system_info::BatteryInfo { minutes: Some(80), ..battery });
        bar.set_system_status(status);
        assert_ne!(bar.qs_main_key(&painter()), before, "the estimate is drawn on the main view");
    }

    #[test]
    fn the_static_card_keeps_the_volume_icon_of_the_live_level() {
        for muted in [false, true] {
            for pct in 0..=100 {
                assert_eq!(volume_icon_name(muted, volume_icon_step(pct)), volume_icon_name(muted, pct), "{pct}");
            }
        }
    }

    #[test]
    fn a_slider_layer_holds_the_dragged_thumb_and_its_ring() {
        let reach = QS_SLIDER_THUMB_DRAG + QS_SLIDER_RING_DRAG * 2 + 2;
        assert!(SLIDER_HALO * 2 >= reach, "a thumb at either end of the track stays inside its layer");
    }

    #[test]
    fn a_split_tile_toggles_on_its_body_and_opens_its_page_from_the_chevron() {
        let mut bar = topbar_with(&["US"]);
        bar.set_system_status(rich_status());
        let bar = with_popup(bar, OpenPopup::QuickSettings);
        for (main, more, toggle, detail) in [
            (Slot::QsWifi, Slot::QsWifiMore, TopbarAction::QsToggleWifi, Detail::Wifi),
            (Slot::QsBluetooth, Slot::QsBluetoothMore, TopbarAction::QsToggleBluetooth, Detail::Bluetooth),
        ] {
            let body = bar.slot_rect(main).expect("tile body");
            let chevron = bar.slot_rect(more).expect("chevron");
            assert_eq!(chevron.size.w, QS_MORE_W);
            assert_eq!(body.loc.x + body.size.w, chevron.loc.x, "the two halves meet at the hairline");
            assert_eq!((body.loc.y, body.size.h), (chevron.loc.y, chevron.size.h));
            assert_eq!(bar.action_at(centre(body)), Some(toggle));
            assert_eq!(bar.action_at(centre(chevron)), Some(TopbarAction::QsPanel(PanelAction::Open(detail))));
        }
        let plain = bar.slot_rect(Slot::QsDnd).expect("plain tile");
        let split = bar.slot_rect(Slot::QsWifi).expect("split tile");
        assert_eq!(plain.size.w, split.size.w + QS_MORE_W, "a plain tile is one target");
    }

    #[test]
    fn set_recording_activates_is_recording_and_sharing_screen() {
        let mut bar = topbar_with(&["US", "UA"]);
        assert!(!bar.is_recording());
        assert!(!bar.sharing_screen());
        assert_eq!(bar.recording_elapsed(Instant::now()), None);
        assert_eq!(bar.slot_rect(Slot::StopRecording), None);

        let started = Instant::now();
        bar.set_recording(Some(started));
        assert!(bar.is_recording());
        assert!(bar.sharing_screen());
        let future = started + Duration::from_secs(5);
        assert_eq!(bar.recording_elapsed(future), Some(Duration::from_secs(5)));

        bar.set_recording(None);
        assert!(!bar.is_recording());
        assert_eq!(bar.recording_elapsed(Instant::now()), None);
    }

    #[test]
    fn stop_recording_pill_layout_hit_tests_and_motion() {
        let mut bar = topbar_with(&["US", "UA"]);
        let started = Instant::now();
        let _pass = glide_ui::clock::FramePass::enter(started);
        bar.set_recording(Some(started));
        assert!(bar.recording_anim.is_animating());
        assert!((bar.recording_anim.target() - 1.0).abs() < f64::EPSILON);

        bar.layout_slots(&painter(), output());

        let rect = bar.slot_rect(Slot::StopRecording).expect("recording slot present");
        assert!(rect.size.w > 40);
        assert_eq!(rect.size.h, BUTTON_HEIGHT);

        let hit = centre(rect);
        assert_eq!(bar.action_at(hit), Some(TopbarAction::StopRecording));
        assert_eq!(Slot::StopRecording.action(), TopbarAction::StopRecording);
        assert_eq!(Slot::StopRecording.accessible_key(), Some(Key::TbScreenRecording));

        let mid = 0.5;
        let progress_half = glide_theme::motion::assist(mid);
        assert!((progress_half - 1.0).abs() < 1e-6);
        let progress_early = glide_theme::motion::assist(0.2);
        assert!(progress_early > 0.0 && progress_early < 1.0);
        let travel_at_half: f64 = (1.0 - mid) * -6.0;
        assert!(travel_at_half.abs() >= 3.0);
        let travel_at_start: f64 = (1.0 - 0.0) * -6.0;
        assert!((travel_at_start.abs() - 6.0).abs() < 1e-6);
        let scale_at_start = lerp(0.92, 1.0, 0.0);
        assert!((scale_at_start - 0.92).abs() < 1e-6);
        assert!((1.0 - scale_at_start) >= 0.06);

        assert!(bar.toggle_popup(OpenPopup::QuickSettings));
        assert!(bar.has_open_popup());
        assert!(bar.set_pressed(Some(Slot::StopRecording)));
        assert!(!bar.has_open_popup());
    }

    #[test]
    fn next_wake_computes_one_second_tick_and_idle_recording_is_not_animating() {
        let mut bar = topbar_with(&["US", "UA"]);
        let base_instant = Instant::now();
        let pass = glide_ui::clock::FramePass::enter(base_instant);

        bar.set_recording(Some(base_instant));
        assert!(bar.is_animating());

        drop(pass);
        let after_tween = base_instant + motion::t2() + Duration::from_millis(50);
        let pass2 = glide_ui::clock::FramePass::enter(after_tween);

        bar.settle_motion();
        assert!(!bar.is_animating());

        let wake = bar.next_wake().expect("wake time scheduled");
        let expected_next_second = base_instant + Duration::from_secs(1);
        assert_eq!(wake, expected_next_second);

        drop(pass2);
        let later = base_instant + Duration::from_millis(3200);
        let pass3 = glide_ui::clock::FramePass::enter(later);
        let wake_later = bar.next_wake().expect("wake time scheduled");
        let expected_later = base_instant + Duration::from_secs(4);
        assert_eq!(wake_later, expected_later);
        drop(pass3);
    }

    #[test]
    fn recording_invalidates_raster_when_whole_second_changes() {
        let mut bar = topbar_with(&["US", "UA"]);
        let start = Instant::now();
        let pass = glide_ui::clock::FramePass::enter(start);
        bar.set_recording(Some(start));
        bar.settle_motion();
        assert_eq!(bar.recording_last_sec, Some(0));
        let rev1 = bar.indicators.rev;

        drop(pass);
        let subsecond = start + Duration::from_millis(400);
        let pass2 = glide_ui::clock::FramePass::enter(subsecond);
        bar.settle_motion();
        assert_eq!(bar.indicators.rev, rev1);

        drop(pass2);
        let next_sec = start + Duration::from_millis(1050);
        let pass3 = glide_ui::clock::FramePass::enter(next_sec);
        bar.settle_motion();
        assert_eq!(bar.recording_last_sec, Some(1));
        assert!(bar.indicators.rev > rev1);
        drop(pass3);
    }
}
