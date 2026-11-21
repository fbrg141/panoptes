#!/bin/sh
# Packs target/release/Panoptes.app (made by install-app.sh) into the release
# .dmg. Its window shows the app next to an Applications shortcut on the
# branded background in crates/panoptes-app/bundle/dmg, so the drag to
# install is obvious. Setting the window up drives Finder, so run it on a
# Mac with a logged-in desktop; macOS asks once to let the terminal control
# Finder.
#
#   sh scripts/install-app.sh --no-open && sh scripts/make-dmg.sh
#
# The background is rendered from bg.html at 640x400 (1x and 2x); re-render
# both PNGs after changing it.
set -eu

cd "$(dirname "$0")/.."

version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
volume="Panoptes $version"
app="target/release/Panoptes.app"
dmg="target/release/Panoptes_${version}_aarch64.dmg"
work="target/release/dmg"
art="crates/panoptes-app/bundle/dmg"

if [ ! -d "$app" ]; then
	echo "$app is missing; run sh scripts/install-app.sh --no-open first" >&2
	exit 1
fi
if [ -d "/Volumes/$volume" ]; then
	echo "/Volumes/$volume is mounted; eject it first" >&2
	exit 1
fi

rm -rf "$work"
mkdir -p "$work/stage/.background"
ditto "$app" "$work/stage/Panoptes.app"
ln -s /Applications "$work/stage/Applications"
tiffutil -cathidpicheck "$art/background.png" "$art/background@2x.png" \
	-out "$work/stage/.background/background.tiff" >/dev/null
cp crates/panoptes-app/bundle/icon.icns "$work/stage/.VolumeIcon.icns"

hdiutil create -volname "$volume" -srcfolder "$work/stage" -fs HFS+ \
	-format UDRW -ov "$work/rw.dmg" >/dev/null
hdiutil attach "$work/rw.dmg" -readwrite -noverify -noautoopen >/dev/null
mount="/Volumes/$volume"
trap 'hdiutil detach "$mount" -force >/dev/null 2>&1 || true' EXIT

# Show the volume's own icon in Finder (needs Xcode's command line tools).
if command -v SetFile >/dev/null 2>&1; then
	SetFile -a C "$mount"
fi

# Window 640x400 of content; icons sit on the background's arrow line.
osascript <<OSA
tell application "Finder"
	tell disk "$volume"
		open
		set current view of container window to icon view
		set toolbar visible of container window to false
		set statusbar visible of container window to false
		set bounds of container window to {200, 120, 840, 548}
		set opts to the icon view options of container window
		set arrangement of opts to not arranged
		set icon size of opts to 104
		set text size of opts to 13
		set background picture of opts to file ".background:background.tiff"
		set position of item "Panoptes.app" of container window to {160, 240}
		set position of item "Applications" of container window to {480, 240}
		close
		open
		update without registering applications
		delay 2
		close
	end tell
end tell
OSA

chmod -Rf go-w "$mount" || true
sync
hdiutil detach "$mount" >/dev/null
trap - EXIT

hdiutil convert "$work/rw.dmg" -format UDZO -imagekey zlib-level=9 -ov -o "$dmg" >/dev/null
hdiutil verify "$dmg" >/dev/null
rm -rf "$work"
echo "Built $dmg"
