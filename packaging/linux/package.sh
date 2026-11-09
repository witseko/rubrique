#!/bin/sh
# Pack the release binary into dist/rubrique-VERSION-linux-ARCH.tar.gz.
# Usage: packaging/linux/package.sh VERSION [BINARY]
set -eu
version="$1"
binary="${2:-target/release/rubrique}"
arch="$(uname -m)"
name="rubrique-$version-linux-$arch"
stage="dist/$name"

rm -rf "$stage"
install -Dm755 "$binary" "$stage/bin/rubrique"
install -Dm644 packaging/linux/rubrique.desktop "$stage/share/applications/rubrique.desktop"
for size in 64 128 256 512; do
    icon="assets/icon/linux/rubrique-$size.png"
    if [ -f "$icon" ]; then
        install -Dm644 "$icon" "$stage/share/icons/hicolor/${size}x${size}/apps/rubrique.png"
    fi
done
install -m755 packaging/linux/install.sh "$stage/install.sh"
cp README.md CHANGELOG.md "$stage/"

tar -C dist -czf "dist/$name.tar.gz" "$name"
rm -rf "$stage"
echo "dist/$name.tar.gz"
