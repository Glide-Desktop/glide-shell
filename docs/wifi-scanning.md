# Wi-Fi scanning, switching between known networks, captive portals

Plan of 2026-11-26. Nothing here is built yet.

## Findings

- The Quick Settings Network page runs `nmcli dev wifi list --rescan yes`
  every 8 s (`qs_net::RESCAN_INTERVAL`). NetworkManager with the iwd backend
  rate-limits scans: a forced scan right after another one does not scan, and
  nmcli waits out its 15 s timeout (measured: 0.05 s for the first, 15.0 s for
  the second). Meanwhile `WIFI_SCANNING` drops every other scan request, so
  the list changes every 15–23 s and "Searching for networks…" spins all along.
- glide-settings does the same in `services/network.rs` (`read(Rescan)`).
- `nmcli dev wifi rescan` (request only) returns in 0.05 s,
  `nmcli dev wifi list --rescan no` (cache only) in 0.03 s.
- iwd 3.12 runs without `/etc/iwd/main.conf`: while disconnected it scans
  after 10 s and doubles the interval up to 300 s, so a known network can take
  minutes to be noticed.
- NetworkManager's connectivity check was off (`ConnectivityCheckEnabled =
  false`), so connectivity always read `full`, even behind a captive portal.
  It was switched on by hand on the user's machine.
- The QS Wi-Fi list does scroll, but only by wheel notches (one row each), with
  no scrollbar or hint that more rows exist.

## 1. Stop blocking on scans (first step, small)

- `qs_net`: `RescanWifi` becomes `nmcli dev wifi rescan` (fire and forget)
  followed by `list --rescan no`; while the page is open the cached list is
  re-read every second.
- glide-settings: the same in `services/network.rs`.

## 2. One scanner in the compositor

A worker in the compositor owns background scanning. Better done over
NetworkManager's D-Bus (`Device.Wireless.RequestScan`, `LastScan`,
`AccessPointAdded` / `AccessPointRemoved`) than nmcli: the signals update the
list the moment iwd reports, with no polling.

Modes:

- **Fast**, while the QS Network page or Settings › Network is open: ask for
  the next scan as soon as `LastScan` changes (one cycle is 3–5 s, the
  hardware floor). The current connection lags a little; acceptable for as
  long as the page is open.
- **Disconnected**: every 15–20 s; every 3–5 s for the first minute after a
  disconnect, resume or lid open. Nothing to harm without a connection.
- **Connected**: every 15–20 s, skipped while the link carries traffic (rx + tx
  above a threshold over the last few seconds, from
  `/sys/class/net/<dev>/statistics`). A scan leaves the operating channel for
  50–200 ms per dwell, which shows as a ping spike in calls and games; neither
  iwd nor NetworkManager can scan a subset of channels. This mode exists for §3.
- **Off**: radio off, airplane mode.

Settings drives its own fast scans through NetworkManager; the compositor's
background timer restarts on every `LastScan` change, so the two never stack.

On battery (a discharging system battery, as the idle policy reads it) every
interval grows, starting points to tune:

| Mode | Mains | Battery |
|---|---|---|
| Fast (page open) | back to back | 5 s pause between scans |
| Disconnected, first minute | 3–5 s | 10 s |
| Disconnected | 15–20 s | 60 s |
| Connected | 15–20 s | 60 s |

The user is told: on battery the scan line of the QS Network page and of
Settings › Network reads "On battery · searching less often" next to
"updated N s ago", and the scan-now button runs one scan at once whatever the
mode. Plugging in or unplugging switches the intervals without waiting for the
current one to end.

## 3. Switch between known networks in the background

NetworkManager never leaves a working connection for a better one, and iwd
roams only between access points of the same network. Glide decides:

- Candidates: saved Wi-Fi connections with autoconnect on, seen in the latest
  scan.
- Switch when the candidate is preferred: a higher
  `connection.autoconnect-priority`, or the current network is metered (a phone
  hotspot: `Device.Metered` guessed yes from Android's DHCP option) and the
  candidate is not.
- Only when the candidate is at least 60 % signal in two scans in a row, the
  link is idle (same check as §2), the user did not pick the current network by
  hand in the last 5 minutes, and the candidate did not fail in the last few
  minutes.
- `nmcli con up id <name>` (or `ActivateConnection`); on failure NetworkManager
  autoconnect picks again.
- An island glance: "Switched to «Home»".
- Settings › Network: a "Switch to preferred networks" switch and a drag order
  of saved networks that writes `autoconnect-priority`.

Same-SSID roaming stays with iwd.

## 4. Captive portals

- Settings switch for the connectivity check (`ConnectivityCheckEnabled`, a
  writable NetworkManager property that it persists itself).
- `Connectivity` turning `PORTAL` (2) shows an island notice "Sign in to
  network" that opens the default browser on `http://<gateway>` (what worked
  live on 2026-11-26), then `CheckConnectivity` once the page is closed or after a timeout.

## 5. QS list scrolling

Pixel scrolling (touchpad included) with `E_OUT` smoothing, a thin scrollbar
and an edge fade, for the Wi-Fi, Bluetooth and Sound pages.
