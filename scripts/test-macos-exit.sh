#!/bin/sh
set -eu
project_dir=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
fixture=$(mktemp "${TMPDIR:-/tmp}/ponte-exit-test.XXXXXX")
trap 'rm -f "$fixture"' EXIT INT TERM
swiftc -warnings-as-errors -O "$project_dir/native/macos/PonteDesk.swift" "$project_dir/native/macos/PonteInterface.swift" "$project_dir/native/macos/PonteArtwork.swift" "$project_dir/native/macos/PonteSettings.swift" "$project_dir/native/macos/PonteStorage.swift" "$project_dir/native/macos/StorageNative.swift" "$project_dir/native/macos/storage/StorageProtocol.swift" "$project_dir/native/macos/storage/StorageVolumes.swift" "$project_dir/native/macos/CameraComponent.swift" "$project_dir/native/macos/WebcamCapture.swift" "$project_dir/native/macos/WebcamSink.swift" "$project_dir/tests/app_exit.swift" -o "$fixture" -framework AppKit -framework ApplicationServices -framework AVFoundation -framework VideoToolbox -framework CoreMediaIO -framework CoreMedia -framework CoreVideo -framework ImageIO -framework DiskArbitration -framework NetFS -framework Security -framework ServiceManagement
chmod +x "$fixture"
"$fixture"
