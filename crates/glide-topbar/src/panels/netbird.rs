use std::time::SystemTime;

use glide_netbird::{self as nb, Remaining, State};

use super::super::Topbar;
use super::{Key, t};

#[derive(Debug)]
pub enum NetbirdUpdate {
    Status(Box<nb::Status>),
    Networks(Vec<nb::Network>),
    Profiles(Vec<nb::Profile>),
    Config(Box<nb::Config>),
    Features(nb::Features),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetbirdPending {
    Connect,
    Disconnect,
    Profile(String),
    Exit(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Summary {
    state: State,
    ip: String,
    peers: (usize, usize),
    session: Option<SystemTime>,
    management_error: String,
}

#[derive(Debug, Default)]
pub struct NetbirdView {
    pub(crate) installed: bool,
    status: Option<nb::Status>,
    profiles: Vec<nb::Profile>,
    networks: Vec<nb::Network>,
    config: Option<nb::Config>,
    features: nb::Features,
    pending: Option<(NetbirdPending, Option<State>)>,
    session: Option<Remaining>,
}

fn summary(status: &nb::Status) -> Summary {
    Summary {
        state: status.state,
        ip: status.local.ip.clone(),
        peers: (status.connected_peers(), status.peers.len()),
        session: status.session_expires_at,
        management_error: status.management.error.clone(),
    }
}

impl NetbirdView {
    fn state(&self) -> Option<State> {
        self.status.as_ref().map(|status| status.state)
    }

    fn session_left(&self, now: SystemTime) -> Option<Remaining> {
        let status = self.status.as_ref().filter(|status| status.state.is_connected())?;
        status.session_expires_at.map(|deadline| nb::remaining_until(deadline, now))
    }

    fn pending(&self) -> Option<&NetbirdPending> {
        self.pending.as_ref().map(|(pending, _)| pending)
    }

    fn apply(&mut self, update: NetbirdUpdate, now: SystemTime) -> bool {
        match update {
            NetbirdUpdate::Status(status) => {
                let before = self.status.as_ref().map(summary);
                let after = summary(&status);
                let settled = matches!(
                    &self.pending,
                    Some((NetbirdPending::Connect | NetbirdPending::Disconnect, from)) if *from != Some(status.state)
                );
                if settled {
                    self.pending = None;
                }
                self.status = Some(*status);
                self.session = self.session_left(now);
                settled || before.as_ref() != Some(&after)
            }
            NetbirdUpdate::Networks(networks) => {
                if matches!(self.pending, Some((NetbirdPending::Exit(_), _))) {
                    self.pending = None;
                } else if self.networks == networks {
                    return false;
                }
                self.networks = networks;
                true
            }
            NetbirdUpdate::Profiles(profiles) => {
                if matches!(self.pending, Some((NetbirdPending::Profile(_), _))) {
                    self.pending = None;
                } else if self.profiles == profiles {
                    return false;
                }
                self.profiles = profiles;
                true
            }
            NetbirdUpdate::Config(config) => {
                self.config = Some(*config);
                false
            }
            NetbirdUpdate::Features(features) => {
                let changed = self.features != features;
                self.features = features;
                changed
            }
        }
    }

    pub(crate) fn tile(&self) -> (bool, &'static str) {
        match (self.pending(), self.state()) {
            (Some(NetbirdPending::Connect), _) => (false, t(Key::QsRowConnecting)),
            (Some(NetbirdPending::Disconnect), _) => (true, t(Key::QsRowDisconnecting)),
            (_, Some(State::Connected)) => (true, "NetBird"),
            (_, state) => (false, state_short(state)),
        }
    }

    pub(crate) fn row(&self) -> (bool, bool, String) {
        let peers = |status: &nb::Status| {
            t(Key::QsNetbirdPeers)
                .replace("{n}", &status.connected_peers().to_string())
                .replace("{total}", &status.peers.len().to_string())
        };
        match (self.pending(), self.status.as_ref()) {
            (Some(NetbirdPending::Connect), _) => (false, true, t(Key::QsRowConnecting).to_string()),
            (Some(NetbirdPending::Disconnect), _) => (true, true, t(Key::QsRowDisconnecting).to_string()),
            (_, Some(status)) if status.state.is_connected() => {
                (true, false, format!("{} \u{b7} {}", t(Key::QsRowConnected), peers(status)))
            }
            (_, Some(status)) if status.state == State::Connecting => {
                (false, true, t(Key::QsRowConnecting).to_string())
            }
            (_, status) => (false, false, state_short(status.map(|status| status.state)).to_string()),
        }
    }
}

fn state_short(state: Option<State>) -> &'static str {
    let key = match state {
        Some(State::Connected) => Key::QsRowConnected,
        Some(State::Connecting) => Key::QsRowConnecting,
        Some(State::NeedsLogin) => Key::NbLoginRequired,
        Some(State::LoginFailed) => Key::NbStateLoginFailed,
        Some(State::SessionExpired) => Key::NbSessionExpired,
        Some(State::Unavailable) => Key::NbNotRunning,
        Some(State::Idle) | None => Key::QsOffLong,
    };
    t(key)
}

impl Topbar {
    pub fn set_netbird_installed(&mut self, installed: bool) {
        if self.qs_panel.netbird.installed != installed {
            self.qs_panel.netbird.installed = installed;
            self.qs_panel.bump();
            self.touch();
        }
    }

    #[must_use]
    pub fn netbird_state(&self) -> Option<State> {
        self.qs_panel.netbird.state()
    }

    #[must_use]
    pub fn netbird_profile_name(&self, id: &str) -> Option<String> {
        self.qs_panel.netbird.profiles.iter().find(|profile| profile.id == id).map(|profile| profile.name.clone())
    }

    pub fn begin_netbird(&mut self, pending: NetbirdPending) {
        let state = self.netbird_state();
        self.qs_panel.netbird.pending = Some((pending, state));
        self.qs_panel.bump();
        self.touch();
    }

    pub fn fail_netbird(&mut self) {
        if self.qs_panel.netbird.pending.take().is_some() {
            self.qs_panel.bump();
            self.touch();
        }
    }

    pub fn apply_netbird(&mut self, update: NetbirdUpdate) -> bool {
        if !self.qs_panel.netbird.apply(update, SystemTime::now()) {
            return false;
        }
        self.qs_panel.bump();
        self.touch();
        true
    }

    pub fn tick_netbird(&mut self, now: SystemTime) -> bool {
        let view = &mut self.qs_panel.netbird;
        let left = view.session_left(now);
        if left == view.session {
            return false;
        }
        view.session = left;
        self.qs_panel.bump();
        self.touch();
        true
    }
}

#[must_use]
pub fn remaining_text(remaining: Remaining) -> String {
    let count = |key: Key, n: u64| t(key).replace("{n}", &n.to_string());
    match remaining {
        Remaining::Expired => t(Key::NbSessionExpired).to_string(),
        Remaining::LessThanMinute => t(Key::NbLessThanMinute).to_string(),
        Remaining::Minutes(n) => count(Key::NbMinutes, n),
        Remaining::Hours(n) => count(Key::NbHours, n),
        Remaining::Days(n) => count(Key::NbDays, n),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn status(state: State) -> nb::Status {
        nb::Status { state, ..nb::Status::default() }
    }

    #[test]
    fn remaining_session_time_fills_in_the_count_of_the_largest_unit() {
        for (remaining, count) in
            [(Remaining::Hours(2), "2"), (Remaining::Minutes(15), "15"), (Remaining::Days(3), "3")]
        {
            let text = remaining_text(remaining);
            assert!(text.contains(count) && !text.contains('{'), "{text}");
        }
        assert_eq!(remaining_text(Remaining::LessThanMinute), t(Key::NbLessThanMinute));
    }

    fn view_with(state: State) -> NetbirdView {
        let mut view = NetbirdView { installed: true, ..NetbirdView::default() };
        view.apply(NetbirdUpdate::Status(Box::new(status(state))), SystemTime::now());
        view
    }

    #[test]
    fn a_pending_connect_settles_when_the_daemon_reports_another_state() {
        let mut view = view_with(State::Idle);
        view.pending = Some((NetbirdPending::Connect, Some(State::Idle)));
        assert_eq!(view.tile().1, t(Key::QsRowConnecting));
        assert!(view.row().1, "the row spins while it connects");
        assert!(!view.apply(NetbirdUpdate::Status(Box::new(status(State::Idle))), SystemTime::now()));
        assert!(view.pending.is_some(), "the same state is not an answer");
        assert!(view.apply(NetbirdUpdate::Status(Box::new(status(State::Connecting))), SystemTime::now()));
        assert!(view.pending.is_none());
    }

    #[test]
    fn only_visible_changes_repaint_the_panel() {
        let mut view = view_with(State::Connected);
        let mut busy = status(State::Connected);
        busy.events.push(nb::Event::default());
        assert!(!view.apply(NetbirdUpdate::Status(Box::new(busy)), SystemTime::now()), "events are not drawn");
        let mut peer = status(State::Connected);
        peer.peers.push(nb::Peer::default());
        assert!(view.apply(NetbirdUpdate::Status(Box::new(peer)), SystemTime::now()));
        assert!(!view.apply(NetbirdUpdate::Config(Box::default()), SystemTime::now()));
    }

    #[test]
    fn the_row_counts_the_peers_while_connected_and_names_the_state_otherwise() {
        let mut connected = status(State::Connected);
        connected.peers =
            vec![nb::Peer { status: nb::PeerStatus::Connected, ..nb::Peer::default() }, nb::Peer::default()];
        let mut view = NetbirdView::default();
        view.apply(NetbirdUpdate::Status(Box::new(connected)), SystemTime::now());
        let (on, busy, line) = view.row();
        assert!(on && !busy);
        assert!(line.contains('1') && line.contains('2'), "{line}");
        assert_eq!(view.tile(), (true, "NetBird"));
        let login = view_with(State::NeedsLogin);
        assert_eq!(login.row(), (false, false, t(Key::NbLoginRequired).to_string()));
    }

    #[test]
    fn the_session_countdown_follows_the_connection() {
        let now = SystemTime::now();
        let mut connected = status(State::Connected);
        connected.session_expires_at = Some(now + Duration::from_hours(2));
        let mut view = NetbirdView::default();
        view.apply(NetbirdUpdate::Status(Box::new(connected.clone())), now);
        assert_eq!(view.session, Some(Remaining::Hours(2)));
        assert_eq!(view.session_left(now + Duration::from_hours(3)), Some(Remaining::Expired));
        let idle = nb::Status { state: State::Idle, ..connected };
        view.apply(NetbirdUpdate::Status(Box::new(idle)), now);
        assert_eq!(view.session, None);
    }
}
