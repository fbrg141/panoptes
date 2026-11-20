#!/bin/sh
# Builds Panoptes in release mode and installs it to /Applications.
#
# The app is signed ad hoc (no Apple developer account), which is enough for
# a build made and run on this Mac: it never gets a quarantine flag, so
# Gatekeeper does not ask. Run it again after each merge to update.
#
#   sh scripts/install-app.sh            # build and install, then open the app
#   sh scripts/install-app.sh --no-open  # build and install only
set -eu

cd "$(dirname "$0")/.."

open_after=1
for arg in "$@"; do
	case "$arg" in
	--no-open) open_after=0 ;;
	*)
		echo "usage: $0 [--no-open]" >&2
		exit 2
		;;
	esac
done

# Apple's /usr/bin/git and some shells set SDKROOT to a Command Line Tools
# SDK that breaks Rust's linking. Cargo may also not be on PATH yet.
unset SDKROOT
PATH="$HOME/.cargo/bin:$PATH"

app="Panoptes.app"
built="target/release/$app"
installed="/Applications/$app"
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)

cargo build --release -p panoptes-app

rm -rf "$built"
mkdir -p "$built/Contents/MacOS" "$built/Contents/Resources"
cp target/release/panoptes "$built/Contents/MacOS/panoptes"
cp crates/panoptes-app/bundle/icon.icns "$built/Contents/Resources/icon.icns"
cat >"$built/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleName</key>
	<string>Panoptes</string>
	<key>CFBundleDisplayName</key>
	<string>Panoptes</string>
	<key>CFBundleIdentifier</key>
	<string>dev.panoptes.app</string>
	<key>CFBundleExecutable</key>
	<string>panoptes</string>
	<key>CFBundleIconFile</key>
	<string>icon</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>$version</string>
	<key>CFBundleVersion</key>
	<string>$version</string>
	<key>LSApplicationCategoryType</key>
	<string>public.app-category.developer-tools</string>
	<key>LSMinimumSystemVersion</key>
	<string>13.0</string>
	<key>NSHighResolutionCapable</key>
	<true/>
</dict>
</plist>
PLIST

codesign --force --deep --sign - "$built"
codesign --verify --deep --strict "$built"

# Replacing the bundle under a running app leaves it half old, half new.
if pgrep -xq panoptes; then
	echo "Quitting the running Panoptes…"
	osascript -e 'quit app "Panoptes"' || true
	for _ in 1 2 3 4 5 6 7 8 9 10; do
		pgrep -xq panoptes || break
		sleep 1
	done
	if pgrep -xq panoptes; then
		echo "Panoptes is still running; quit it and run this again." >&2
		exit 1
	fi
fi

rm -rf "$installed"
ditto "$built" "$installed"
echo "Installed $installed ($(git rev-parse --short HEAD))"

if [ "$open_after" = 1 ]; then
	open "$installed"
fi
