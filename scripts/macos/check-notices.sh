#!/bin/bash
# Fails unless every third-party notice is in the .dmg, beside the app and in
# its Contents/Resources, and not empty (review 2026-11-01 PL-62).
#
#   scripts/macos/check-notices.sh DMG APP_NAME NOTICE...
set -euo pipefail

dmg="$1" app="$2"
shift 2
mount="$(mktemp -d)"
hdiutil attach -quiet -nobrowse -readonly -mountpoint "$mount" "$dmg"
trap 'hdiutil detach -quiet "$mount"; rmdir "$mount"' EXIT
for notice in "$@"; do
    for path in "$mount/$notice" "$mount/$app/Contents/Resources/$notice"; do
        if [ ! -s "$path" ]; then
            echo "$(basename "$dmg") lacks the third-party notice ${path#"$mount/"}" >&2
            exit 1
        fi
    done
done
