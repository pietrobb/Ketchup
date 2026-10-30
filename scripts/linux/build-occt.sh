#!/usr/bin/env bash
# Builds the pinned OCCT 8.0.1 (same module set as scripts/windows/build-occt.ps1
# and scripts/macos/build-occt.sh) into third_party/occt-install-r0-v1 with the
# Unix layout that crates/ketchup-exact/build.rs expects on Linux. The
# libraries find each other through $ORIGIN; a Kečup binary finds them through
# its rpath (README, "Linux").
#
# As on macOS, results must equal the Windows ones bit for bit (ADR 0009):
# clang with -ffp-contract=off, and no transcendental function from glibc's
# libm. Every OCCT library gets third_party/crmath linked in whole (correctly
# rounded CORE-MATH functions, hidden, so each library binds to its own copy),
# and -fno-builtin-* keeps clang from folding those calls with its host math.
# The gate at the end fails the build if any OCCT library still imports one of
# them from the system or exports the private copy.
#
#   git clone https://github.com/Open-Cascade-SAS/OCCT.git third_party/occt-src
#   git -C third_party/occt-src checkout b8f597c677811d1f9f4d8a97f5ae2825c0353a42
#   scripts/linux/build-occt.sh
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../.." && pwd)"
src="$repo_root/third_party/occt-src"
build="$repo_root/third_party/occt-build-r0-v1"
install="$repo_root/third_party/occt-install-r0-v1"
expected_commit="b8f597c677811d1f9f4d8a97f5ae2825c0353a42"

actual_commit="$(git -C "$src" rev-parse HEAD)"
[[ "$actual_commit" == "$expected_commit" ]] || {
    echo "OCCT source must be V8.0.1 commit $expected_commit; found $actual_commit" >&2
    exit 1
}
[[ -z "$(git -C "$src" status --porcelain --untracked-files=all)" ]] || {
    echo "OCCT source checkout has local modifications." >&2
    exit 1
}
for dir in "$build" "$install"; do
    if [[ -d "$dir" && -n "$(ls -A "$dir")" ]]; then
        echo "A clean build requires an empty directory: $dir" >&2
        exit 1
    fi
done

export CC="${CC:-clang}" CXX="${CXX:-clang++}"
jobs="${KETCHUP_OCCT_JOBS:-$(nproc)}"

crmath_build="$build-crmath"
rm -rf "$crmath_build"
cmake -S "$repo_root/third_party/crmath" -B "$crmath_build" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="$install"
cmake --build "$crmath_build"
cmake --install "$crmath_build"
crmath="$install/lib/libketchup_crmath.a"

math_functions=(sin cos tan asin acos atan atan2 sinh cosh tanh asinh acosh atanh
    exp exp2 exp10 log log2 log10 log1p pow cbrt hypot sinf cosf acosf powf tanf)
fp_flags="-ffp-contract=off"
for function in "${math_functions[@]}"; do
    fp_flags+=" -fno-builtin-$function"
done

cmake -S "$src" -B "$build" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_C_FLAGS="$fp_flags" \
    -DCMAKE_CXX_FLAGS="$fp_flags" \
    -DCMAKE_SHARED_LINKER_FLAGS="-Wl,--whole-archive,$crmath,--no-whole-archive -Wl,--exclude-libs,libketchup_crmath.a" \
    -DCMAKE_INSTALL_RPATH='$ORIGIN' \
    -DBUILD_LIBRARY_TYPE=Shared \
    -DBUILD_CPP_STANDARD=C++17 \
    -DBUILD_MODULE_FoundationClasses=ON \
    -DBUILD_MODULE_ModelingData=ON \
    -DBUILD_MODULE_ModelingAlgorithms=ON \
    -DBUILD_MODULE_DataExchange=ON \
    -DBUILD_MODULE_Visualization=OFF \
    -DBUILD_MODULE_ApplicationFramework=OFF \
    -DBUILD_MODULE_Draw=OFF \
    -DBUILD_USE_VCPKG=OFF \
    -DBUILD_USE_PCH=OFF \
    -DBUILD_YACCLEX=OFF \
    -DBUILD_GTEST=OFF \
    -DBUILD_DOC_Overview=OFF \
    -DBUILD_DOC_RefMan=OFF \
    -DBUILD_RELEASE_DISABLE_EXCEPTIONS=OFF \
    -DBUILD_WITH_DEBUG=OFF \
    -DUSE_FREETYPE=OFF \
    -DUSE_FREEIMAGE=OFF \
    -DUSE_FFMPEG=OFF \
    -DUSE_OPENVR=OFF \
    -DUSE_OPENGL=OFF \
    -DUSE_GLES2=OFF \
    -DUSE_D3D=OFF \
    -DUSE_EIGEN=OFF \
    -DUSE_RAPIDJSON=OFF \
    -DUSE_DRACO=OFF \
    -DUSE_TK=OFF \
    -DUSE_TBB=OFF \
    -DUSE_VTK=OFF \
    -DUSE_XLIB=OFF \
    -DINSTALL_DIR_LAYOUT=Unix \
    -DINSTALL_DIR="$install" \
    -DCMAKE_INSTALL_PREFIX="$install" \
    -DUSE_GIT_HASH=ON

cmake --build "$build" --parallel "$jobs"
cmake --install "$build"

# Gate: no OCCT library may take a transcendental function from glibc's libm
# (that is what makes results platform dependent), and none may export the
# private crmath copy. nm -D prints versioned names (sin@GLIBC_2.2.5).
system_math='^(sincosf?|exp10f?|(a?(sin|cos|tan)h?|atan2|exp(2|10|m1)?|log(2|10|1p)?|pow|cbrt|hypot|erfc?|[lt]gamma)f?)$'
status=0
# The real files; libTKernel.so and the like are links to them.
libraries=("$install"/lib/libTK*.so*)
[[ -e "${libraries[0]}" ]] || { echo "no OCCT libraries in $install/lib" >&2; exit 1; }
for library in "${libraries[@]}"; do
    [[ -L "$library" ]] && continue
    if nm -D --undefined-only "$library" | awk '{print $NF}' | sed 's/@.*//' | grep -E "$system_math"; then
        echo "$library still imports system math" >&2
        status=1
    fi
    if nm -D --defined-only "$library" | awk '{print $NF}' | sed 's/@.*//' | grep -E "$system_math"; then
        echo "$library exports crmath functions" >&2
        status=1
    fi
done
exit $status
