#!/bin/sh
# Build Rubrique.app from the two release binaries (Apple Silicon and Intel)
# and pack it as dist/rubrique-VERSION-macos-universal.dmg and .zip.
# Usage: packaging/macos/bundle.sh VERSION
#
# Signing:
# - MACOS_SIGN_IDENTITY set, e.g. "Developer ID Application: Name (TEAMID)":
#   signs with that identity and the hardened runtime.
# - MACOS_PROFILE also set, the path of a Developer ID provisioning profile
#   for the bundle identifier: embeds it and claims the application
#   identifier, which Touch ID for encrypted scratches needs (see below).
#   Without it the app works, but Touch ID is unavailable.
# - APPLE_ID, APPLE_TEAM_ID and APPLE_APP_PASSWORD also set: notarizes the
#   .dmg with Apple and staples the ticket to the .dmg and the app, so
#   Gatekeeper opens it with no warning.
# - Nothing set: ad-hoc signature only. macOS blocks the downloaded app.
set -eu
version="$1"
name="rubrique-$version-macos-universal"
app="dist/Rubrique.app"
dmg="dist/$name.dmg"
identity="${MACOS_SIGN_IDENTITY:-}"
profile="${MACOS_PROFILE:-}"

rm -rf "$app" "$dmg" "dist/$name.zip"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
lipo -create \
    target/aarch64-apple-darwin/release/rubrique \
    target/x86_64-apple-darwin/release/rubrique \
    -output "$app/Contents/MacOS/rubrique"
sed "s/__VERSION__/$version/g" packaging/macos/Info.plist > "$app/Contents/Info.plist"
# App icon: an .iconset from the PNG sizes, turned into .icns by iconutil.
iconset="dist/rubrique.iconset"
rm -rf "$iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
    cp "assets/icon/rubrique-$size.png" "$iconset/icon_${size}x${size}.png"
    cp "assets/icon/rubrique-$((size * 2)).png" "$iconset/icon_${size}x${size}@2x.png"
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/rubrique.icns"
rm -rf "$iconset"

if [ -n "$identity" ]; then
    entitlements="packaging/macos/entitlements.plist"
    if [ -n "$profile" ]; then
        # Touch ID keeps the scratch key in the data protection keychain, which
        # macOS opens only to an app whose application identifier a provisioning
        # profile authorizes. The profile goes into the bundle and the signature
        # claims the identifier (team id, dot, bundle identifier). Without the
        # profile macOS refuses to start an app claiming it, so the two
        # entitlements are added only here, to a copy of the file.
        team="${APPLE_TEAM_ID:-$(printf "%s" "$identity" | sed -n "s/.*(\([A-Z0-9]*\))\$/\1/p")}"
        if [ -z "$team" ]; then
            echo "Cannot tell the team id from the identity; set APPLE_TEAM_ID" >&2
            exit 1
        fi
        bundle_id=$(/usr/libexec/PlistBuddy -c "Print CFBundleIdentifier" "$app/Contents/Info.plist")
        cp "$profile" "$app/Contents/embedded.provisionprofile"
        entitlements="dist/entitlements.plist"
        cp packaging/macos/entitlements.plist "$entitlements"
        # PlistBuddy, not plutil: plutil reads the dots in these keys as a path.
        /usr/libexec/PlistBuddy -c "Add :com.apple.application-identifier string $team.$bundle_id" "$entitlements"
        /usr/libexec/PlistBuddy -c "Add :com.apple.developer.team-identifier string $team" "$entitlements"
    else
        echo "No MACOS_PROFILE: Touch ID for encrypted scratches is unavailable in this build" >&2
    fi
    # Hardened runtime and a secure timestamp: both needed for notarization.
    # The entitlements let the hardened runtime use the microphone.
    codesign --force --options runtime --timestamp \
        --entitlements "$entitlements" --sign "$identity" "$app"
else
    # Ad-hoc signature: keeps Apple Silicon happy, but Gatekeeper rejects it.
    codesign --force --sign - "$app"
fi
codesign --verify --strict --verbose=2 "$app"

# The disk image: the app, a link to /Applications and a background with
# an arrow between them. Finder keeps the window size, the icon size and
# where each icon sits in the image's .DS_Store, and it is the only thing
# that writes that file, so the image is first made writable, laid out by
# Finder through AppleScript, and then converted to the compressed
# read-only image that ships. Without this, Finder opens the image like
# any folder: in the last window size it had, icons in the top left corner.
volume="Rubrique $version"
dmg_root="dist/dmg"
rm -rf "$dmg_root"
mkdir -p "$dmg_root/.background"
cp -R "$app" "$dmg_root/"
ln -s /Applications "$dmg_root/Applications"
cp packaging/macos/background.png "$dmg_root/.background/background.png"
# Room for the .DS_Store and the catalog on top of the files.
size_mb=$(( $(du -sm "$dmg_root" | cut -f1) + 16 ))
rw_dmg="dist/$name-rw.dmg"
rm -f "$rw_dmg"
hdiutil create -volname "$volume" -srcfolder "$dmg_root" -ov -format UDRW -fs HFS+ \
    -size "${size_mb}m" "$rw_dmg"
rm -rf "$dmg_root"

mount_point="/Volumes/$volume"
if [ -d "$mount_point" ]; then
    # Left over from an earlier run: Finder would otherwise mount at "name 1".
    hdiutil detach "$mount_point" -force
fi
# The whole-disk device (/dev/diskN, first line of the attach output) is
# what gets detached later: detaching by mount point breaks when the first
# attempt unmounts the volume but cannot eject the disk, because the mount
# point is gone by then and a retry by path has nothing to find.
attached=$(hdiutil attach -readwrite -noverify -noautoopen "$rw_dmg")
echo "$attached"
device=$(echo "$attached" | awk 'NR == 1 { print $1 }')

# The window is 600 x 400 points with 128 point icons; the icon centers
# match the arrow drawn in background.swift. Finder sometimes answers an
# AppleScript with an error right after a mount, so the layout gets a few
# attempts. The close and open at the end make Finder write .DS_Store.
lay_out_dmg() {
    osascript <<EOF
tell application "Finder"
    tell disk "$volume"
        open
        set current view of container window to icon view
        set toolbar visible of container window to false
        set statusbar visible of container window to false
        set the bounds of container window to {400, 200, 1000, 600}
        set view_options to the icon view options of container window
        set arrangement of view_options to not arranged
        set icon size of view_options to 128
        set text size of view_options to 13
        set background picture of view_options to file ".background:background.png"
        set position of item "Rubrique.app" of container window to {150, 185}
        set position of item "Applications" of container window to {450, 185}
        close
        open
        update without registering applications
        delay 1
        close
    end tell
end tell
EOF
}
attempt=1
while ! lay_out_dmg; do
    if [ "$attempt" -ge 5 ]; then
        echo "Finder could not lay out the disk image after $attempt attempts" >&2
        hdiutil detach "$device" -force || true
        exit 1
    fi
    echo "Finder layout failed, retrying in 3s (attempt $attempt of 5)" >&2
    sleep 3
    attempt=$((attempt + 1))
done
sync
# Finder may still hold the disk for a moment after the window closes
# ("Resource busy"), so the detach gets a few attempts before it is forced.
attempt=1
while ! hdiutil detach "$device"; do
    if [ "$attempt" -ge 5 ]; then
        echo "the disk image is still busy after $attempt attempts, forcing the eject" >&2
        hdiutil detach "$device" -force
        break
    fi
    echo "detach failed, retrying in 2s (attempt $attempt of 5)" >&2
    sleep 2
    attempt=$((attempt + 1))
done
hdiutil convert "$rw_dmg" -format UDZO -imagekey zlib-level=9 -ov -o "$dmg"
rm -f "$rw_dmg"

# stapler fetches the ticket from Apple's CloudKit, and that connection drops
# now and then ("The network connection was lost", exit code 68): it failed
# two release runs in a row on GitHub's runners. Apple's advice is to retry,
# at once and then after a pause, so each stapler call gets a few attempts.
retry_stapler() {
    attempt=1
    while ! xcrun stapler "$@"; do
        if [ "$attempt" -ge 8 ]; then
            echo "stapler $1 failed $attempt times, giving up" >&2
            return 1
        fi
        echo "stapler $1 failed, retrying in 15s (attempt $attempt of 8)" >&2
        sleep 15
        attempt=$((attempt + 1))
    done
}

if [ -n "$identity" ]; then
    codesign --force --timestamp --sign "$identity" "$dmg"
    if [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ] && [ -n "${APPLE_APP_PASSWORD:-}" ]; then
        # Notarizing the .dmg covers the app inside it too, so both get the ticket.
        xcrun notarytool submit "$dmg" \
            --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$APPLE_APP_PASSWORD" \
            --wait --timeout 30m
        retry_stapler staple "$dmg"
        retry_stapler staple "$app"
        retry_stapler validate "$dmg"
        # The same checks Gatekeeper runs on a downloaded app.
        spctl --assess --type execute --verbose=2 "$app"
        spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg"
    fi
fi

# The .zip is made last, so it holds the stapled app.
ditto -c -k --keepParent "$app" "dist/$name.zip"
echo "$dmg dist/$name.zip"
