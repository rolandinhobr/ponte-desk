#!/bin/sh
# Inspect the actual distributable without installing it or changing services.
set -eu
project_dir=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
app=$1
stage=$(mktemp -d "${TMPDIR:-/tmp}/ponte-package-test.XXXXXX")
trap 'rm -rf "$stage"' EXIT INT TERM
pkgutil --expand-full "$app/Contents/Resources/PonteDeskStorage.pkg" "$stage/package"
payload="$stage/package/Payload/Library"
cmp "$app/Contents/MacOS/ponte-storage-helper" "$payload/PrivilegedHelperTools/br.com.ponte-desk.storage-helper"
cmp "$project_dir/native/macos/storage/installed-helper.plist" "$payload/LaunchDaemons/br.com.ponte-desk.storage-helper.plist"
cmp "$project_dir/native/macos/storage/installer/preinstall" "$stage/package/Scripts/preinstall"
cmp "$project_dir/native/macos/storage/installer/postinstall" "$stage/package/Scripts/postinstall"
codesign --verify --strict "$payload/PrivilegedHelperTools/br.com.ponte-desk.storage-helper"
plutil -lint "$payload/LaunchDaemons/br.com.ponte-desk.storage-helper.plist"
lsbom -p fMUG "$stage/package/Bom" > "$stage/ownership"
# Ownership in the package, rather than the invoking user's expanded files.
awk -F '\t' '$1 ~ /^\.\/Library\/(PrivilegedHelperTools|LaunchDaemons)\/br\.com\.ponte-desk\.storage-helper(\.plist)?$/ {if ($3 != "root" || $4 != "wheel") exit 1; count++} END {if (count != 2) exit 1}' "$stage/ownership"
printf '%s\n' 'PASS: packaged helper, scripts, daemon and root ownership match the app'
