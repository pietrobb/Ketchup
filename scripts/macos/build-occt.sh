#!/usr/bin/env bash
# Builds the pinned OCCT 8.0.1 (same module set as scripts/windows/build-occt.ps1)
# into third_party/occt-install-r0-v1 with the Unix layout that
# crates/ketchup-exact/build.rs expects on macOS. The libraries carry their
# absolute install path, so every workspace binary and test finds them without
# copying (Windows copies the DLLs instead); build-app.sh rewrites the paths
# to the app bundle.
#
# -ffp-contract=off: clang on arm64 otherwise fuses a*b+c into one FMA, which
# rounds differently from MSVC on x64. Results would then differ in the last
# bit, and documents pin exact results bit for bit (result_fingerprint hashes
# the volume's bits), so a Windows document would not evaluate on a Mac.
#
# For the same reason OCCT does not use the system libm: Apple's and
# Microsoft's sin, cos, pow, ... differ in the last bit. Every OCCT library
# gets third_party/crmath linked in (correctly rounded CORE-MATH functions,
# identical on every platform), and -fno-builtin-* keeps clang from folding
# or fusing those calls with its own host math. The gate at the end fails
# the build if any OCCT library still imports one of them from the system.
#
#   git clone https://github.com/Open-Cascade-SAS/OCCT.git third_party/occt-src
#   git -C third_party/occt-src checkout b8f597c677811d1f9f4d8a97f5ae2825c0353a42
#   scripts/macos/build-occt.sh
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

archs="${KETCHUP_OCCT_ARCHS:-arm64}"
crmath_build="$build-crmath"
rm -rf "$crmath_build"
cmake -S "$repo_root/third_party/crmath" -B "$crmath_build" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_OSX_ARCHITECTURES="$archs" \
    -DCMAKE_OSX_DEPLOYMENT_TARGET=11.0 \
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
    -DCMAKE_SHARED_LINKER_FLAGS="-Wl,-force_load,$crmath -Wl,-dead_strip" \
    -DCMAKE_OSX_ARCHITECTURES="$archs" \
    -DCMAKE_OSX_DEPLOYMENT_TARGET=11.0 \
    -DCMAKE_MACOSX_RPATH=ON \
    -DINSTALL_NAME_DIR="$install/lib" \
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
    -DINSTALL_DIR_LAYOUT=Unix \
    -DINSTALL_DIR="$install" \
    -DCMAKE_INSTALL_PREFIX="$install" \
    -DUSE_GIT_HASH=ON

cmake --build "$build" --parallel "$(sysctl -n hw.ncpu)"
cmake --install "$build"

# Gate: no OCCT library may take a transcendental function from the system
# libm (that is what made results platform dependent), and none may export the
# private crmath copy.
system_math='^_(_sincos_stret|_sincospi_stret|_exp10|(a?(sin|cos|tan)h?|atan2|exp(2|10|m1)?|log(2|10|1p)?|pow|cbrt|hypot|erfc?|[lt]gamma)f?)$'
status=0
for library in "$install"/lib/libTK*.dylib; do
    [[ -L "$library" ]] && continue
    if nm -u "$library" | grep -E "$system_math"; then
        echo "$library still imports system math" >&2
        status=1
    fi
    if nm -gU "$library" | awk '{print $3}' | grep -E "$system_math"; then
        echo "$library exports crmath functions" >&2
        status=1
    fi
done
exit $status
