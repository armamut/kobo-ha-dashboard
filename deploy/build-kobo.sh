#!/bin/bash
# ==============================================================================
# Kobo HA Dashboard - Cross Compilation Helper
# Builds a static ARMv7 binary for Kobo e-readers
# ==============================================================================
set -e

TARGET="armv7-unknown-linux-musleabihf"

echo "============================================================"
echo "Building for Kobo (ARMv7)"
echo "Target: $TARGET"
echo "============================================================"

# Check if rustup target is installed
if ! rustup target list | grep -q "$TARGET (installed)"; then
    echo "Adding rustup target $TARGET..."
    rustup target add "$TARGET"
fi

# Check available cross-compilation tool
if command -v cargo-zigbuild >/dev/null 2>&1; then
    echo "Building with cargo-zigbuild..."
    cargo zigbuild --target "$TARGET" --release
elif command -v cross >/dev/null 2>&1; then
    echo "Building with cross..."
    cross build --target "$TARGET" --release
else
    echo "Neither cargo-zigbuild nor cross was found."
    echo "The easiest way is cargo-zigbuild:"
    echo "  1) brew install zig"
    echo "  2) cargo install cargo-zigbuild"
    echo "  3) ./deploy/build-kobo.sh"
    echo ""
    echo "Trying plain cargo..."
    cargo build --target "$TARGET" --release || {
        echo "Plain cargo needs a cross linker."
        echo "Install zig and cargo-zigbuild: brew install zig && cargo install cargo-zigbuild"
        exit 1
    }
fi

OUTPUT="target/$TARGET/release/kobo-ha-dashboard"
if [ -f "$OUTPUT" ]; then
    echo ""
    echo "Build succeeded"
    echo "Binary: $OUTPUT"
    ls -lh "$OUTPUT"
fi
