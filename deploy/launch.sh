#!/bin/sh
# ==============================================================================
# Kobo HA Dashboard - Standalone Launcher (started from NickelMenu)
#
# Pauses Nickel (and its watchdog), runs the dashboard, and always resumes
# Nickel afterwards - even if the dashboard crashes.
#
# Why the watchdog matters: Kobo firmware ships `sickel`, which reboots the
# device when Nickel stops responding. Pausing only Nickel makes sickel think
# it hung, so the e-reader freezes and then restarts. We pause both.
# ==============================================================================

APP_DIR="/mnt/onboard/.kobo-ha"
BINARY="$APP_DIR/kobo-ha-dashboard"
CONFIG="$APP_DIR/config.yaml"
LOG="$APP_DIR/dashboard.log"
LOCK="/tmp/kobo-ha-dashboard.lock"
WIFI_MARKER="/tmp/kobo-ha-wifi-off"
WIFI_USED="/tmp/kobo-ha-wifi-used"
IFACE="${INTERFACE:-wlan0}"

# Only one instance at a time (a double tap in NickelMenu would otherwise
# start two copies fighting over the screen).
if ! mkdir "$LOCK" 2>/dev/null; then
    exit 0
fi

[ -f "$LOG" ] && mv -f "$LOG" "$LOG.old"
exec >"$LOG" 2>&1
echo "=== $(date) launching dashboard ==="
rm -f "$WIFI_MARKER" "$WIFI_USED"

if [ ! -f "$BINARY" ]; then
    echo "ERROR: $BINARY not found"
    rmdir "$LOCK"
    exit 1
fi
chmod +x "$BINARY" 2>/dev/null

# The dashboard puts the frontlight back itself when it exits. If it crashes
# with the light off, restore Nickel's saved level instead (reading the sysfs
# value back returns 0 on some models, so it can't be saved here).
LIGHT="/sys/class/backlight/mxc_msp430.0/brightness"
# Firmware 6 no longer keeps the level in Nickel's settings: the level at
# start is the fallback.
LIGHT_START="$(cat /sys/class/backlight/mxc_msp430.0/actual_brightness 2>/dev/null)"
NICKEL_CONF="/mnt/onboard/.kobo/Kobo/Kobo eReader.conf"
CLEAN_EXIT=""
nickel_light() {
    tr -d '\r' <"$NICKEL_CONF" 2>/dev/null | awk -F= '
        /^\[/ { p = ($0 == "[PowerOptions]") }
        p && $1 == "FrontLightLevel" { level = $2 }
        p && $1 == "FrontLightState" { state = $2 }
        END { if (state == "false") level = 0; print level }'
}

# CPU cores other than cpu0 are switched off while the dashboard runs (it
# needs one core); remember their state to give Nickel back what it had.
CORES_SAVED=""
for c in /sys/devices/system/cpu/cpu[1-9]*/online; do
    [ -w "$c" ] || continue
    CORES_SAVED="$CORES_SAVED $c=$(cat "$c" 2>/dev/null)"
done
WIFI_PM_SAVED=""
WIFI_PM="/sys/class/net/$IFACE/device/power/control"
[ -w "$WIFI_PM" ] && WIFI_PM_SAVED="$(cat "$WIFI_PM" 2>/dev/null)"

wifi_state() {
    if [ -e "/sys/class/net/$IFACE" ]; then
        state="$(cat "/sys/class/net/$IFACE/operstate" 2>/dev/null), $(wpa_cli -i "$IFACE" status 2>/dev/null | grep '^wpa_state=')"
    else
        state="absent"
    fi
    echo "Wi-Fi $1: $state; $(ps 2>/dev/null | grep -E 'wpa_supplicant|dhcpcd|udhcpc' | grep -v grep | sed 's/  */ /g' | tr '\n' ';')"
}

PAUSED=""
resume() {
    if [ -z "$CLEAN_EXIT" ] && [ -w "$LIGHT" ]; then
        level="$(nickel_light)"
        [ -n "$level" ] || level="$LIGHT_START"
        [ -n "$level" ] && echo "$level" >"$LIGHT" 2>/dev/null && echo "Frontlight restored to $level"
    fi
    for entry in $CORES_SAVED; do
        echo "${entry#*=}" >"${entry%=*}" 2>/dev/null
    done
    # Gone while Wi-Fi is off (the error would come from the shell, not echo).
    [ -n "$WIFI_PM_SAVED" ] && [ -w "$WIFI_PM" ] && echo "$WIFI_PM_SAVED" >"$WIFI_PM"
    # Get the log onto the disk before anything that could hang the kernel.
    sync
    # Sleep mode switches Wi-Fi off and on with its own wpa_supplicant while
    # Nickel is paused believing it still has its own connection. Handed back
    # like that, Nickel could not find any network again until a restart.
    # As KOReader does, give it back switched off: Nickel brings it up itself
    # when it needs it. Untouched Wi-Fi is left as Nickel had it.
    if [ -e "$WIFI_USED" ] && [ -n "$WIFI_MODULE" ] && [ -e "/sys/class/net/$IFACE" ] && [ -f "$APP_DIR/wifi.sh" ]; then
        /bin/sh "$APP_DIR/wifi.sh" off
    fi
    [ -e "$WIFI_USED" ] && wifi_state "handed back to Nickel"
    rm -f "$WIFI_MARKER" "$WIFI_USED"
    # Nickel first, so it can answer the watchdog as soon as that wakes up.
    if [ -n "$PAUSED" ]; then
        killall -CONT nickel 2>/dev/null
        sleep 2
        killall -CONT sickel 2>/dev/null
        PAUSED=""
        echo "Nickel resumed"
        # Nickel keeps no log we can read: note what it did with Wi-Fi. Nothing
        # on the user partition stays open meanwhile, so connecting to a
        # computer can still unmount it.
        if [ -n "$WIFI_MODULE" ]; then
            (
                cd / && exec </dev/null >/dev/null 2>&1
                sleep 120
                [ -f "$LOG" ] && wifi_state "2 minutes after resuming Nickel" >>"$LOG"
                sync
            ) &
        fi
    fi
    rmdir "$LOCK" 2>/dev/null
    sync
}
trap resume EXIT
trap 'exit 1' INT TERM HUP

echo "Device: PLATFORM=$PLATFORM WIFI_MODULE=$WIFI_MODULE INTERFACE=$INTERFACE PRODUCT=$PRODUCT"
echo "Firmware: $(cut -d, -f3 /mnt/onboard/.kobo/version 2>/dev/null)"

# How Nickel runs Wi-Fi (it changed with firmware 5/6): its processes with
# their arguments, and where saved networks live. Only file names and the
# number of networks, never their contents.
nickel_wifi_info() {
    for p in $(pidof wpa_supplicant dhcpcd dhcpcd-dbus udhcpc connmand NetworkManager iwd 2>/dev/null); do
        echo "Nickel Wi-Fi process: $(tr '\0' ' ' <"/proc/$p/cmdline" 2>/dev/null)"
    done
    for f in $(find /etc /data /var/lib /mnt/settings -maxdepth 3 \( -name '*wpa*' -o -name '*.nmconnection' -o -name '*wifi*' \) 2>/dev/null); do
        if [ -f "$f" ]; then
            echo "Nickel Wi-Fi file: $f ($(wc -c <"$f") bytes, $(grep -c 'network={' "$f" 2>/dev/null) network blocks)"
        else
            echo "Nickel Wi-Fi file: $f/"
        fi
    done
    ls /var/run/wpa_supplicant /run/wpa_supplicant 2>/dev/null | sed 's/^/Nickel Wi-Fi control socket: /'
}
nickel_wifi_info

# Whether Bluetooth is on, as the kernel shows it: on firmware 6 a suspend
# with Bluetooth on breaks Wi-Fi, so the dashboard needs to know beforehand.
for d in /sys/class/rfkill/rfkill*; do
    [ -e "$d/type" ] && echo "rfkill: $(basename "$d") type=$(cat "$d/type" 2>/dev/null) name=$(cat "$d/name" 2>/dev/null) state=$(cat "$d/state" 2>/dev/null) soft=$(cat "$d/soft" 2>/dev/null) hard=$(cat "$d/hard" 2>/dev/null)"
done
ls /sys/class/bluetooth 2>/dev/null | sed 's/^/Bluetooth device: /'
for h in /sys/class/bluetooth/hci*; do
    [ -e "$h" ] && echo "Bluetooth $(basename "$h"): $(ls "$h" 2>/dev/null | tr '\n' ' ')"
done
command -v hciconfig >/dev/null 2>&1 && hciconfig 2>/dev/null | grep -E '^hci|UP|DOWN' | sed 's/^/hciconfig: /'
pidof bluetoothd >/dev/null 2>&1 && echo "bluetoothd running" || echo "bluetoothd not running"

# Power saving while the dashboard runs (restored on exit):
# - let the Wi-Fi chip power-save between packets (updates are pushed by
#   Home Assistant, so the radio may sleep most of the time)
# - one CPU core is plenty for the dashboard
[ -w "$WIFI_PM" ] && echo auto >"$WIFI_PM" 2>/dev/null
command -v iw >/dev/null 2>&1 && iw dev "$IFACE" set power_save on 2>/dev/null
for c in /sys/devices/system/cpu/cpu[1-9]*/online; do
    [ -w "$c" ] && echo 0 >"$c" 2>/dev/null
done

# Let NickelMenu's menu close and Nickel finish drawing before we pause it.
sleep 1

# Watchdog first, then Nickel.
killall -STOP sickel 2>/dev/null
killall -STOP nickel 2>/dev/null
PAUSED=1
echo "Nickel paused"

cd "$APP_DIR" || exit 1
RUST_LOG="${RUST_LOG:-kobo_ha_dashboard=info}" "$BINARY" --device --config "$CONFIG"
STATUS=$?
echo "Dashboard exited with status $STATUS"
[ "$STATUS" -eq 0 ] && CLEAN_EXIT=1

exit 0
