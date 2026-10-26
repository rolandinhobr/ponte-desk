#!/bin/sh
# Preserve the provider's signed bundle; only runtime device properties change.
set -eu
output_dir=${1:?Usage: prepare-component.sh OUTPUT_DIRECTORY}
stage=$(mktemp -d "${TMPDIR:-/tmp}/ponte-camera.XXXXXX")
mounted=''
cleanup() { if [ -n "$mounted" ]; then hdiutil detach "$mounted" >/dev/null 2>&1 || true; fi; rm -rf "$stage"; }
trap cleanup EXIT INT TERM
curl --fail --location --proto '=https' --tlsv1.2 \
 'https://github.com/adrbn/liveloop/releases/download/v1.3.4/LiveLoop-1.3.4.dmg' -o "$stage/component.dmg"
actual=$(shasum -a 256 "$stage/component.dmg" | cut -d ' ' -f1)
[ "$actual" = '91850c858e83804128f191d9607f56cb8e2d3ac658c2ad0e6dad3270589c6946' ] || { echo 'Camera component checksum mismatch' >&2; exit 1; }
mkdir -p "$stage/mount" "$output_dir"
hdiutil attach -nobrowse -readonly -mountpoint "$stage/mount" "$stage/component.dmg" >/dev/null
mounted="$stage/mount"
ditto --norsrc --noextattr --qtn "$mounted/LiveLoop.app" "$output_dir/CameraComponent.app"
component="$output_dir/CameraComponent.app"
codesign --verify --deep --strict "$component"
team=$(codesign --display -v "$component" 2>&1 | sed -n 's/^TeamIdentifier=//p')
[ "$team" = '2TWQF4T93E' ] || { echo 'Unexpected camera component signing team' >&2; exit 1; }
printf '%s\n' "$component"
