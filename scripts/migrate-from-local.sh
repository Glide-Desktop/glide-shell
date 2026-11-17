#!/bin/sh
# Replace a Glide that scripts/install.sh put under /usr/local (the monorepo
# installer) with the Glide pacman packages, without losing the login screen.
#
# Usage: sudo scripts/migrate-from-local.sh [--dry-run] [--yes] [PACKAGE.pkg.tar.zst ...]
#
#   With package files, they are installed with pacman -U; without, the
#   `glide` metapackage is installed from the configured repositories.
#
# Order, so a working greeter exists at every step:
#   1. back up every file this touches to /var/backups/glide-migrate-<time>.tar.gz
#   2. install the packages, overwriting only the exact files the old installer
#      left outside /usr/local (which pacman would otherwise refuse)
#   3. point greetd and the FaceKey PAM lines at /usr/bin
#   4. remove the old copies under /usr/local, which shadow the packages
#      (PATH, the systemd user unit and the settings defaults look there first)
#   5. check that everything greetd, PAM, D-Bus and systemd name exists
#
# Kept on purpose: per-user data in ~/.config/glide and ~/.local/state/glide,
# and the pam_glide_keychain lines in /etc/pam.d/greetd and /etc/pam.d/passwd
# (they name the module, not a path). The FaceKey models and ONNX Runtime the
# old installer copied are taken over by glide-facekey-models.
#
# Rollback: sudo tar -xzpPf /var/backups/glide-migrate-<time>.tar.gz
# then remove the packages with pacman -Rns glide and its dependencies.
set -eu

DRY=0
YES=0
PACKAGES=""
for arg in "$@"; do
    case "$arg" in
        --dry-run) DRY=1 ;;
        --yes) YES=1 ;;
        -h|--help) sed -n '2,26p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        -*) echo "unknown option: $arg" >&2; exit 2 ;;
        *) PACKAGES="$PACKAGES $arg" ;;
    esac
done

if [ "$DRY" -eq 0 ] && [ "$(id -u)" -ne 0 ]; then
    echo "error: run as root (sudo $0 $*), or pass --dry-run" >&2
    exit 1
fi

OLD=/usr/local
OLD_BINARIES="glide glidectl glide-configd glide-settings glide-files glide-terminal glide-text glide-passwords
glide-photos glide-folio glide-clips glide-catalog glide-portal glide-keychain glide-phone glide-session glide-lock
glide-welcome glide-xembed facekey"
OLD_APPS="glide-settings glide-files glide-terminal glide-text glide-passwords glide-photos glide-folio glide-clips
glide-catalog glide-phone glide-welcome"
OLD_DEFAULTS="shell passwords permissions pins terminal text folio"

shadowing() {
    for bin in $OLD_BINARIES; do echo "$OLD/bin/$bin"; done
    for app in $OLD_APPS; do echo "$OLD/share/applications/$app.desktop"; done
    for name in $OLD_DEFAULTS; do echo "$OLD/share/glide/defaults/$name.json"; done
    echo "$OLD/share/glide/sounds/terminal-bell.mp3"
    echo "$OLD/lib/systemd/user/glide.service"
    echo "$OLD/lib/systemd/user/glide-shutdown.target"
}

# What the old installer wrote outside its prefix and a package now owns.
overwritten() {
    cat <<'EOF'
/usr/share/icons/hicolor/scalable/apps/glide-folio.svg
/usr/share/icons/hicolor/scalable/apps/glide-phone.svg
/usr/share/icons/hicolor/scalable/apps/org.kde.kdeconnect.svg
/usr/share/wayland-sessions/glide.desktop
/usr/share/xdg-desktop-portal/glide-portals.conf
/usr/share/xdg-desktop-portal/portals/glide.portal
/usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.glide.service
/usr/share/dbus-1/services/org.glide.Files.service
/usr/share/dbus-1/services/org.glide.Terminal.service
/usr/share/dbus-1/services/org.glide.Phone.service
/usr/share/dbus-1/services/org.glide.Config.service
/usr/share/dbus-1/interfaces/org.glide.Shell1.xml
/usr/share/dbus-1/interfaces/org.glide.Config1.xml
/usr/share/gnome-shell/search-providers/glide-files-search-provider.ini
/usr/share/gnome-shell/search-providers/glide-terminal-search-provider.ini
/etc/xdg/autostart/glide-phone.desktop
/etc/pam.d/glide-lock
/usr/share/polkit-1/actions/org.glide.facekey.policy
/usr/lib/security/pam_glide_keychain.so
/usr/share/glide/face/face_detection_yunet_2023mar.onnx
/usr/share/glide/face/ghostfacenet_v1_w1.3_s1_ms1mv3.onnx
/usr/share/glide/face/MiniFASNetV2.onnx
/usr/share/glide/face/2d106det.onnx
/usr/lib/glide/face/libonnxruntime.so
EOF
}

# Left outside the prefix by the old installer, owned by nothing now.
orphaned() {
    echo /usr/share/icons/hicolor/scalable/apps/glide-welcome.svg
}

edited() {
    for f in /etc/greetd/config.toml /etc/pam.d/greetd /etc/pam.d/passwd /etc/pam.d/sudo /etc/pam.d/polkit-1; do
        echo "$f"
    done
}

run() {
    echo "+ $*"
    [ "$DRY" -eq 1 ] || "$@"
}

existing() {
    while read -r f; do
        [ -e "$f" ] && echo "$f"
    done
    return 0
}

if ! shadowing | existing | grep -q . && [ -x /usr/bin/glide ] && pacman -Qq glide >/dev/null 2>&1; then
    echo "Nothing under $OLD and the glide package is installed: already migrated."
    exit 0
fi

echo "== 1. backup"
STAMP="$(date +%Y%m%d-%H%M%S)"
BACKUP="/var/backups/glide-migrate-$STAMP.tar.gz"
LIST="$( { shadowing; overwritten; orphaned; edited; } | existing)"
if [ -n "$LIST" ]; then
    echo "$LIST" | sed 's/^/  /'
    if [ "$DRY" -eq 0 ]; then
        install -d -m 700 /var/backups
        echo "$LIST" | tar -czpf "$BACKUP" -P -T -
        echo "saved $BACKUP"
    fi
fi

echo "== 2. packages"
OVERWRITE=""
for f in $(overwritten); do
    OVERWRITE="$OVERWRITE --overwrite $f"
done
CONFIRM=""
[ "$YES" -eq 1 ] && CONFIRM="--noconfirm"
if pacman -Qq glide-secret-service >/dev/null 2>&1; then
    run pacman -Rdd $CONFIRM glide-secret-service
fi
if [ -n "$PACKAGES" ]; then
    # shellcheck disable=SC2086
    run pacman -U --needed $CONFIRM $OVERWRITE $PACKAGES
else
    # shellcheck disable=SC2086
    run pacman -S --needed $CONFIRM $OVERWRITE glide
fi
if [ "$DRY" -eq 0 ] && ! [ -x /usr/bin/glide ]; then
    echo "error: /usr/bin/glide is missing after the install; nothing under $OLD was touched" >&2
    exit 1
fi

echo "== 3. greetd and PAM"
if [ -f /etc/greetd/config.toml ] && grep -q "$OLD/bin/glide --greeter" /etc/greetd/config.toml; then
    run sed -i "s|$OLD/bin/glide --greeter|/usr/bin/glide --greeter|" /etc/greetd/config.toml
fi
for pair in polkit:polkit-1 sudo:sudo login:greetd; do
    service="${pair%%:*}"
    file="/etc/pam.d/${pair#*:}"
    if [ -f "$file" ] && grep -q "$OLD/bin/facekey" "$file"; then
        if [ -x /usr/bin/facekey ]; then
            run /usr/bin/facekey -pam-enable "$service"
        else
            echo "warning: $file names $OLD/bin/facekey but /usr/bin/facekey is missing; face unlock for $service stays off" >&2
        fi
    fi
done

echo "== 4. old copies"
for f in $( { shadowing; orphaned; } | existing); do
    run rm -f "$f"
done
for d in "$OLD/share/glide/defaults" "$OLD/share/glide/sounds" "$OLD/share/glide"; do
    [ -d "$d" ] && run rmdir --ignore-fail-on-non-empty "$d"
done

echo "== 5. checks"
[ "$DRY" -eq 1 ] && { echo "dry run: nothing changed"; exit 0; }
fail=0
check() {
    if [ -e "$1" ]; then
        echo "  ok   $1 ($2)"
    else
        echo "  MISSING $1 ($2)" >&2
        fail=1
    fi
}
if [ -f /etc/greetd/config.toml ] && grep -q 'glide --greeter' /etc/greetd/config.toml; then
    cmd="$(sed -n 's/^command *= *"\([^ "]*\).*/\1/p' /etc/greetd/config.toml | head -n1)"
    case "$cmd" in /*) check "$cmd" "greetd command" ;; *) check "$(command -v "$cmd" || echo "$cmd")" "greetd command" ;; esac
fi
for f in /usr/share/dbus-1/services/org.glide.*.service /usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.glide.service; do
    [ -f "$f" ] && check "$(sed -n 's/^Exec=\([^ ]*\).*/\1/p' "$f")" "$(basename "$f")"
done
check "$(sed -n 's/^ExecStart=\([^ ]*\).*/\1/p' /usr/lib/systemd/user/glide.service)" glide.service
check "$(sed -n 's/^Exec=\([^ ]*\).*/\1/p' /usr/share/wayland-sessions/glide.desktop)" "session entry"
if grep -qs pam_glide_keychain.so /etc/pam.d/greetd /etc/pam.d/passwd; then
    check /usr/lib/security/pam_glide_keychain.so "keychain PAM module"
fi
for file in /etc/pam.d/sudo /etc/pam.d/polkit-1 /etc/pam.d/greetd; do
    path="$(sed -n 's/.*pam_exec\.so quiet \([^ ]*facekey\).*/\1/p' "$file" 2>/dev/null | head -n1)"
    [ -n "$path" ] && check "$path" "FaceKey in $file"
done
for f in $(shadowing); do
    if [ -e "$f" ]; then
        echo "  LEFT $f" >&2
        fail=1
    fi
done
if [ "$fail" -ne 0 ]; then
    echo "Some checks failed. Do not reboot before fixing them; the backup is $BACKUP." >&2
    exit 1
fi
echo
echo "Done. Log out and back in (or run: systemctl --user daemon-reload) so the session uses /usr/bin."
echo "Backup: ${BACKUP:-none}"
