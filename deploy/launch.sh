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

# Only one instance at a time (a double tap in NickelMenu would otherwise
# start two copies fighting over the screen).
if ! mkdir "$LOCK" 2>/dev/null; then
    exit 0
fi

[ -f "$LOG" ] && mv -f "$LOG" "$LOG.old"
exec >"$LOG" 2>&1
echo "=== $(date) launching dashboard ==="
rm -f "$WIFI_MARKER"

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
WIFI_PM="/sys/class/net/${INTERFACE:-wlan0}/device/power/control"
[ -w "$WIFI_PM" ] && WIFI_PM_SAVED="$(cat "$WIFI_PM" 2>/dev/null)"

PAUSED=""
resume() {
    if [ -z "$CLEAN_EXIT" ] && [ -w "$LIGHT" ]; then
        level="$(nickel_light)"
        [ -n "$level" ] && echo "$level" >"$LIGHT" 2>/dev/null && echo "Frontlight restored to $level"
    fi
    for entry in $CORES_SAVED; do
        echo "${entry#*=}" >"${entry%=*}" 2>/dev/null
    done
    [ -n "$WIFI_PM_SAVED" ] && echo "$WIFI_PM_SAVED" >"$WIFI_PM" 2>/dev/null
    # Get the log onto the disk before anything that could hang the kernel.
    sync
    # Sleep mode powers Wi-Fi down between refreshes; make sure Nickel gets it
    # back. Only if the dashboard turned it off: when Wi-Fi was already off in
    # Nickel, loading the driver behind its back can freeze older Kobos (Glo).
    if [ -e "$WIFI_MARKER" ] && [ -n "$WIFI_MODULE" ] && [ ! -e "/sys/class/net/${INTERFACE:-wlan0}" ] && [ -f "$APP_DIR/wifi.sh" ]; then
        /bin/sh "$APP_DIR/wifi.sh" on
    fi
    rm -f "$WIFI_MARKER"
    # Nickel first, so it can answer the watchdog as soon as that wakes up.
    if [ -n "$PAUSED" ]; then
        killall -CONT nickel 2>/dev/null
        sleep 2
        killall -CONT sickel 2>/dev/null
        PAUSED=""
        echo "Nickel resumed"
    fi
    rmdir "$LOCK" 2>/dev/null
    sync
}
trap resume EXIT
trap 'exit 1' INT TERM HUP

echo "Device: PLATFORM=$PLATFORM WIFI_MODULE=$WIFI_MODULE INTERFACE=$INTERFACE PRODUCT=$PRODUCT"

# Power saving while the dashboard runs (restored on exit):
# - let the Wi-Fi chip power-save between packets (updates are pushed by
#   Home Assistant, so the radio may sleep most of the time)
# - one CPU core is plenty for the dashboard
[ -w "$WIFI_PM" ] && echo auto >"$WIFI_PM" 2>/dev/null
command -v iw >/dev/null 2>&1 && iw dev "${INTERFACE:-wlan0}" set power_save on 2>/dev/null
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
