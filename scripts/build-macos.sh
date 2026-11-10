#!/bin/sh
set -eu
project_dir=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
# Build one lightweight native application with an original signed camera component.
cargo build --manifest-path "$project_dir/Cargo.toml" --release --no-default-features -j 1
target_dir=${CARGO_TARGET_DIR:-"$project_dir/target"}
stage=$(mktemp -d "${TMPDIR:-/tmp}/ponte-desk-build.XXXXXX")
trap 'rm -rf "$stage"' EXIT INT TERM
app="$stage/Ponte Desk.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Frameworks" "$app/Contents/Helpers"
swiftc -emit-library -O -module-name PonteDeskUI "$project_dir/native/macos/PonteDesk.swift" "$project_dir/native/macos/PonteInterface.swift" "$project_dir/native/macos/PonteArtwork.swift" "$project_dir/native/macos/PonteSettings.swift" "$project_dir/native/macos/PonteStorage.swift" "$project_dir/native/macos/StorageNative.swift" "$project_dir/native/macos/storage/StorageProtocol.swift" "$project_dir/native/macos/storage/StorageVolumes.swift" "$project_dir/native/macos/CameraComponent.swift" "$project_dir/native/macos/WebcamCapture.swift" "$project_dir/native/macos/WebcamSink.swift" -o "$app/Contents/Frameworks/PonteDeskUI.dylib" -framework AppKit -framework ApplicationServices -framework AVFoundation -framework VideoToolbox -framework CoreMediaIO -framework CoreMedia -framework CoreVideo -framework ImageIO -framework DiskArbitration -framework NetFS -framework Security -framework ServiceManagement
mkdir -p "$app/Contents/Library/LaunchDaemons"
swiftc -O -parse-as-library "$project_dir/native/macos/storage/StorageProtocol.swift" "$project_dir/native/macos/storage/StorageVolumes.swift" "$project_dir/native/macos/storage/StorageHelper.swift" -o "$app/Contents/MacOS/ponte-storage-helper" -framework DiskArbitration -framework Security
codesign --force --sign - --identifier br.com.ponte-desk.storage-helper "$app/Contents/MacOS/ponte-storage-helper"
cp "$project_dir/native/macos/storage/br.com.ponte-desk.storage-helper.plist" "$app/Contents/Library/LaunchDaemons/"
cp "$target_dir/release/ponte-desk" "$app/Contents/MacOS/ponte-desk"
cp "$project_dir/native/macos/Info.plist" "$app/Contents/Info.plist"
mkdir -p "$app/Contents/Resources"
sh "$project_dir/scripts/build-macos-storage-package.sh" "$app/Contents/MacOS/ponte-storage-helper" "$app/Contents/Resources/PonteDeskStorage.pkg"
swiftc -O "$project_dir/native/macos/PonteArtwork.swift" "$project_dir/native/macos/render-icon.swift" -o "$stage/render-icon" -framework AppKit
"$stage/render-icon" "$stage/PonteDesk.iconset"
iconutil --convert icns "$stage/PonteDesk.iconset" --output "$app/Contents/Resources/PonteDesk.icns"
if [ -n "${PONTE_DESK_CAMERA_HELPER:-}" ]; then
  ditto --norsrc --noextattr --qtn "$PONTE_DESK_CAMERA_HELPER" "$app/Contents/Helpers/CameraComponent.app"
else
  "$project_dir/native/macos/camera-extension/prepare-component.sh" "$app/Contents/Helpers"
fi
# Finder/File Provider can attach these unsigned presentation attributes while
# copying a bundle. Remove only those attributes, never quarantine or signatures.
xattr -dr com.apple.FinderInfo "$app" 2>/dev/null || true
xattr -dr com.apple.ResourceFork "$app" 2>/dev/null || true
codesign --verify --deep --strict "$app/Contents/Helpers/CameraComponent.app"
codesign --force --sign - "$app/Contents/Frameworks/PonteDeskUI.dylib"
# Deliberately do not use --deep --force: the vendor helper keeps its signature.
codesign --force --sign - "$app"
codesign --verify --deep --strict "$app"
output_dir=${PONTE_DESK_OUTPUT_DIR:-"$project_dir/dist"}
mkdir -p "$output_dir"
published=$(mktemp -d "$output_dir/.ponte-desk.XXXXXX")
ditto --norsrc --noextattr --qtn "$app" "$published/Ponte Desk.app"
xattr -dr com.apple.FinderInfo "$published/Ponte Desk.app" 2>/dev/null || true
xattr -dr com.apple.ResourceFork "$published/Ponte Desk.app" 2>/dev/null || true
codesign --verify --deep --strict "$published/Ponte Desk.app"
sh "$project_dir/scripts/test-macos-storage-package.sh" "$published/Ponte Desk.app"
if [ -e "$output_dir/Ponte Desk.app" ]; then
  mv "$output_dir/Ponte Desk.app" "$output_dir/Ponte Desk.previous.$(date +%s).app"
fi
mv "$published/Ponte Desk.app" "$output_dir/Ponte Desk.app"
rmdir "$published"
printf '%s\n' "$output_dir/Ponte Desk.app"
