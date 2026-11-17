#!/bin/bash
# Builds Ketchup-<version>-Linux-x86_64.tar.gz: the desktop app, its exact and program
# workers, the Viewer and the pinned OCCT libraries in lib/ beside them, with the
# third-party notices. Needs OCCT from scripts/linux/build-occt.sh, patchelf, and
# cargo-about 0.9.2 for the notices. scripts/linux/check-package.sh then checks the
# archive; CI runs it in a clean container (.github/workflows/linux-package.yml).
#
#   scripts/linux/build-package.sh [--output DIR]
set -euo pipefail

repo="$(cd "$(dirname "$0")/../.." && pwd)"
version="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' "$repo/Cargo.toml")"
output="$repo/artifacts/linux-package/$version"
while [ $# -gt 0 ]; do
    case "$1" in
        --output) output="$2"; shift 2 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

name="Ketchup-$version-Linux-x86_64"
occt="$repo/third_party/occt-install-r0-v1"
occt_lib="$occt/lib"
[ -d "$occt_lib" ] || { echo "missing OCCT: run scripts/linux/build-occt.sh" >&2; exit 1; }
command -v patchelf >/dev/null || { echo "missing patchelf" >&2; exit 1; }

cd "$repo"
cargo build --locked --release -p ketchup-app --bin ketchup-app -p ketchup-viewer --bin ketchup-viewer -p ketchup-scheduler --bin ketchup-exact-worker --bin ketchup-program-worker

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
package="$work/$name"
mkdir -p "$package/lib"
programs=(ketchup-app ketchup-viewer ketchup-exact-worker ketchup-program-worker)
for program in "${programs[@]}"; do
    cp "target/release/$program" "$package/"
done

# The OCCT libraries the programs load, directly or through each other, by the
# name they ask for (the soname, a link to the versioned file in the install).
occt_dependencies() {
    readelf -d "$1" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p' | while read -r library; do
        if [ -e "$occt_lib/$library" ]; then echo "$library"; fi
    done
}
queue=("${programs[@]/#/$package/}")
while [ ${#queue[@]} -gt 0 ]; do
    image="${queue[0]}"
    queue=("${queue[@]:1}")
    for library in $(occt_dependencies "$image"); do
        if [ ! -e "$package/lib/$library" ]; then
            cp -L "$occt_lib/$library" "$package/lib/$library"
            chmod u+w,a-x "$package/lib/$library"
            queue+=("$package/lib/$library")
        fi
    done
done
[ -n "$(ls "$package/lib")" ] || { echo "the programs load no OCCT library" >&2; exit 1; }

# Each program finds OCCT in lib/ beside it, each library the others beside it, and
# nothing on the build machine.
for program in "${programs[@]}"; do
    patchelf --set-rpath '$ORIGIN/lib' "$package/$program"
done
for library in "$package"/lib/*; do
    patchelf --set-rpath '$ORIGIN' "$library"
done
if readelf -d "$package"/ketchup-* "$package"/lib/* | grep -E '(RUNPATH|RPATH)' | grep -F "$repo"; then
    echo "a packaged program or library still references the build machine" >&2
    exit 1
fi

# Third-party notices (review 2026-11-01 PL-62), as in the Windows and macOS packages.
python3 scripts/third_party_notices.py --package ketchup-linux --output "$package"
cp "$occt/LICENSE_LGPL_21.txt" "$package/LICENSE-OCCT-LGPL-2.1.txt"
cp "$occt/OCCT_LGPL_EXCEPTION.txt" "$package/LICENSE-OCCT-LGPL-EXCEPTION.txt"
cp third_party/starlark/LICENSE "$package/LICENSE-Starlark-Apache-2.0.txt"
cp third_party/crmath/core-math/LICENSE "$package/LICENSE-CORE-MATH-MIT.txt"
cp LICENSE "$package/LICENSE.txt"

mkdir -p "$output"
archive="$output/$name.tar.gz"
rm -f "$archive"
# Owned by root and dated by the commit, so the same source gives the same archive.
tar --sort=name --owner=0 --group=0 --numeric-owner \
    --mtime="@$(git log -1 --format=%ct)" -C "$work" -cf - "$name" | gzip -n -9 > "$archive"
echo "$(ls "$package/lib" | wc -l) OCCT libraries packaged"
scripts/linux/check-package.sh "$archive"
echo "Built $archive"
