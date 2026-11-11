#!/bin/sh
# Install Rubrique from this folder. Default: for the current user, in ~/.local.
# For everyone: sudo ./install.sh /usr/local
set -eu
prefix="${1:-$HOME/.local}"
here="$(cd "$(dirname "$0")" && pwd)"

install -Dm755 "$here/bin/rubrique" "$prefix/bin/rubrique"
install -Dm644 "$here/share/applications/rubrique.desktop" "$prefix/share/applications/rubrique.desktop"
for icon in "$here"/share/icons/hicolor/*/apps/rubrique.png; do
    [ -f "$icon" ] || continue
    size_dir="$(basename "$(dirname "$(dirname "$icon")")")"
    install -Dm644 "$icon" "$prefix/share/icons/hicolor/$size_dir/apps/rubrique.png"
done

# Refresh the file type list so "Open With" offers Rubrique for Markdown,
# text, JSON and XML files. Not every desktop has the tool; skip it then.
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$prefix/share/applications" || true
fi

echo "Installed Rubrique to $prefix/bin/rubrique"
case ":$PATH:" in
    *":$prefix/bin:"*) ;;
    *) echo "Note: $prefix/bin is not on your PATH." ;;
esac
