#!/bin/bash
# Fails unless every third-party notice is in the .dmg, beside the app and in
# its Contents/Resources, and not empty (review 2026-11-01 PL-62).
#
#   scripts/macos/check-notices.sh DMG APP_NAME NOTICE...
set -euo pipefail

dmg="$1" app="$2"
shift 2
mount="$(mktemp -d)"
# A fresh image can be busy for a few seconds while the system scans it, and
# -quiet would hide why hdiutil failed, so retry and keep its error visible.
for attempt in 1 2 3 4 5; do
    if hdiutil attach -nobrowse -readonly -mountpoint "$mount" "$dmg" >/dev/null; then
        break
    fi
    if [ "$attempt" -eq 5 ]; then
        echo "cannot mount $(basename "$dmg") to check its notices" >&2
        exit 1
    fi
    sleep 5
done
trap 'hdiutil detach -quiet "$mount" || hdiutil detach -force "$mount"; rmdir "$mount"' EXIT
for notice in "$@"; do
    for path in "$mount/$notice" "$mount/$app/Contents/Resources/$notice"; do
        if [ ! -s "$path" ]; then
            echo "$(basename "$dmg") lacks the third-party notice ${path#"$mount/"}" >&2
            exit 1
        fi
    done
done
