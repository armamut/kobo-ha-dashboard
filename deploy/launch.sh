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

# Only one instance at a time (a double tap in NickelMenu would otherwise
# start two copies fighting over the screen).
if ! mkdir "$LOCK" 2>/dev/null; then
    exit 0
fi

[ -f "$LOG" ] && mv -f "$LOG" "$LOG.old"
exec >"$LOG" 2>&1
echo "=== $(date) launching dashboard ==="

if [ ! -f "$BINARY" ]; then
    echo "ERROR: $BINARY not found"
    rmdir "$LOCK"
    exit 1
fi
chmod +x "$BINARY" 2>/dev/null

# Remember the frontlight level so it is restored even if the dashboard
# crashes while the light is switched off.
LIGHT="/sys/class/backlight/mxc_msp430.0/brightness"
LIGHT_LEVEL=""
[ -r "$LIGHT" ] && LIGHT_LEVEL="$(cat "$LIGHT" 2>/dev/null)"

PAUSED=""
resume() {
    if [ -n "$LIGHT_LEVEL" ] && [ -w "$LIGHT" ]; then
        echo "$LIGHT_LEVEL" >"$LIGHT" 2>/dev/null
    fi
    # Nickel first, so it can answer the watchdog as soon as that wakes up.
    if [ -n "$PAUSED" ]; then
        killall -CONT nickel 2>/dev/null
        sleep 2
        killall -CONT sickel 2>/dev/null
        PAUSED=""
        echo "Nickel resumed"
    fi
    rmdir "$LOCK" 2>/dev/null
}
trap resume EXIT
trap 'exit 1' INT TERM HUP

# Keep Wi-Fi from power-saving while we poll Home Assistant
if [ -f "/sys/class/net/wlan0/power/control" ]; then
    echo on >/sys/class/net/wlan0/power/control 2>/dev/null
fi

# Let NickelMenu's menu close and Nickel finish drawing before we pause it.
sleep 1

# Watchdog first, then Nickel.
killall -STOP sickel 2>/dev/null
killall -STOP nickel 2>/dev/null
PAUSED=1
echo "Nickel paused"

cd "$APP_DIR" || exit 1
RUST_LOG="${RUST_LOG:-kobo_ha_dashboard=info}" "$BINARY" --device --config "$CONFIG"
echo "Dashboard exited with status $?"

exit 0
