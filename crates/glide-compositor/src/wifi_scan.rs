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

#[cfg(test)]
mod tests {
    use super::*;

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
