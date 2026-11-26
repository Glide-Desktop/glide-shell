use std::{
    sync::{
        Arc, Condvar, Mutex, PoisonError,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, anyhow, bail};
use x11rb::{
    CURRENT_TIME, NONE,
    connection::Connection,
    protocol::{
        Event,
        damage::{self, ConnectionExt as _},
        shape::{self, ConnectionExt as _},
        xfixes::ConnectionExt as _,
        xproto::{
            Atom, AtomEnum, ButtonPressEvent, ButtonReleaseEvent, ChangeWindowAttributesAux, ClientMessageEvent,
            ColormapAlloc, ConfigWindow, ConfigureWindowAux, ConnectionExt as _, CreateWindowAux, EnterNotifyEvent,
            EventMask, ImageFormat, InputFocus, KeyButMask, NotifyDetail, NotifyMode, PropMode, Window, WindowClass,
        },
    },
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

use crate::{
    pixmap::{self, Icon},
    sni::{self, Click, Clicker, Exported, Look},
};

pub const CONTAINER_CLASS: &str = "glide-xembed";
pub const ICON_SIZE: u16 = 48;
const MAX_CAPTURE_SIDE: u16 = 256;
const SYSTEM_TRAY_REQUEST_DOCK: u32 = 0;
const XEMBED_EMBEDDED_NOTIFY: u32 = 0;
const XEMBED_WINDOW_ACTIVATE: u32 = 1;
const XEMBED_REQUEST_FOCUS: u32 = 3;
const XEMBED_FOCUS_IN: u32 = 4;
const XEMBED_FOCUS_CURRENT: u32 = 0;
const XEMBED_MAPPED: u32 = 1;
const WM_SIZE_P_MIN_SIZE: u32 = 1 << 4;
const WM_SIZE_P_MAX_SIZE: u32 = 1 << 5;
const CLICK_HOLD: Duration = Duration::from_millis(20);
const TIME_WAIT: Duration = Duration::from_millis(100);

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        _GLIDE_XEMBED_TIME,
        _NET_SYSTEM_TRAY_ICON_SIZE,
        _NET_SYSTEM_TRAY_OPCODE,
        _NET_SYSTEM_TRAY_ORIENTATION,
        _NET_SYSTEM_TRAY_VISUAL,
        _NET_WM_NAME,
        _NET_WM_WINDOW_TYPE,
        _NET_WM_WINDOW_TYPE_NORMAL,
        _XEMBED,
        _XEMBED_INFO,
        MANAGER,
        UTF8_STRING,
    }
}

pub type Registered = Arc<Mutex<Vec<String>>>;

#[derive(Default)]
struct ClockState {
    time: u32,
    generation: u64,
}

#[derive(Default)]
struct ServerClock {
    state: Mutex<ClockState>,
    changed: Condvar,
}

impl ServerClock {
    fn note(&self, time: u32) {
        if time == CURRENT_TIME {
            return;
        }
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.time = time;
    }

    fn deliver(&self, time: u32) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if time != CURRENT_TIME {
            state.time = time;
        }
        state.generation = state.generation.wrapping_add(1);
        self.changed.notify_all();
    }

    fn current(&self) -> u32 {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).time
    }

    fn generation(&self) -> u64 {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).generation
    }

    fn wait_after(&self, generation: u64, timeout: Duration) -> u32 {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let (state, _) = self
            .changed
            .wait_timeout_while(state, timeout, |state| state.generation == generation)
            .unwrap_or_else(PoisonError::into_inner);
        state.time
    }
}

struct XClicker {
    conn: Arc<RustConnection>,
    root: Window,
    owner: Window,
    container: Window,
    xembed_atom: Atom,
    time_atom: Atom,
    clock: Arc<ServerClock>,
    last_clicked: Arc<AtomicU32>,
    serial: Mutex<()>,
}

fn held_mask(button: u8) -> KeyButMask {
    match button {
        1 => KeyButMask::BUTTON1,
        2 => KeyButMask::BUTTON2,
        3 => KeyButMask::BUTTON3,
        4 => KeyButMask::BUTTON4,
        5 => KeyButMask::BUTTON5,
        _ => KeyButMask::default(),
    }
}

#[must_use]
pub fn valid_root_point(at: Option<(i32, i32)>) -> Option<(i16, i16)> {
    let (x, y) = at?;
    if x == 0 && y == 0 {
        return None;
    }
    Some((i16::try_from(x).ok()?, i16::try_from(y).ok()?))
}

#[must_use]
pub fn container_origin(root_at: (i16, i16), child_pos: (i16, i16), event_at: (i16, i16)) -> (i32, i32) {
    (
        i32::from(root_at.0) - i32::from(child_pos.0) - i32::from(event_at.0),
        i32::from(root_at.1) - i32::from(child_pos.1) - i32::from(event_at.1),
    )
}

#[must_use]
pub fn target_size(desired: u16, hints: Option<&[u32]>) -> (u16, u16) {
    let (mut w, mut h) = (desired.max(1), desired.max(1));
    let Some(hints) = hints else {
        return (w, h);
    };
    let flags = hints.first().copied().unwrap_or(0);
    if flags & WM_SIZE_P_MAX_SIZE != 0
        && let (Some(&max_w), Some(&max_h)) = (hints.get(7), hints.get(8))
    {
        if max_w > 0 {
            w = w.min(u16::try_from(max_w).unwrap_or(u16::MAX));
        }
        if max_h > 0 {
            h = h.min(u16::try_from(max_h).unwrap_or(u16::MAX));
        }
    }
    if flags & WM_SIZE_P_MIN_SIZE != 0
        && let (Some(&min_w), Some(&min_h)) = (hints.get(5), hints.get(6))
    {
        if min_w > 0 {
            w = w.max(u16::try_from(min_w).unwrap_or(desired));
        }
        if min_h > 0 {
            h = h.max(u16::try_from(min_h).unwrap_or(desired));
        }
    }
    (w.clamp(1, desired.max(1)), h.clamp(1, desired.max(1)))
}

fn send_xembed(
    conn: &RustConnection, atom: Atom, window: Window, time: u32, opcode: u32, detail: u32, data1: u32,
) -> anyhow::Result<()> {
    let message = ClientMessageEvent::new(32, window, atom, [time, opcode, detail, data1, 0]);
    conn.send_event(false, window, EventMask::NO_EVENT, message)?;
    Ok(())
}

impl XClicker {
    fn server_time(&self) -> u32 {
        let generation = self.clock.generation();
        let poked = self
            .conn
            .change_property8(PropMode::APPEND, self.owner, self.time_atom, AtomEnum::STRING, &[])
            .and_then(|_| self.conn.flush());
        if poked.is_ok() {
            return self.clock.wait_after(generation, TIME_WAIT);
        }
        self.clock.current()
    }

    fn send(&self, window: Window, click: Click, at: Option<(i32, i32)>) -> anyhow::Result<()> {
        let _guard = self.serial.lock().unwrap_or_else(PoisonError::into_inner);
        self.last_clicked.store(window, Ordering::Relaxed);
        let (win_x, win_y, event_x, event_y) =
            self.conn.get_geometry(window).ok().and_then(|cookie| cookie.reply().ok()).map_or_else(
                || {
                    let half = i16::try_from(ICON_SIZE / 2).unwrap_or(0);
                    (0, 0, half, half)
                },
                |geom| {
                    (
                        geom.x,
                        geom.y,
                        i16::try_from(geom.width / 2).unwrap_or(0),
                        i16::try_from(geom.height / 2).unwrap_or(0),
                    )
                },
            );
        let (root_x, root_y) = if let Some((rx, ry)) = valid_root_point(at) {
            let (cx, cy) = container_origin((rx, ry), (win_x, win_y), (event_x, event_y));
            let _ = self.conn.configure_window(self.container, &ConfigureWindowAux::new().x(cx).y(cy));
            (rx, ry)
        } else {
            let translated = self.conn.translate_coordinates(window, self.root, event_x, event_y)?.reply()?;
            (translated.dst_x, translated.dst_y)
        };
        let press_time = self.server_time();
        send_xembed(&self.conn, self.xembed_atom, window, press_time, XEMBED_WINDOW_ACTIVATE, 0, 0)?;
        send_xembed(&self.conn, self.xembed_atom, window, press_time, XEMBED_FOCUS_IN, XEMBED_FOCUS_CURRENT, 0)?;
        let _ = self.conn.set_input_focus(InputFocus::PARENT, window, press_time);
        let enter = EnterNotifyEvent {
            response_type: x11rb::protocol::xproto::ENTER_NOTIFY_EVENT,
            detail: NotifyDetail::NONLINEAR,
            sequence: 0,
            time: press_time,
            root: self.root,
            event: window,
            child: NONE,
            root_x,
            root_y,
            event_x,
            event_y,
            state: KeyButMask::default(),
            mode: NotifyMode::NORMAL,
            same_screen_focus: 1,
        };
        let button = click.button();
        let press = ButtonPressEvent {
            response_type: x11rb::protocol::xproto::BUTTON_PRESS_EVENT,
            detail: button,
            sequence: 0,
            time: press_time,
            root: self.root,
            event: window,
            child: NONE,
            root_x,
            root_y,
            event_x,
            event_y,
            state: KeyButMask::default(),
            same_screen: true,
        };
        self.conn.send_event(false, window, EventMask::ENTER_WINDOW, enter)?;
        self.conn.send_event(false, window, EventMask::BUTTON_PRESS, press)?;
        self.conn.flush()?;
        std::thread::sleep(CLICK_HOLD);
        let release_time = self.server_time().max(press_time);
        let release = ButtonReleaseEvent {
            response_type: x11rb::protocol::xproto::BUTTON_RELEASE_EVENT,
            time: release_time,
            state: held_mask(button),
            ..press
        };
        self.conn.send_event(false, window, EventMask::BUTTON_RELEASE, release)?;
        self.conn.flush()?;
        Ok(())
    }
}

impl Clicker for XClicker {
    fn click(&self, window: u32, click: Click, at: Option<(i32, i32)>) {
        if let Err(err) = self.send(window, click, at) {
            tracing::debug!(window, ?click, ?at, "could not pass a click to a tray icon: {err}");
        }
    }
}

struct Docked {
    window: Window,
    damage: u32,
    mapped: bool,
    damaged: bool,
    look: Look,
    exported: Option<Exported>,
}

pub struct Host {
    conn: Arc<RustConnection>,
    root: Window,
    atoms: Atoms,
    selection: Atom,
    owner: Window,
    container: Window,
    has_shape: bool,
    clock: Arc<ServerClock>,
    last_clicked: Arc<AtomicU32>,
    clicker: Arc<XClicker>,
    control: zbus::blocking::Connection,
    bus: Option<String>,
    registered: Registered,
    icons: Vec<Docked>,
    serial: u32,
}

fn argb_visual(screen: &x11rb::protocol::xproto::Screen) -> Option<u32> {
    screen
        .allowed_depths
        .iter()
        .filter(|depth| depth.depth == 32)
        .flat_map(|depth| depth.visuals.iter())
        .find(|visual| visual.class == x11rb::protocol::xproto::VisualClass::TRUE_COLOR)
        .map(|visual| visual.visual_id)
}

fn text_property(conn: &RustConnection, window: Window, property: Atom, kind: Atom) -> Option<String> {
    let reply = conn.get_property(false, window, property, kind, 0, 1024).ok()?.reply().ok()?;
    let text = String::from_utf8_lossy(&reply.value).trim_end_matches('\0').trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn wm_class(conn: &RustConnection, window: Window) -> Option<String> {
    let reply = conn.get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 1024).ok()?.reply().ok()?;
    let mut fields = reply.value.split(|byte| *byte == 0).filter(|field| !field.is_empty());
    let instance = fields.next();
    let class = fields.next().or(instance)?;
    Some(String::from_utf8_lossy(class).into_owned())
}

fn wm_normal_hints(conn: &RustConnection, window: Window) -> Option<Vec<u32>> {
    let reply = conn
        .get_property(false, window, AtomEnum::WM_NORMAL_HINTS, AtomEnum::WM_SIZE_HINTS, 0, 18)
        .ok()?
        .reply()
        .ok()?;
    let values: Vec<u32> = reply.value32()?.collect();
    (!values.is_empty()).then_some(values)
}

pub const MAX_ICONS: usize = 64;

#[must_use]
pub const fn has_room(docked: usize) -> bool {
    docked < MAX_ICONS
}

#[must_use]
pub fn xembed_mapped(info: Option<&[u32]>) -> bool {
    info.and_then(|info| info.get(1)).is_none_or(|flags| flags & XEMBED_MAPPED != 0)
}

fn create_container(conn: &RustConnection, root: Window, visual: u32, atoms: &Atoms) -> anyhow::Result<Window> {
    let colormap = conn.generate_id()?;
    conn.create_colormap(ColormapAlloc::NONE, colormap, root, visual)?;
    let container = conn.generate_id()?;
    conn.create_window(
        32,
        container,
        root,
        0,
        0,
        ICON_SIZE,
        ICON_SIZE,
        0,
        WindowClass::INPUT_OUTPUT,
        visual,
        &CreateWindowAux::new()
            .background_pixel(0)
            .border_pixel(0)
            .colormap(colormap)
            .event_mask(EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT),
    )?;
    let class = format!("{CONTAINER_CLASS}\0{CONTAINER_CLASS}\0");
    conn.change_property8(PropMode::REPLACE, container, AtomEnum::WM_CLASS, AtomEnum::STRING, class.as_bytes())?;
    conn.change_property8(
        PropMode::REPLACE,
        container,
        atoms._NET_WM_NAME,
        atoms.UTF8_STRING,
        CONTAINER_CLASS.as_bytes(),
    )?;
    conn.change_property32(
        PropMode::REPLACE,
        container,
        atoms._NET_WM_WINDOW_TYPE,
        AtomEnum::ATOM,
        &[atoms._NET_WM_WINDOW_TYPE_NORMAL],
    )?;
    Ok(container)
}

fn create_owner(
    conn: &RustConnection, root: Window, visual: u32, selection: Atom, atoms: &Atoms,
) -> anyhow::Result<Window> {
    let owner = conn.generate_id()?;
    conn.create_window(
        0,
        owner,
        root,
        -1,
        -1,
        1,
        1,
        0,
        WindowClass::INPUT_ONLY,
        0,
        &CreateWindowAux::new().override_redirect(1).event_mask(EventMask::PROPERTY_CHANGE),
    )?;
    conn.change_property32(PropMode::REPLACE, owner, atoms._NET_SYSTEM_TRAY_ORIENTATION, AtomEnum::CARDINAL, &[0])?;
    conn.change_property32(PropMode::REPLACE, owner, atoms._NET_SYSTEM_TRAY_VISUAL, AtomEnum::VISUALID, &[visual])?;
    conn.change_property32(
        PropMode::REPLACE,
        owner,
        atoms._NET_SYSTEM_TRAY_ICON_SIZE,
        AtomEnum::CARDINAL,
        &[u32::from(ICON_SIZE)],
    )?;
    conn.set_selection_owner(owner, selection, CURRENT_TIME)?;
    if conn.get_selection_owner(selection)?.reply()?.owner != owner {
        bail!("could not take the X11 system tray selection");
    }
    let announce = ClientMessageEvent::new(32, root, atoms.MANAGER, [CURRENT_TIME, selection, owner, 0, 0]);
    conn.send_event(false, root, EventMask::STRUCTURE_NOTIFY, announce)?;
    Ok(owner)
}

impl Host {
    pub fn connect(
        display: Option<&str>, bus: Option<String>, control: zbus::blocking::Connection, registered: Registered,
    ) -> anyhow::Result<Self> {
        let (conn, screen_num) = x11rb::connect(display).context("no X display")?;
        let conn = Arc::new(conn);
        let screen = conn.setup().roots.get(screen_num).ok_or_else(|| anyhow!("no X screen {screen_num}"))?.clone();
        let root = screen.root;
        let atoms = Atoms::new(conn.as_ref())?.reply()?;
        let selection = conn.intern_atom(false, format!("_NET_SYSTEM_TRAY_S{screen_num}").as_bytes())?.reply()?.atom;
        let current = conn.get_selection_owner(selection)?.reply()?.owner;
        if current != NONE {
            bail!("another tray owns the X11 system tray");
        }
        conn.damage_query_version(1, 1)?.reply().context("the X server has no DAMAGE extension")?;
        conn.xfixes_query_version(5, 0)?.reply().context("the X server has no XFIXES extension")?;
        let has_shape = conn.shape_query_version().ok().and_then(|cookie| cookie.reply().ok()).is_some();

        let visual = argb_visual(&screen).ok_or_else(|| anyhow!("the X server offers no 32-bit visual"))?;
        let container = create_container(&conn, root, visual, &atoms)?;
        let owner = create_owner(&conn, root, visual, selection, &atoms)?;
        conn.flush()?;
        tracing::info!(screen = screen_num, "owning the X11 system tray");

        let clock = Arc::new(ServerClock::default());
        let last_clicked = Arc::new(AtomicU32::new(NONE));
        let clicker = Arc::new(XClicker {
            conn: Arc::clone(&conn),
            root,
            owner,
            container,
            xembed_atom: atoms._XEMBED,
            time_atom: atoms._GLIDE_XEMBED_TIME,
            clock: Arc::clone(&clock),
            last_clicked: Arc::clone(&last_clicked),
            serial: Mutex::new(()),
        });
        Ok(Self {
            conn,
            root,
            atoms,
            selection,
            owner,
            container,
            has_shape,
            clock,
            last_clicked,
            clicker,
            control,
            bus,
            registered,
            icons: Vec::new(),
            serial: 0,
        })
    }

    pub fn run(mut self) -> anyhow::Result<()> {
        loop {
            let event = self.conn.wait_for_event()?;
            self.handle(event)?;
            while let Some(event) = self.conn.poll_for_event()? {
                self.handle(event)?;
            }
            self.conn.flush()?;
        }
    }

    fn handle(&mut self, event: Event) -> anyhow::Result<()> {
        match event {
            Event::ClientMessage(message) => {
                let data = message.data.as_data32();
                if message.window == self.owner
                    && message.type_ == self.atoms._NET_SYSTEM_TRAY_OPCODE
                    && data[1] == SYSTEM_TRAY_REQUEST_DOCK
                {
                    self.dock(data[2]);
                } else if message.type_ == self.atoms._XEMBED && data[1] == XEMBED_REQUEST_FOCUS {
                    self.grant_focus(message.window, data[0])?;
                }
            }
            Event::SelectionClear(clear) if clear.selection == self.selection => {
                bail!("another tray took the X11 system tray");
            }
            Event::DestroyNotify(destroyed) => self.undock(destroyed.window),
            Event::ReparentNotify(moved) if moved.parent != self.container => self.undock(moved.window),
            Event::MapRequest(request) if self.position(request.window).is_some() => {
                self.conn.map_window(request.window)?;
            }
            Event::MapNotify(mapped) if mapped.event == mapped.window => {
                if let Some(at) = self.position(mapped.window) {
                    self.icons[at].mapped = true;
                    self.refresh(mapped.window);
                }
            }
            Event::UnmapNotify(unmapped) if unmapped.event == unmapped.window => {
                if let Some(at) = self.position(unmapped.window) {
                    self.hide(at);
                }
            }
            Event::PropertyNotify(changed) => {
                if changed.window == self.owner && changed.atom == self.atoms._GLIDE_XEMBED_TIME {
                    self.clock.deliver(changed.time);
                } else {
                    self.clock.note(changed.time);
                    if changed.atom == self.atoms._XEMBED_INFO {
                        self.sync_visibility(changed.window);
                    } else if changed.atom == u32::from(AtomEnum::WM_NORMAL_HINTS) {
                        self.apply_size_hints(changed.window)?;
                    } else if changed.atom == self.atoms._NET_WM_NAME || changed.atom == u32::from(AtomEnum::WM_NAME) {
                        self.refresh(changed.window);
                    }
                }
            }
            Event::ConfigureRequest(request) => {
                if let Some(at) = self.position(request.window) {
                    let x = i32::try_from(at * usize::from(ICON_SIZE)).unwrap_or(i32::MAX);
                    let mut aux = ConfigureWindowAux::new().x(x).y(0);
                    if request.value_mask.contains(ConfigWindow::WIDTH) {
                        aux = aux.width(u32::from(request.width.clamp(1, ICON_SIZE)));
                    }
                    if request.value_mask.contains(ConfigWindow::HEIGHT) {
                        aux = aux.height(u32::from(request.height.clamp(1, ICON_SIZE)));
                    }
                    self.conn.configure_window(request.window, &aux)?;
                }
            }
            Event::ConfigureNotify(configured) if self.position(configured.window).is_some() => {
                if configured.width == 0
                    || configured.height == 0
                    || configured.width > ICON_SIZE
                    || configured.height > ICON_SIZE
                {
                    let size = ConfigureWindowAux::new()
                        .width(u32::from(configured.width.clamp(1, ICON_SIZE)))
                        .height(u32::from(configured.height.clamp(1, ICON_SIZE)));
                    self.conn.configure_window(configured.window, &size)?;
                } else {
                    self.refresh(configured.window);
                }
            }
            Event::DamageNotify(notice) => {
                self.clock.note(notice.timestamp);
                self.conn.damage_subtract(notice.damage, NONE, NONE)?;
                if let Some(at) = self.position(notice.drawable) {
                    self.icons[at].damaged = true;
                    self.icons[at].mapped = true;
                }
                self.repainted(notice.drawable);
            }
            Event::Error(err) => tracing::debug!(?err, "X11 request failed"),
            _ => {}
        }
        Ok(())
    }

    fn position(&self, window: Window) -> Option<usize> {
        self.icons.iter().position(|icon| icon.window == window)
    }

    fn grant_focus(&self, source: Window, requested_time: u32) -> anyhow::Result<()> {
        let target = if self.position(source).is_some() {
            Some(source)
        } else {
            let last = self.last_clicked.load(Ordering::Relaxed);
            self.position(last).map(|_| last).or_else(|| self.icons.last().map(|icon| icon.window))
        };
        let Some(window) = target else {
            return Ok(());
        };
        let time = if requested_time == CURRENT_TIME { self.clock.current() } else { requested_time };
        send_xembed(&self.conn, self.atoms._XEMBED, window, time, XEMBED_WINDOW_ACTIVATE, 0, 0)?;
        send_xembed(&self.conn, self.atoms._XEMBED, window, time, XEMBED_FOCUS_IN, XEMBED_FOCUS_CURRENT, 0)?;
        let _ = self.conn.set_input_focus(InputFocus::PARENT, window, time);
        Ok(())
    }

    fn apply_size_hints(&self, window: Window) -> anyhow::Result<()> {
        if self.position(window).is_none() {
            return Ok(());
        }
        let hints = wm_normal_hints(&self.conn, window);
        let (width, height) = target_size(ICON_SIZE, hints.as_deref());
        let size = ConfigureWindowAux::new().width(u32::from(width)).height(u32::from(height));
        self.conn.configure_window(window, &size)?;
        Ok(())
    }

    fn xembed_info(&self, window: Window) -> Option<Vec<u32>> {
        let reply = self
            .conn
            .get_property(false, window, self.atoms._XEMBED_INFO, self.atoms._XEMBED_INFO, 0, 2)
            .ok()?
            .reply()
            .ok()?;
        let values: Vec<u32> = reply.value32()?.collect();
        (!values.is_empty()).then_some(values)
    }

    fn dock(&mut self, window: Window) {
        if window == NONE || self.position(window).is_some() {
            return;
        }
        if !has_room(self.icons.len()) {
            tracing::warn!(window, "the X11 tray is full; a tray icon was not embedded");
            return;
        }
        if let Err(err) = self.embed(window) {
            tracing::info!(window, "could not embed a tray icon: {err}");
        }
    }

    fn embed(&mut self, window: Window) -> anyhow::Result<()> {
        let events = EventMask::PROPERTY_CHANGE | EventMask::STRUCTURE_NOTIFY;
        self.conn.change_window_attributes(window, &ChangeWindowAttributesAux::new().event_mask(events))?.check()?;
        let info = self.xembed_info(window);
        let hints = wm_normal_hints(&self.conn, window);
        let (width, height) = target_size(ICON_SIZE, hints.as_deref());
        let x = i16::try_from(self.icons.len() * usize::from(ICON_SIZE)).unwrap_or(i16::MAX);
        self.conn.reparent_window(window, self.container, x, 0)?.check()?;
        let size = ConfigureWindowAux::new().width(u32::from(width)).height(u32::from(height));
        self.conn.configure_window(window, &size)?;
        let time = self.clock.current();
        send_xembed(&self.conn, self.atoms._XEMBED, window, time, XEMBED_EMBEDDED_NOTIFY, 0, self.container)?;
        send_xembed(&self.conn, self.atoms._XEMBED, window, time, XEMBED_WINDOW_ACTIVATE, 0, 0)?;
        if xembed_mapped(info.as_deref()) {
            self.conn.map_window(window)?;
        }
        let damage = self.conn.generate_id()?;
        self.conn.damage_create(damage, window, damage::ReportLevel::NON_EMPTY)?;
        self.serial += 1;
        let id = wm_class(&self.conn, window).unwrap_or_else(|| format!("xembed-{}", self.serial));
        self.icons.push(Docked {
            window,
            damage,
            mapped: false,
            damaged: false,
            look: Look { id, ..Look::default() },
            exported: None,
        });
        self.arrange()?;
        tracing::info!(window, "embedded an X11 tray icon");
        Ok(())
    }

    fn hide(&mut self, at: usize) {
        self.icons[at].mapped = false;
        if let Some(exported) = self.icons[at].exported.take() {
            self.forget(exported.name());
        }
    }

    fn undock(&mut self, window: Window) {
        let Some(at) = self.position(window) else {
            return;
        };
        let icon = self.icons.remove(at);
        let _ = self.conn.damage_destroy(icon.damage);
        if let Some(exported) = icon.exported {
            self.forget(exported.name());
        }
        tracing::info!(window, "an X11 tray icon went away");
        if let Err(err) = self.arrange() {
            tracing::debug!("could not rearrange the tray container: {err}");
        }
    }

    fn arrange(&self) -> anyhow::Result<()> {
        for (index, icon) in self.icons.iter().enumerate() {
            let x = i32::try_from(index * usize::from(ICON_SIZE)).unwrap_or(i32::MAX);
            self.conn.configure_window(icon.window, &ConfigureWindowAux::new().x(x).y(0))?;
        }
        if self.icons.is_empty() {
            self.conn.unmap_window(self.container)?;
            return Ok(());
        }
        let width = u32::try_from(self.icons.len()).unwrap_or(1) * u32::from(ICON_SIZE);
        self.conn.configure_window(self.container, &ConfigureWindowAux::new().width(width))?;
        self.conn.map_window(self.container)?;
        Ok(())
    }

    fn sync_visibility(&mut self, window: Window) {
        let Some(at) = self.position(window) else {
            return;
        };
        let mapped = xembed_mapped(self.xembed_info(window).as_deref());
        let result =
            if mapped { self.conn.map_window(window).map(drop) } else { self.conn.unmap_window(window).map(drop) };
        if let Err(err) = result {
            tracing::debug!(window, "could not follow the icon's XEMBED state: {err}");
        }
        if mapped {
            if self.icons[at].mapped {
                self.refresh(window);
            }
        } else {
            self.hide(at);
        }
    }

    fn capture(&self, window: Window) -> Option<(Icon, u8)> {
        let geom = self.conn.get_geometry(window).ok()?.reply().ok()?;
        let width = geom.width.min(MAX_CAPTURE_SIDE);
        let height = geom.height.min(MAX_CAPTURE_SIDE);
        if width == 0 || height == 0 {
            return None;
        }
        let reply =
            self.conn.get_image(ImageFormat::Z_PIXMAP, window, 0, 0, width, height, u32::MAX).ok()?.reply().ok()?;
        let mut icon = pixmap::from_zpixmap(&reply.data, width, height, reply.depth)?;
        if reply.depth != 32
            && self.has_shape
            && let Ok(cookie) = self.conn.shape_get_rectangles(window, shape::SK::BOUNDING)
            && let Ok(shape_reply) = cookie.reply()
        {
            let rects: Vec<(i16, i16, u16, u16)> =
                shape_reply.rectangles.into_iter().map(|rect| (rect.x, rect.y, rect.width, rect.height)).collect();
            icon.apply_shape_mask(&rects);
        }
        Some((icon.trim_to_square(), reply.depth))
    }

    fn repainted(&mut self, window: Window) {
        let Some(at) = self.position(window).filter(|&at| self.icons[at].exported.is_some()) else {
            self.refresh(window);
            return;
        };
        let damaged = self.icons[at].damaged;
        let known = &self.icons[at].look;
        let Some((icon, _)) = self.capture(window).filter(|(icon, depth)| {
            !icon.is_blank() && !icon.is_unpainted_opaque(*depth, damaged) && *icon != known.icon
        }) else {
            return;
        };
        let look = Look { icon, ..known.clone() };
        self.icons[at].look = look.clone();
        if let Some(exported) = self.icons[at].exported.as_ref()
            && let Err(err) = exported.update(look)
        {
            tracing::debug!(window, "could not announce a tray icon change: {err}");
        }
    }

    fn refresh(&mut self, window: Window) {
        let Some(at) = self.position(window) else {
            return;
        };
        if (!self.icons[at].mapped && !self.icons[at].damaged) || !xembed_mapped(self.xembed_info(window).as_deref()) {
            return;
        }
        let title = text_property(&self.conn, window, self.atoms._NET_WM_NAME, self.atoms.UTF8_STRING)
            .or_else(|| text_property(&self.conn, window, AtomEnum::WM_NAME.into(), AtomEnum::STRING.into()))
            .unwrap_or_default();
        let damaged = self.icons[at].damaged;
        let icon = self
            .capture(window)
            .filter(|(icon, depth)| !icon.is_blank() && !icon.is_unpainted_opaque(*depth, damaged))
            .map_or_else(|| self.icons[at].look.icon.clone(), |(icon, _)| icon);
        let look = Look { id: self.icons[at].look.id.clone(), title, icon };
        if look.icon.argb.is_empty() {
            return;
        }
        if self.icons[at].exported.is_none() {
            self.export(at, look);
            return;
        }
        if self.icons[at].look == look {
            return;
        }
        self.icons[at].look = look.clone();
        if let Some(exported) = self.icons[at].exported.as_ref()
            && let Err(err) = exported.update(look)
        {
            tracing::debug!(window, "could not announce a tray icon change: {err}");
        }
    }

    fn export(&mut self, at: usize, look: Look) {
        self.serial += 1;
        let name = format!("org.kde.StatusNotifierItem-{}-{}", std::process::id(), self.serial);
        let clicker: Arc<dyn Clicker> = Arc::clone(&self.clicker) as Arc<dyn Clicker>;
        let window = self.icons[at].window;
        let builder = match self.bus.as_deref() {
            Some(address) => zbus::blocking::connection::Builder::address(address),
            None => zbus::blocking::connection::Builder::session(),
        };
        let exported = builder.and_then(|builder| Exported::on(builder, name.clone(), window, look.clone(), clicker));
        match exported {
            Ok(exported) => {
                self.icons[at].look = look;
                self.icons[at].exported = Some(exported);
                self.registered.lock().unwrap_or_else(PoisonError::into_inner).push(name.clone());
                if let Err(err) = sni::register(&self.control, &name) {
                    tracing::info!(name, "no StatusNotifierWatcher yet: {err}");
                }
            }
            Err(err) => tracing::warn!(window, "could not export a tray icon: {err}"),
        }
    }

    fn forget(&self, name: &str) {
        self.registered.lock().unwrap_or_else(PoisonError::into_inner).retain(|known| known != name);
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        for icon in self.icons.drain(..) {
            let _ = self.conn.reparent_window(icon.window, self.root, 0, 0);
            if let Some(exported) = icon.exported {
                self.registered.lock().unwrap_or_else(PoisonError::into_inner).retain(|known| known != exported.name());
            }
        }
        let _ = self.conn.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_icon_is_shown_unless_its_xembed_info_says_it_is_unmapped() {
        assert!(xembed_mapped(None), "no _XEMBED_INFO: map it, as every tray does");
        assert!(xembed_mapped(Some(&[0, 1])));
        assert!(!xembed_mapped(Some(&[0, 0])));
        assert!(xembed_mapped(Some(&[0])), "a truncated property is not a request to hide");
    }

    #[test]
    fn the_tray_takes_a_bounded_number_of_icons() {
        assert!(has_room(0) && has_room(MAX_ICONS - 1));
        assert!(!has_room(MAX_ICONS), "each icon is a bus connection and a slot in the top bar");
    }

    #[test]
    fn a_horizontal_wheel_release_holds_no_button_because_x_has_no_mask_for_buttons_six_and_seven() {
        assert_eq!(held_mask(Click::ScrollDown.button()), KeyButMask::BUTTON5);
        assert_eq!(held_mask(Click::ScrollLeft.button()), KeyButMask::default());
        assert_eq!(held_mask(Click::ScrollRight.button()), KeyButMask::default());
    }

    #[test]
    fn wm_normal_hints_cap_or_raise_target_size_within_icon_slot() {
        assert_eq!(target_size(48, None), (48, 48));
        let mut fixed = [0u32; 18];
        fixed[0] = WM_SIZE_P_MIN_SIZE | WM_SIZE_P_MAX_SIZE;
        fixed[5] = 24;
        fixed[6] = 24;
        fixed[7] = 24;
        fixed[8] = 24;
        assert_eq!(target_size(48, Some(&fixed)), (24, 24));

        let mut huge = [0u32; 18];
        huge[0] = WM_SIZE_P_MIN_SIZE;
        huge[5] = 200;
        huge[6] = 200;
        assert_eq!(target_size(48, Some(&huge)), (48, 48));
    }

    #[test]
    fn root_click_coordinates_reject_zero_and_compute_container_origin() {
        assert_eq!(valid_root_point(None), None);
        assert_eq!(valid_root_point(Some((0, 0))), None);
        assert_eq!(valid_root_point(Some((1800, 14))), Some((1800, 14)));
        assert_eq!(valid_root_point(Some((i32::MAX, 14))), None);
        assert_eq!(container_origin((1800, 14), (48, 0), (24, 24)), (1728, -10));
    }
}
