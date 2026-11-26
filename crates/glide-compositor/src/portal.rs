use std::time::{Duration, Instant};

pub const NOTICE_ID: u32 = u32::MAX - 14;
pub const SIGN_IN: &str = "sign-in";
pub const CHECK_EVERY: Duration = Duration::from_secs(5);
pub const RECHECK_EVERY: Duration = Duration::from_secs(10);
pub const WATCH: Duration = Duration::from_mins(1);

#[derive(Debug, Default)]
pub struct Portal {
    pub behind: bool,
    network: String,
    watch_until: Option<Instant>,
    checked: Option<Instant>,
    probed: Option<Instant>,
}

impl Portal {
    pub fn note_network(&mut self, network: &str, now: Instant) {
        if self.network != network {
            self.network = network.to_string();
            self.watch_until = Some(now + WATCH);
        }
    }

    pub fn due(&mut self, now: Instant) -> Option<Probe> {
        let watching = self.watch_until.is_some_and(|until| now < until);
        if !self.behind && !watching {
            return None;
        }
        if self.checked.is_some_and(|at| now.saturating_duration_since(at) < CHECK_EVERY) {
            return None;
        }
        self.checked = Some(now);
        if self.behind && self.probed.is_none_or(|at| now.saturating_duration_since(at) >= RECHECK_EVERY) {
            self.probed = Some(now);
            return Some(Probe::Check);
        }
        Some(Probe::Read)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    Read,
    Check,
}

#[must_use]
pub fn is_portal(connectivity: &str) -> bool {
    connectivity.trim() == "portal"
}

#[must_use]
pub fn default_gateway(route: &str) -> Option<std::net::Ipv4Addr> {
    route.lines().skip(1).find_map(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [_, destination, gateway, ..] = fields.as_slice() else { return None };
        if *destination != "00000000" {
            return None;
        }
        let raw = u32::from_str_radix(gateway, 16).ok().filter(|raw| *raw != 0)?;
        Some(std::net::Ipv4Addr::from(raw.to_le_bytes()))
    })
}

#[must_use]
pub fn sign_in_url() -> String {
    std::fs::read_to_string("/proc/net/route")
        .ok()
        .and_then(|route| default_gateway(&route))
        .map_or_else(|| "http://neverssl.com".to_string(), |gateway| format!("http://{gateway}"))
}

impl crate::GlideState {
    pub fn handle_captive(&mut self, behind: bool) {
        use crate::i18n::{Key, t};
        if self.session.portal.behind == behind {
            return;
        }
        self.session.portal.behind = behind;
        tracing::info!(behind, "the network's sign-in page");
        if behind {
            let ssid = self.topbar.system_status().wifi_ssid.clone();
            let summary = if ssid.is_empty() {
                t(Key::PortalSummaryWired).to_string()
            } else {
                t(Key::PortalSummary).replace("{name}", &ssid)
            };
            let notice = glide_notify::Notification {
                id: NOTICE_ID,
                app_name: "Glide".to_string(),
                app_icon: "network-wireless".to_string(),
                summary,
                body: t(Key::PortalBody).to_string(),
                actions: vec![glide_notify::Action { key: SIGN_IN.into(), label: t(Key::PortalSignIn).into() }],
                resident: true,
                ..glide_notify::Notification::default()
            };
            self.notifications.push(&mut self.dock.icons, notice, true);
        } else {
            self.notifications.withdraw(NOTICE_ID, glide_notify::CloseReason::Revoked);
        }
        self.refresh_topbar();
        self.queue_redraw();
    }

    pub fn open_sign_in_page(&mut self) {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(sign_in_url());
        if let Err(err) = self.spawn_user_process(command) {
            tracing::warn!("could not open the network's sign-in page: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_route_names_the_gateway() {
        let route =
            "Iface\tDestination\tGateway \tFlags\nwlan0\t00000000\t013012AC\t0003\ndocker0\t000011AC\t00000000\t0001\n";
        assert_eq!(default_gateway(route), Some(std::net::Ipv4Addr::new(172, 18, 48, 1)));
        assert_eq!(default_gateway("Iface\tDestination\tGateway\ndocker0\t000011AC\t00000000\n"), None);
    }

    #[test]
    fn only_a_portal_state_counts() {
        assert!(is_portal("portal\n"));
        assert!(!is_portal("full"));
        assert!(!is_portal("limited"));
    }

    #[test]
    fn a_portal_is_checked_for_real_while_it_lasts_and_only_read_otherwise() {
        let start = Instant::now();
        let mut portal = Portal::default();
        assert_eq!(portal.due(start), None, "nothing is asked before a network shows up");
        portal.note_network("Cafe", start);
        assert_eq!(portal.due(start), Some(Probe::Read));
        assert_eq!(portal.due(start + Duration::from_secs(2)), None);
        portal.behind = true;
        assert_eq!(portal.due(start + CHECK_EVERY), Some(Probe::Check), "after signing in the state is asked anew");
        assert_eq!(portal.due(start + CHECK_EVERY * 2), Some(Probe::Read));
        assert_eq!(portal.due(start + CHECK_EVERY * 3), Some(Probe::Check));
        portal.behind = false;
        assert_eq!(portal.due(start + WATCH), None, "a settled network is left alone after a minute");
        portal.note_network("Cafe", start + WATCH);
        assert_eq!(portal.due(start + WATCH + CHECK_EVERY), None, "the same network starts no new watch");
        portal.note_network("Home", start + WATCH);
        assert!(portal.due(start + WATCH + CHECK_EVERY).is_some(), "a new one does");
    }
}
