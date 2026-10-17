#!/bin/bash
# ==============================================================================
# Kobo HA Dashboard - command-line installer (for developers)
#
# Most people should use the setup tool instead (run kobo-ha-dashboard on your
# computer). This script copies a locally built binary plus config.yaml.
#
# Usage: ./deploy/install.sh [/path/to/KOBOeReader]
# ==============================================================================
set -e

KOBO_PATH="$1"
if [ -z "$KOBO_PATH" ]; then
    for p in "/Volumes/KOBOeReader" "/media/$USER/KOBOeReader" "/run/media/$USER/KOBOeReader"; do
        [ -d "$p/.kobo" ] && KOBO_PATH="$p" && break
    done
fi
if [ -z "$KOBO_PATH" ] || [ ! -d "$KOBO_PATH/.kobo" ]; then
    echo "No Kobo found. Connect it with USB, tap 'Connect' on its screen, or pass its path."
    exit 1
fi
echo "Kobo found at $KOBO_PATH"

# macOS: don't copy Finder metadata (._ files); NickelMenu would try to parse them.
if [ "$(uname)" = "Darwin" ]; then
    cp() { command cp -X "$@"; }
fi

TARGET_DIR="$KOBO_PATH/.kobo-ha"
mkdir -p "$TARGET_DIR"

BIN=""
for b in target/armv7-unknown-linux-musleabihf/release/kobo-ha-dashboard deploy/kobo-ha-dashboard; do
    [ -f "$b" ] && BIN="$b" && break
done
if [ -z "$BIN" ]; then
    echo "No ARM build found. Run ./deploy/build-kobo.sh first."
    exit 1
fi
cp "$BIN" "$TARGET_DIR/"
echo "Copied $BIN"

cp deploy/launch.sh "$TARGET_DIR/"
echo "Copied launch.sh"

if [ -f config.yaml ]; then
    [ -f "$TARGET_DIR/config.yaml" ] && cp "$TARGET_DIR/config.yaml" "$TARGET_DIR/config.yaml.bak"
    cp config.yaml "$TARGET_DIR/"
    echo "Copied config.yaml (previous one saved as config.yaml.bak)"
elif [ ! -f "$TARGET_DIR/config.yaml" ]; then
    cp config.example.yaml "$TARGET_DIR/config.yaml"
    echo "Copied config.example.yaml as config.yaml - edit it with your Home Assistant details"
fi

# Fonts for Chinese/Japanese, if the setup tool downloaded them
for dir in "$HOME/Library/Caches/kobo-ha-dashboard/fonts" "${XDG_CACHE_HOME:-$HOME/.cache}/kobo-ha-dashboard/fonts"; do
    if ls "$dir"/*.otf >/dev/null 2>&1; then
        mkdir -p "$TARGET_DIR/fonts"
        cp "$dir"/*.otf "$TARGET_DIR/fonts/"
        echo "Copied fonts from $dir"
    fi
done

NM_DIR="$KOBO_PATH/.adds/nm"
mkdir -p "$NM_DIR"
cp deploy/nickelmenu/kobo-ha "$NM_DIR/"
echo "Installed the NickelMenu entry"

find "$TARGET_DIR" "$NM_DIR" -name '._*' -delete 2>/dev/null || true

echo
echo "Done. Eject the Kobo, restart it, and open the menu at the bottom right."
