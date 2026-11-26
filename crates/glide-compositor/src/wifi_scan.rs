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

    pub fn run(&mut self, now: Instant, start: impl FnOnce(Background) -> bool) -> bool {
        let Some(every) = self.interval(now) else {
            return false;
        };
        let kind = if self.link == Link::Connected { Background::WhenIdle } else { Background::Now };
        Cadence::run_on(&mut self.last, now, every, || start(kind))
    }

    pub fn scanned(&mut self, now: Instant) {
        self.last = Some(now);
    }
}

#[derive(Debug, Default)]
pub struct Cadence {
    last: Option<Instant>,
}

impl Cadence {
    pub const fn reset(&mut self) {
        self.last = None;
    }

    pub fn stamp(&mut self, now: Instant) {
        self.last = Some(now);
    }

    pub fn run(&mut self, now: Instant, every: Duration, start: impl FnOnce() -> bool) -> bool {
        Self::run_on(&mut self.last, now, every, start)
    }

    fn run_on(last: &mut Option<Instant>, now: Instant, every: Duration, start: impl FnOnce() -> bool) -> bool {
        if last.is_some_and(|last| now.saturating_duration_since(last) < every) {
            return false;
        }
        let started = start();
        if started {
            *last = Some(now);
        }
        started
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Background {
    Now,
    WhenIdle,
}

pub const IDLE_SAMPLE: Duration = Duration::from_secs(2);
pub const BUSY_BYTES_PER_SEC: u64 = 32 * 1024;
pub const BUSY_PACKETS_PER_SEC: u64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Traffic {
    pub bytes: u64,
    pub packets: u64,
}

#[must_use]
pub fn busy(before: Traffic, after: Traffic, over: Duration) -> bool {
    let secs = over.as_secs_f64().max(0.001);
    let rate = |a: u64, b: u64| b.saturating_sub(a) as f64 / secs;
    rate(before.bytes, after.bytes) >= BUSY_BYTES_PER_SEC as f64
        || rate(before.packets, after.packets) >= BUSY_PACKETS_PER_SEC as f64
}

#[must_use]
pub fn wireless_interface() -> Option<String> {
    std::fs::read_dir("/sys/class/net").ok()?.flatten().find_map(|entry| {
        entry.path().join("wireless").is_dir().then(|| entry.file_name().to_string_lossy().into_owned())
    })
}

#[must_use]
pub fn traffic(interface: &str) -> Option<Traffic> {
    let read = |name: &str| -> Option<u64> {
        std::fs::read_to_string(format!("/sys/class/net/{interface}/statistics/{name}")).ok()?.trim().parse().ok()
    };
    Some(Traffic { bytes: read("rx_bytes")? + read("tx_bytes")?, packets: read("rx_packets")? + read("tx_packets")? })
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
pub const FAILURE_HOLD: Duration = Duration::from_mins(10);

#[must_use]
pub fn may_switch(known: &[Known], metered: bool) -> bool {
    known.iter().find(|profile| profile.active).is_some_and(|current| {
        metered || known.iter().any(|profile| profile.autoconnect && profile.priority > current.priority)
    })
}

#[derive(Debug, Default)]
pub struct Switcher {
    pending: Option<String>,
    held: Option<String>,
    left: Option<String>,
    failed: std::collections::HashMap<String, Instant>,
    metered: std::collections::HashSet<String>,
}

impl Switcher {
    pub fn hand_picked(&mut self, ssid: Option<&str>) {
        self.held = ssid.map(str::to_string);
        self.left = None;
        self.pending = None;
    }

    pub fn failed(&mut self, uuid: &str, now: Instant) {
        self.failed.insert(uuid.to_string(), now);
        self.pending = None;
    }

    pub fn switched_away(&mut self, uuid: &str) {
        self.left = Some(uuid.to_string());
        self.held = None;
    }

    pub fn decide(
        &mut self, known: &[Known], visible: &[crate::qs_net::WifiNetwork], metered: bool, now: Instant,
    ) -> Option<Known> {
        let current = known.iter().find(|profile| profile.active)?;
        if metered {
            self.metered.insert(current.uuid.clone());
        }
        if self.left.as_deref() == Some(&current.uuid) {
            self.held = Some(current.ssid.clone());
            self.left = None;
        }
        if self.held.as_ref().is_some_and(|held| *held != current.ssid) {
            self.held = None;
        }
        let held = self.held.is_some();
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
        switcher.hand_picked(Some("Pixel"));
        for step in 0..30 {
            assert_eq!(switcher.decide(&profiles, &strong, true, at(start, step * 20)), None, "the user chose it");
        }
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

    fn due(scanner: &mut Scanner, now: Instant) -> Option<Background> {
        let mut kind = None;
        scanner.run(now, |started| {
            kind = Some(started);
            true
        });
        kind
    }

    #[test]
    fn a_scan_refused_while_another_runs_is_retried_on_the_next_tick_not_a_whole_interval_later() {
        let start = Instant::now();
        let mut cadence = Cadence::default();
        let every = Duration::from_secs(4);
        assert!(cadence.run(start, every, || true));
        assert!(!cadence.run(at(start, 4), every, || false), "the worker still holds the last scan");
        assert!(cadence.run(at(start, 5), every, || true), "one tick later, not at 8 s");
        assert!(!cadence.run(at(start, 8), every, || true));
        cadence.reset();
        assert!(cadence.run(at(start, 8), every, || true), "a reset page starts at once");
    }

    #[test]
    fn connected_scans_only_run_when_a_switch_could_follow() {
        assert!(!may_switch(&[known("Home", 0, true), known("Cafe", 0, false)], false), "all equal and unmetered");
        assert!(may_switch(&[known("Pixel", 0, true), known("Home", 0, false)], true), "a metered link");
        assert!(may_switch(&[known("Cafe", 0, true), known("Office", 5, false)], false), "a preferred one exists");
        assert!(!may_switch(&[known("Cafe", 0, false)], true), "nothing is connected");
    }

    #[test]
    fn going_back_to_the_network_the_shell_left_keeps_it_there() {
        let start = Instant::now();
        let mut switcher = Switcher::default();
        let on_pixel = [known("Pixel", 0, true), known("Home", 0, false)];
        let visible = [seen("Home", 90)];
        switcher.switched_away("Pixel-uuid");
        for step in 0..4 {
            assert_eq!(switcher.decide(&on_pixel, &visible, true, at(start, step * 20)), None, "{step}");
        }
        let on_home = [known("Pixel", 0, false), known("Home", 0, true)];
        assert_eq!(switcher.decide(&on_home, &[], false, at(start, 100)), None);
        assert_eq!(switcher.decide(&on_pixel, &visible, true, at(start, 120)), None, "the hold ends once it is left");
        assert!(
            switcher.decide(&on_pixel, &visible, true, at(start, 140)).is_some(),
            "a later, separate visit to the hotspot is switched again once Home is seen twice"
        );
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
        assert_eq!(due(&mut scanner, Instant::now()), None);
    }

    #[test]
    fn losing_the_network_scans_often_for_a_minute_then_settles() {
        let start = Instant::now();
        let mut scanner = Scanner::default();
        scanner.set_link(Link::Connected, start);
        assert_eq!(due(&mut scanner, start), Some(Background::WhenIdle), "connected scans wait for a quiet link");
        scanner.set_link(Link::Disconnected, at(start, 1));
        assert_eq!(scanner.interval(at(start, 1)), Some(Duration::from_secs(4)));
        assert_eq!(due(&mut scanner, at(start, 21)), Some(Background::Now));
        assert_eq!(due(&mut scanner, at(start, 23)), None);
        assert_eq!(due(&mut scanner, at(start, 25)), Some(Background::Now));
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
        assert!(due(&mut scanner, at(start, 100)).is_some());
        scanner.wake(at(start, 101));
        assert_eq!(due(&mut scanner, at(start, 101)), Some(Background::Now));
        assert_eq!(scanner.interval(at(start, 101)), Some(Duration::from_secs(4)));
    }

    #[test]
    fn a_page_scan_restarts_the_background_clock() {
        let start = Instant::now();
        let mut scanner = Scanner::default();
        scanner.set_link(Link::Connected, start);
        scanner.scanned(at(start, 5));
        assert_eq!(due(&mut scanner, at(start, 20)), None);
        assert!(due(&mut scanner, at(start, 25)).is_some());
    }

    #[test]
    fn a_call_counts_as_traffic_and_idle_chatter_does_not() {
        let sample = |bytes, packets| Traffic { bytes, packets };
        let zero = Traffic::default();
        assert!(busy(zero, sample(400 * 1024, 300), IDLE_SAMPLE), "a video call");
        assert!(busy(zero, sample(30 * 1024, 200), IDLE_SAMPLE), "a voice call: few bytes, many packets");
        assert!(busy(zero, sample(4 * 1024, 40), IDLE_SAMPLE), "a game or a typed-in ssh session");
        assert!(!busy(zero, sample(8 * 1024, 8), IDLE_SAMPLE), "background chatter");
        assert!(!busy(sample(500, 50), sample(100, 10), IDLE_SAMPLE), "a counter reset is no traffic");
    }
}
