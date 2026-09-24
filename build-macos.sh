#!/bin/bash
set -e
cd "$(dirname "$0")"

cargo build --release

APP="Trayce.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp target/release/trayce "$APP/Contents/MacOS/trayce"
cp macos/Info.plist "$APP/Contents/Info.plist"

echo "built: $(pwd)/$APP"
