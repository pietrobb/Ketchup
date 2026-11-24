#!/bin/bash
# Checks a Linux package from scripts/linux/build-package.sh where it is unpacked:
# every library its programs need is in it or on the system (CI: a clean container
# with only scripts/linux/runtime-packages.txt installed), each worker answers the
# typed Hello, the app verifies its install and reads a document, `ketchup-program`
# writes a static calculation, and the Viewer opens a package. Run from the repository, which has the sample files.
#
#   scripts/linux/check-package.sh Ketchup-<version>-Linux-x86_64.tar.gz
set -euo pipefail

archive="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
repo="$(cd "$(dirname "$0")/../.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
tar -xzf "$archive" -C "$work"
package="$work/$(basename "$archive" .tar.gz)"
[ -d "$package" ] || { echo "the archive has no folder $(basename "$package")" >&2; exit 1; }

# ldd fails when any argument is not a dynamic ELF; under pipefail that failure
# would hide a "not found" from the others, so its status is not the test.
libraries="$(ldd "$package"/ketchup-* "$package"/lib/* 2>&1 || true)"
if grep "not found" <<<"$libraries"; then
    echo "a library the package needs is neither in it nor on the system" >&2
    exit 1
fi

# The typed Hello frame (u32 LE length + CBOR "Hello"), as build-app.sh and the
# Windows installer check it; the reply is {"Hello": {"protocol": <64-character
# text>}} behind its length.
workers=(ketchup-exact-worker ketchup-program-worker)
hello_reply="$(printf '%s' 'a165' '48656c6c6f' 'a168' '70726f746f636f6c' '7840')"
for worker in "${workers[@]}"; do
    reply="$(printf '\x06\x00\x00\x00\x65Hello' | "$package/$worker" | od -An -v -tx1 | tr -d ' \n')"
    if [ "${reply:0:8}" != "53000000" ] || [ "${reply:8:${#hello_reply}}" != "$hello_reply" ] || [ ${#reply} -ne 174 ]; then
        echo "$worker in the package failed its Hello handshake" >&2
        exit 1
    fi
done

cd "$package"
./ketchup-app --verify-install "$work/verify.ketchup"
./ketchup-app --inspect-native-document "$repo/examples/house-project.ketchup" >/dev/null
./ketchup-viewer --verify-open "$repo/viewer/samples/kitchen.ketchup-view"
# The release notes offer `ketchup-program check --calculation sk` (review 2026-11-19
# REL-800). It exits 1 or 3 for a structure that fails or is not verified, 2 when it
# cannot evaluate.
status=0
./ketchup-program check "$repo/examples/programs/tiny-house.star" --calculation sk >"$work/calculation.html" || status=$?
if [ "$status" -eq 2 ] || ! grep -q '<html lang="sk">' "$work/calculation.html"; then
    echo "ketchup-program in the package wrote no static calculation (exit $status)" >&2
    exit 1
fi
echo "OK: $(basename "$archive")"
