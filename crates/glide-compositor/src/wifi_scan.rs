use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Link {
    #[default]
    Off,
    Disconnected,
    Connected,
}

impl Link {
    #[must_use]
    pub fn of(status: &crate::system_info::SystemStatus) -> Self {
        if !status.wifi_enabled || status.airplane || !status.wifi_available {
            Self::Off
        } else if !status.wifi_ssid.is_empty() {
            Self::Connected
        } else if status.ethernet_connected {
            Self::Off
        } else {
            Self::Disconnected
        }
    }
}

pub const BURST: Duration = Duration::from_mins(1);

#[derive(Debug, Default)]
pub struct Scanner {
    link: Link,
    on_battery: bool,
    last: Option<Instant>,
    burst_from: Option<Instant>,
}

impl Scanner {
    pub fn set_link(&mut self, link: Link, now: Instant) {
        if link == Link::Disconnected && self.link != Link::Disconnected {
            self.burst_from = Some(now);
        }
        self.link = link;
    }

    pub const fn set_on_battery(&mut self, on_battery: bool) {
        self.on_battery = on_battery;
    }

    pub fn wake(&mut self, now: Instant) {
        self.burst_from = Some(now);
        self.last = None;
    }

    #[must_use]
    pub fn interval(&self, now: Instant) -> Option<Duration> {
        let bursting = self.burst_from.is_some_and(|from| now.saturating_duration_since(from) < BURST);
        let secs = match (self.link, bursting, self.on_battery) {
            (Link::Off, ..) => return None,
            (Link::Disconnected, true, false) => 4,
            (Link::Disconnected, true, true) => 10,
            (Link::Disconnected, false, false) => 15,
            (Link::Connected, _, false) => 20,
            (_, _, true) => 60,
        };
        Some(Duration::from_secs(secs))
    }

    pub fn due(&mut self, now: Instant) -> Option<Background> {
        let every = self.interval(now)?;
        if self.last.is_some_and(|last| now.saturating_duration_since(last) < every) {
            return None;
        }
        self.last = Some(now);
        Some(if self.link == Link::Connected { Background::WhenIdle } else { Background::Now })
    }

    pub fn scanned(&mut self, now: Instant) {
        self.last = Some(now);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Background {
    Now,
    WhenIdle,
}

pub const IDLE_SAMPLE: Duration = Duration::from_secs(2);
pub const BUSY_BYTES_PER_SEC: u64 = 32 * 1024;

#[must_use]
pub fn busy(before: u64, after: u64, over: Duration) -> bool {
    let secs = over.as_secs_f64().max(0.001);
    after.saturating_sub(before) as f64 / secs >= BUSY_BYTES_PER_SEC as f64
}

#[must_use]
pub fn wireless_interface() -> Option<String> {
    std::fs::read_dir("/sys/class/net").ok()?.flatten().find_map(|entry| {
        entry.path().join("wireless").is_dir().then(|| entry.file_name().to_string_lossy().into_owned())
    })
}

#[must_use]
pub fn traffic_bytes(interface: &str) -> Option<u64> {
    let read = |name: &str| -> Option<u64> {
        std::fs::read_to_string(format!("/sys/class/net/{interface}/statistics/{name}")).ok()?.trim().parse().ok()
    };
    Some(read("rx_bytes")? + read("tx_bytes")?)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Known {
    pub uuid: String,
    pub ssid: String,
    pub autoconnect: bool,
    pub priority: i32,
    pub active: bool,
}

pub const SWITCH_SIGNAL: u8 = 60;
pub const HAND_PICK_HOLD: Duration = Duration::from_mins(5);
pub const FAILURE_HOLD: Duration = Duration::from_mins(10);

#[derive(Debug, Default)]
pub struct Switcher {
    pending: Option<String>,
    hand_picked: Option<Instant>,
    failed: std::collections::HashMap<String, Instant>,
    metered: std::collections::HashSet<String>,
}

impl Switcher {
    pub fn hand_picked(&mut self, now: Instant) {
        self.hand_picked = Some(now);
        self.pending = None;
    }

    pub fn failed(&mut self, uuid: &str, now: Instant) {
        self.failed.insert(uuid.to_string(), now);
        self.pending = None;
    }

    pub fn decide(
        &mut self, known: &[Known], visible: &[crate::qs_net::WifiNetwork], metered: bool, now: Instant,
    ) -> Option<Known> {
        let current = known.iter().find(|profile| profile.active)?;
        if metered {
            self.metered.insert(current.uuid.clone());
        }
        let held = self.hand_picked.is_some_and(|at| now.saturating_duration_since(at) < HAND_PICK_HOLD);
        self.failed.retain(|_, at| now.saturating_duration_since(*at) < FAILURE_HOLD);
        let signal =
            |ssid: &str| visible.iter().find(|network| network.ssid == ssid).map_or(0, |network| network.signal);
        let best = known
            .iter()
            .filter(|profile| profile.autoconnect && !profile.active && profile.ssid != current.ssid)
            .filter(|profile| !self.metered.contains(&profile.uuid) && !self.failed.contains_key(&profile.uuid))
            .filter(|profile| signal(&profile.ssid) >= SWITCH_SIGNAL)
            .filter(|profile| profile.priority > current.priority || (metered && profile.priority == current.priority))
            .max_by_key(|profile| (profile.priority, signal(&profile.ssid)))
            .filter(|_| !held);
        let Some(best) = best else {
            self.pending = None;
            return None;
        };
        if self.pending.as_deref() == Some(&best.uuid) {
            self.pending = None;
            return Some(best.clone());
        }
        self.pending = Some(best.uuid.clone());
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(ssid: &str, priority: i32, active: bool) -> Known {
        Known { uuid: format!("{ssid}-uuid"), ssid: ssid.into(), autoconnect: true, priority, active }
    }

    fn seen(ssid: &str, signal: u8) -> crate::qs_net::WifiNetwork {
        crate::qs_net::WifiNetwork {
            ssid: ssid.into(),
            security: crate::qs_net::Security::Passphrase,
            signal,
            connected: false,
            saved: true,
        }
    }

    #[test]
    fn a_metered_hotspot_gives_way_to_a_strong_home_network_after_two_scans() {
        let start = Instant::now();
        let mut switcher = Switcher::default();
        let profiles = [known("Pixel", 0, true), known("Home", 0, false)];
        let visible = [seen("Pixel", 90), seen("Home", 75)];
        assert_eq!(switcher.decide(&profiles, &visible, true, start), None, "one sighting is not enough");
        let picked = switcher.decide(&profiles, &visible, true, at(start, 20)).expect("switches on the second");
        assert_eq!(picked.ssid, "Home");
        assert_eq!(switcher.decide(&profiles, &visible, false, at(start, 40)), None, "an unmetered link stays put");
    }

    #[test]
    fn weak_failed_hand_picked_and_metered_candidates_are_left_alone() {
        let start = Instant::now();
        let profiles = [known("Pixel", 0, true), known("Home", 0, false)];
        let weak = [seen("Home", 40)];
        let mut switcher = Switcher::default();
        for step in 0..3 {
            assert_eq!(switcher.decide(&profiles, &weak, true, at(start, step * 20)), None, "too weak");
        }
        let strong = [seen("Home", 80)];
        let mut switcher = Switcher::default();
        switcher.hand_picked(start);
        for step in 0..3 {
            assert_eq!(switcher.decide(&profiles, &strong, true, at(start, step * 20)), None, "the user chose it");
        }
        assert_eq!(switcher.decide(&profiles, &strong, true, at(start, 301)), None);
        assert!(switcher.decide(&profiles, &strong, true, at(start, 321)).is_some(), "five minutes later it may");
        let mut switcher = Switcher::default();
        switcher.failed("Home-uuid", start);
        assert_eq!(switcher.decide(&profiles, &strong, true, at(start, 20)), None);
        assert_eq!(switcher.decide(&profiles, &strong, true, at(start, 40)), None, "a failure holds it back");
        let mut switcher = Switcher::default();
        let flipped = [known("Pixel", 0, false), known("Home", 0, true)];
        switcher.decide(&flipped, &[seen("Pixel", 90)], true, start);
        assert_eq!(switcher.decide(&profiles, &strong, true, at(start, 20)), None);
        assert_eq!(
            switcher.decide(&profiles, &[seen("Home", 90)], true, at(start, 40)),
            None,
            "a network once seen metered is never a way out of another metered one"
        );
    }

    #[test]
    fn a_higher_priority_wins_even_on_an_unmetered_link() {
        let start = Instant::now();
        let mut switcher = Switcher::default();
        let profiles = [known("Cafe", 0, true), known("Office", 10, false), known("Lower", -5, false)];
        let visible = [seen("Office", 70), seen("Lower", 99)];
        switcher.decide(&profiles, &visible, false, start);
        assert_eq!(switcher.decide(&profiles, &visible, false, at(start, 20)).map(|k| k.ssid), Some("Office".into()));
        let mut switcher = Switcher::default();
        let off = [known("Cafe", 0, true), Known { autoconnect: false, ..known("Office", 10, false) }];
        switcher.decide(&off, &visible, false, start);
        assert_eq!(switcher.decide(&off, &visible, false, at(start, 20)), None, "autoconnect off means hands off");
    }

    fn at(start: Instant, secs: u64) -> Instant {
        start + Duration::from_secs(secs)
    }

    #[test]
    fn the_link_follows_the_radio_the_network_and_the_cable() {
        let mut status = crate::system_info::SystemStatus::default();
        assert_eq!(Link::of(&status), Link::Off);
        status.wifi_enabled = true;
        assert_eq!(Link::of(&status), Link::Disconnected);
        status.ethernet_connected = true;
        assert_eq!(Link::of(&status), Link::Off, "a cable needs no wi-fi to fall back on");
        status.wifi_ssid = "Home".into();
        assert_eq!(Link::of(&status), Link::Connected);
        status.airplane = true;
        assert_eq!(Link::of(&status), Link::Off);
    }

    #[test]
    fn nothing_is_scanned_with_the_radio_off() {
        let mut scanner = Scanner::default();
        assert_eq!(scanner.due(Instant::now()), None);
    }

    #[test]
    fn losing_the_network_scans_often_for_a_minute_then_settles() {
        let start = Instant::now();
        let mut scanner = Scanner::default();
        scanner.set_link(Link::Connected, start);
        assert_eq!(scanner.due(start), Some(Background::WhenIdle), "connected scans wait for a quiet link");
        scanner.set_link(Link::Disconnected, at(start, 1));
        assert_eq!(scanner.interval(at(start, 1)), Some(Duration::from_secs(4)));
        assert_eq!(scanner.due(at(start, 21)), Some(Background::Now));
        assert_eq!(scanner.due(at(start, 23)), None);
        assert_eq!(scanner.due(at(start, 25)), Some(Background::Now));
        assert_eq!(scanner.interval(at(start, 62)), Some(Duration::from_secs(15)), "the burst lasts a minute");
        scanner.set_link(Link::Disconnected, at(start, 70));
        assert_eq!(scanner.interval(at(start, 70)), Some(Duration::from_secs(15)), "staying offline is no new burst");
    }

    #[test]
    fn battery_stretches_every_interval() {
        let start = Instant::now();
        let mut scanner = Scanner::default();
        scanner.set_on_battery(true);
        scanner.set_link(Link::Disconnected, start);
        assert_eq!(scanner.interval(start), Some(Duration::from_secs(10)));
        assert_eq!(scanner.interval(at(start, 61)), Some(Duration::from_mins(1)));
        scanner.set_link(Link::Connected, at(start, 61));
        assert_eq!(scanner.interval(at(start, 61)), Some(Duration::from_mins(1)));
        scanner.set_on_battery(false);
        assert_eq!(scanner.interval(at(start, 61)), Some(Duration::from_secs(20)), "plugging in applies at once");
    }

    #[test]
    fn waking_up_scans_at_once_and_bursts() {
        let start = Instant::now();
        let mut scanner = Scanner::default();
        scanner.set_link(Link::Disconnected, start);
        assert!(scanner.due(at(start, 100)).is_some());
        scanner.wake(at(start, 101));
        assert_eq!(scanner.due(at(start, 101)), Some(Background::Now));
        assert_eq!(scanner.interval(at(start, 101)), Some(Duration::from_secs(4)));
    }

    #[test]
    fn a_page_scan_restarts_the_background_clock() {
        let start = Instant::now();
        let mut scanner = Scanner::default();
        scanner.set_link(Link::Connected, start);
        scanner.scanned(at(start, 5));
        assert_eq!(scanner.due(at(start, 20)), None);
        assert!(scanner.due(at(start, 25)).is_some());
    }

    #[test]
    fn a_call_counts_as_traffic_and_idle_chatter_does_not() {
        assert!(busy(0, 200 * 1024, IDLE_SAMPLE), "a video call");
        assert!(!busy(0, 8 * 1024, IDLE_SAMPLE), "background chatter");
        assert!(!busy(500, 100, IDLE_SAMPLE), "a counter reset is no traffic");
    }
}
