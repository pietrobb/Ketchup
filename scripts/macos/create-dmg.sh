#!/bin/bash
# Builds a compressed .dmg from a folder. On a CI runner hdiutil can fail for a
# few seconds while the system still scans the new app ("Resource busy"), and
# -quiet would hide why it failed, so retry and keep its error visible.
#
#   scripts/macos/create-dmg.sh VOLUME_NAME SOURCE_FOLDER DMG
set -euo pipefail

name="$1" source="$2" dmg="$3"
for attempt in 1 2 3 4 5; do
    if hdiutil create -volname "$name" -srcfolder "$source" -format UDZO "$dmg" >/dev/null; then
        exit 0
    fi
    rm -f "$dmg"
    sleep 5
done
echo "cannot create $(basename "$dmg")" >&2
exit 1
