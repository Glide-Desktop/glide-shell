# Wi-Fi scanning, switching between known networks, captive portals

Plan of 2026-11-26. Built in glide-shell and glide-settings; not run live.
Left: pixel scrolling (§5) and whether a phone hotspot reads as metered (§3).

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
- Under iwd, `LastScan` jumps to the request time the moment a scan is asked
  for, while access points keep arriving for up to 5 s after it. It says
  nothing about a finished scan, so scans run on a fixed cadence.

## 1. Stop blocking on scans — done in the shell

- `qs_net`: opening the page or the scan-now button (`RescanWifi`) asks
  NetworkManager for a scan without waiting (`nmcli dev wifi rescan`) and shows
  the spinner for `WIFI_SCAN_SETTLE` (4 s); while the page stays open a quiet
  `RefreshWifi` repeats that every 4 s (8 s on battery) without the spinner,
  and the cached list (`list --rescan no`) is read every second. Saved
  networks are cached between reads, rows sort by bars rather than raw signal,
  and no scan is asked for while a connection is being made.
- glide-settings: the Network page asks for a scan without waiting on open,
  after a change and on each 4 s poll (every other one on battery), lists the
  cache, and reads "Fewer scans on battery · …" on battery.

## 2. One scanner in the compositor — done

`wifi_scan::Scanner` (pure, unit-tested) owns background scanning, driven by
the compositor's 1 s network timer (`tick_network_scans`) through the same
`nmcli dev wifi rescan` requests. The link comes from the system status
(`Link::of`: off without a radio, in airplane mode or on a cable), the battery
from `handle_system_status`; resume and lid open restart the burst. A scan is
stamped only when the worker accepted it (`Cadence`, `try_request`), so a
request refused while the previous scan still runs is retried on the next
tick. A connected scan runs only when a switch could follow (`may_switch`:
switching on, and the link metered or a saved network of higher priority),
and is skipped when the wireless interface moved 32 KiB/s or 10 packets/s
over 2 s — the packet rate catches calls, games and ssh. The QS Wi-Fi page
counts as a scan, so the background waits a full interval after it closes.

Modes:

- **Fast**, while the QS Network page or Settings › Network is open: a scan
  every 4 s (one cycle is 3–5 s, the hardware floor). The current connection
  lags a little; acceptable for as long as the page is open.
- **Disconnected**: every 15–20 s; every 3–5 s for the first minute after a
  disconnect, resume or lid open. Nothing to harm without a connection.
- **Connected**: every 15–20 s, skipped while the link carries traffic (rx + tx
  above a threshold over the last few seconds, from
  `/sys/class/net/<dev>/statistics`). A scan leaves the operating channel for
  50–200 ms per dwell, which shows as a ping spike in calls and games; neither
  iwd nor NetworkManager can scan a subset of channels. This mode exists for §3.
- **Off**: radio off, airplane mode.

Settings drives its own fast scans through NetworkManager.

On battery (a discharging system battery, as the idle policy reads it) every
interval grows, starting points to tune:

| Mode | Mains | Battery |
|---|---|---|
| Fast (page open) | 4 s | 8 s |
| Disconnected, first minute | 4 s | 10 s |
| Disconnected | 15 s | 60 s |
| Connected | 20 s | 60 s |

The user is told: on battery the scan line of the QS Network page and of
Settings › Network reads "Fewer scans on battery · N s ago" ("Від батареї
пошук рідше · N с тому"), and the scan-now button runs one scan at once whatever the
mode. Plugging in or unplugging switches the intervals without waiting for the
current one to end.

## 3. Switch between known networks in the background — done in the shell

Built as `wifi_scan::Switcher` (pure, unit-tested), run on the worker after a
connected background scan that found the link idle, behind
`general.switch-to-preferred-networks` (default on). A network picked by hand
in QS is held for as long as it stays connected (not 5 minutes), and going
back to the network the shell just left holds that one too, which also covers
picks made in Settings or nmcli. A network the device was ever metered on is
remembered and never chosen as a way out of another
metered one, so two phone hotspots do not swap back and forth. Not yet known:
whether NetworkManager reports a Pixel hotspot as metered at all
(`nmcli -g GENERAL.METERED dev show wlan0` while on it). Settings › Network
has the switch ("Switch to preferred networks") and a "Network order" group
whose up / down buttons renumber `connection.autoconnect-priority`.

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

## 4. Captive portals — the notice is done in the shell

`portal.rs`: for a minute after the network changes, the network timer reads
NetworkManager's connectivity every 5 s (`nmcli -t networking connectivity`,
the cached state) and, while it says
`portal`, asks for a real check every 10 s so the notice goes as soon as the
user has signed in. The notice is a resident island notice ("Sign in to
«SSID»") whose Sign In button opens `http://<default gateway>` (from
`/proc/net/route`) with `xdg-open`, launched with an activation token so the
browser comes forward. Nothing shows while NetworkManager's check
is switched off; Settings › Network › "Detect sign-in pages" sets it
(NetworkManager's ConnectivityCheckEnabled; whether NetworkManager keeps it
across restarts was not checked — its state file is root-only).

- Settings switch for the connectivity check (`ConnectivityCheckEnabled`, a
  writable NetworkManager property that it persists itself).
- `Connectivity` turning `PORTAL` (2) shows an island notice "Sign in to
  network" that opens the default browser on `http://<gateway>` (what worked
  live on 2026-11-26), then `CheckConnectivity` once the page is closed or after a timeout.

## 5. QS list scrolling — a thumb is done

The Wi-Fi, Bluetooth and Sound lists already scrolled by wheel and touchpad,
one row per step; nothing showed that more rows existed. They now draw the
notification centre's 3 px thumb in the right gutter (`panels::list_thumb`)
whenever rows are hidden above or below. Pixel scrolling (a page raster
cropped by an animated offset, with an edge fade) is not built: it reworks
the page raster pipeline and could not be seen from a headless machine.
