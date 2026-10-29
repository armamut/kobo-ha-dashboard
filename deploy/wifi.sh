#!/bin/sh
# ==============================================================================
# Kobo HA Dashboard - Wi-Fi power control for sleep mode
#
#   wifi.sh off   disconnect and power the Wi-Fi chip down (required before
#                 suspend-to-RAM: suspending with Wi-Fi up can crash the kernel)
#   wifi.sh on    power it up, start wpa_supplicant with Nickel's saved
#                 networks and get an address
#
# Uses WIFI_MODULE, INTERFACE and PLATFORM, which the dashboard inherits from
# Nickel when it is started from NickelMenu.
# ==============================================================================

IFACE="${INTERFACE:-wlan0}"
# KOBO_HA_RUN_DIR is only set by the tests.
RUN_DIR="${KOBO_HA_RUN_DIR:-/tmp}"
# Left behind by "off" so launch.sh only turns Wi-Fi back on if we turned it off
MARKER="$RUN_DIR/kobo-ha-wifi-off"
MODULE="${WIFI_MODULE:-}"
KMOD_DIR="/drivers/${PLATFORM}/wifi"

if [ -z "$MODULE" ]; then
    echo "WIFI_MODULE is not set"
    exit 2
fi

# MediaTek chips stay loaded and are powered through /dev/wmtWifi;
# others are powered by loading/unloading their kernel modules.
case "$MODULE" in
    wlan_drv_gen4m) KIND="wmt"; DRIVER="nl80211"; KMOD_DIR="/drivers/${PLATFORM}/mt66xx" ;;
    *) KIND="module"; DRIVER="wext" ;;
esac

loaded() { grep -q "^$1 " /proc/modules; }
address() { ifconfig "$IFACE" 2>/dev/null | sed -n 's/.*inet \(addr:\)\{0,1\}\([0-9.]*\).*/\2/p'; }
has_route() { { ip route 2>/dev/null || route -n 2>/dev/null; } | grep -q '^default\|^0\.0\.0\.0'; }

wifi_off() {
    if command -v dhcpcd >/dev/null 2>&1; then
        dhcpcd -k "$IFACE" >/dev/null 2>&1
    fi
    killall -q udhcpc 2>/dev/null
    wpa_cli -i "$IFACE" terminate >/dev/null 2>&1
    # A supplicant that outlives the driver can't join anything after the
    # next "on"; make sure it is gone.
    n=0
    while pidof wpa_supplicant >/dev/null && [ $n -lt 10 ]; do usleep 200000; n=$((n + 1)); done
    pidof wpa_supplicant >/dev/null && killall wpa_supplicant 2>/dev/null
    # Broadcom (dhd) chips are switched off with wlarm_le before the interface
    [ "$MODULE" = "dhd" ] && command -v wlarm_le >/dev/null 2>&1 && wlarm_le -i "$IFACE" down
    ifconfig "$IFACE" down 2>/dev/null
    if [ "$KIND" = "wmt" ]; then
        echo 0 >/dev/wmtWifi
    else
        loaded "$MODULE" && { sleep 1; rmmod "$MODULE"; }
        loaded sdio_wifi_pwr && { sleep 1; rmmod sdio_wifi_pwr; }
    fi
    touch "$MARKER"
    echo "off"
}

wifi_on() {
    if [ -e "/sys/class/net/$IFACE" ] && [ -n "$(address)" ] &&
        wpa_cli -i "$IFACE" status 2>/dev/null | grep -q "wpa_state=COMPLETED"; then
        rm -f "$MARKER"
        echo "on: $(address) (already)"
        return 0
    fi
    if [ "$KIND" = "wmt" ]; then
        for m in wmt_drv wmt_chrdev_wifi wmt_cdev_bt "$MODULE"; do
            loaded "$m" || { insmod "$KMOD_DIR/$m.ko" 2>/dev/null; sleep 1; }
        done
        # Reset the connectivity chip before powering Wi-Fi, as Nickel's
        # firmware does on MediaTek Kobos.
        if [ -w /proc/driver/wmt_dbg ]; then
            echo "0xDB9DB9" >/proc/driver/wmt_dbg
            echo "7 9 0" >/proc/driver/wmt_dbg
            sleep 1
            echo "0xDB9DB9" >/proc/driver/wmt_dbg
            echo "7 9 1" >/proc/driver/wmt_dbg
        fi
        echo 1 >/dev/wmtWifi
    else
        if ! loaded sdio_wifi_pwr && [ -e "$KMOD_DIR/sdio_wifi_pwr.ko" ]; then
            insmod "$KMOD_DIR/sdio_wifi_pwr.ko"
        fi
        if ! loaded "$MODULE"; then
            RELOADED=1
            if [ -e "$KMOD_DIR/$MODULE.ko" ]; then
                insmod "$KMOD_DIR/$MODULE.ko"
            else
                insmod "/drivers/${PLATFORM}/$MODULE.ko"
            fi
        fi
    fi

    # Wait for the interface to appear
    n=0
    while [ ! -e "/sys/class/net/$IFACE" ] && [ $n -lt 20 ]; do usleep 500000; n=$((n + 1)); done
    # The driver needs a moment after the interface appears (KOReader: "race-y
    # as hell, don't try to optimize this").
    sleep 1
    ifconfig "$IFACE" up || { echo "interface $IFACE did not come up"; exit 1; }
    # Broadcom (dhd) chips can't scan until they are switched on with wlarm_le
    # (as Nickel and KOReader do).
    [ "$MODULE" = "dhd" ] && command -v wlarm_le >/dev/null 2>&1 && wlarm_le -i "$IFACE" up

    # A supplicant left from before the driver was reloaded can't use the new
    # interface: start a fresh one.
    if [ -n "$RELOADED" ] && pidof wpa_supplicant >/dev/null; then
        echo "restarting wpa_supplicant left from before"
        killall wpa_supplicant 2>/dev/null
        n=0
        while pidof wpa_supplicant >/dev/null && [ $n -lt 10 ]; do usleep 200000; n=$((n + 1)); done
    fi
    if ! pidof wpa_supplicant >/dev/null; then
        wpa_supplicant -D "$DRIVER" -s -i "$IFACE" -c /etc/wpa_supplicant/wpa_supplicant.conf \
            -C /var/run/wpa_supplicant -B
    else
        wpa_cli -i "$IFACE" reconnect >/dev/null 2>&1
    fi

    # Wait for association with one of Nickel's saved networks
    n=0
    until wpa_cli -i "$IFACE" status 2>/dev/null | grep -q "wpa_state=COMPLETED"; do
        n=$((n + 1))
        # Nudge a supplicant that is still idle after 10 s.
        [ $n -eq 20 ] && wpa_cli -i "$IFACE" reassociate >/dev/null 2>&1
        if [ $n -ge 60 ]; then
            echo "could not join a saved network ($(wpa_cli -i "$IFACE" status 2>/dev/null | grep '^wpa_state='), $(wpa_cli -i "$IFACE" scan_results 2>/dev/null | tail -n +2 | wc -l) networks in range)"
            exit 1
        fi
        usleep 500000
    done

    echo "joined $(wpa_cli -i "$IFACE" status 2>/dev/null | sed -n 's/^ssid=//p') after $((n / 2)) s"
    if command -v dhcpcd >/dev/null 2>&1; then
        dhcpcd -t 30 -w "$IFACE" >/dev/null 2>&1 || echo "dhcpcd failed"
    else
        udhcpc -i "$IFACE" -s /etc/udhcpc.d/default.script -q -n -t 10 >/dev/null 2>&1 || echo "udhcpc failed"
    fi
    # Wait until the address and default route are really there.
    n=0
    while [ -z "$(address)" ] || ! has_route; do
        n=$((n + 1))
        if [ $n -ge 30 ]; then
            echo "no address or route: '$(address)'"
            exit 1
        fi
        usleep 500000
    done
    rm -f "$MARKER"
    echo "on: $(address)"
}

# Keep going if whoever reads our output goes away (the dashboard exiting
# while Wi-Fi comes up in the background): dying on SIGPIPE would leave the
# lock behind.
trap '' PIPE

# One at a time: the dashboard may still be bringing Wi-Fi up in the
# background when launch.sh does the same on exit. A lock whose owner is
# gone is taken over.
LOCK="$RUN_DIR/kobo-ha-wifi.lock"
n=0
while ! mkdir "$LOCK" 2>/dev/null; do
    owner="$(cat "$LOCK/pid" 2>/dev/null)"
    if [ -z "$owner" ] || ! kill -0 "$owner" 2>/dev/null; then
        rm -rf "$LOCK"
        continue
    fi
    n=$((n + 1))
    [ $n -ge 120 ] && break
    usleep 500000
done
echo $$ >"$LOCK/pid" 2>/dev/null
trap 'rm -rf "$LOCK"' EXIT
trap 'exit 1' INT TERM HUP

case "$1" in
    off) wifi_off ;;
    on) wifi_on ;;
    *) echo "usage: $0 on|off"; exit 2 ;;
esac
