#!/bin/sh
# Packs target/release/Panoptes.app (made by install-app.sh) into a .dmg
# with an Applications shortcut, so the window shows where to drag the app.
#
#   sh scripts/install-app.sh --no-open && sh scripts/make-dmg.sh
set -eu

cd "$(dirname "$0")/.."

version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
app="target/release/Panoptes.app"
dmg="target/release/Panoptes_${version}_aarch64.dmg"
stage="target/release/dmg"

if [ ! -d "$app" ]; then
	echo "$app is missing; run sh scripts/install-app.sh --no-open first" >&2
	exit 1
fi

rm -rf "$stage"
mkdir -p "$stage"
ditto "$app" "$stage/Panoptes.app"
ln -s /Applications "$stage/Applications"

hdiutil create -volname "Panoptes $version" -srcfolder "$stage" -ov -format UDZO "$dmg"
hdiutil verify "$dmg"
rm -rf "$stage"
echo "Built $dmg"
