#!/bin/sh
# Root ownership is established by Installer, never by trusting a user-writable
# daemon in the application bundle. No Apple subscription is required.
set -eu
project_dir=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
helper=$1
destination=$2
stage=$(mktemp -d "${TMPDIR:-/tmp}/ponte-storage-package.XXXXXX")
trap 'rm -rf "$stage"' EXIT INT TERM
mkdir -p "$stage/root/Library/PrivilegedHelperTools" "$stage/root/Library/LaunchDaemons" "$stage/scripts"
cp -X "$helper" "$stage/root/Library/PrivilegedHelperTools/br.com.ponte-desk.storage-helper"
cp -X "$project_dir/native/macos/storage/installed-helper.plist" "$stage/root/Library/LaunchDaemons/br.com.ponte-desk.storage-helper.plist"
cp -X "$project_dir/native/macos/storage/installer/preinstall" "$project_dir/native/macos/storage/installer/postinstall" "$stage/scripts/"
chmod 755 "$stage/scripts/preinstall" "$stage/scripts/postinstall" "$stage/root/Library/PrivilegedHelperTools/br.com.ponte-desk.storage-helper"
chmod 644 "$stage/root/Library/LaunchDaemons/br.com.ponte-desk.storage-helper.plist"
COPYFILE_DISABLE=1 pkgbuild --root "$stage/root" --scripts "$stage/scripts" --ownership recommended \
  --identifier br.com.ponte-desk.storage-package --version 0.5.1 --install-location / "$destination"
