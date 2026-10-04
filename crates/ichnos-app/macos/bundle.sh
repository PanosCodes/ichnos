#!/bin/sh
# Wraps the installed `ichnos-app` in `~/Applications/Ichnos.app`, so the viewer has its icon and
# name in the Dock and the app switcher. The binary moves into the bundle and `~/.cargo/bin` keeps
# a symlink to it, so `ichnos open` and `PATH` lookups still find it.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
bin="${CARGO_HOME:-$HOME/.cargo}/bin/ichnos-app"
app="$HOME/Applications/Ichnos.app"
version=$(cargo pkgid -p ichnos-app | sed 's/.*[#@]//')

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
sed "s/VERSION/$version/" "$here/Info.plist" > "$app/Contents/Info.plist"
cp "$here/Ichnos.icns" "$app/Contents/Resources/"
mv "$bin" "$app/Contents/MacOS/ichnos-app"
ln -s "$app/Contents/MacOS/ichnos-app" "$bin"
