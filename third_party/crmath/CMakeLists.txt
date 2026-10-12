# Static library "ketchup_crmath": correctly rounded libm replacements (see
# ketchup_crmath.c). Built and installed next to OCCT by
# scripts/macos/build-occt.sh and scripts/windows/build-occt.ps1, which link
# it into every OCCT library; crates/ketchup-exact/build.rs links it into the
# Rust binaries. CORE-MATH uses GCC extensions (__int128, __builtin_*), so on
# Windows it is compiled with clang-cl; the objects link with MSVC code.
#
# CORE-MATH source: https://gitlab.inria.fr/core-math/core-math
# commit 81a899d67953a619d703660fd9630f0554673524 (MIT, core-math/LICENSE).
cmake_minimum_required(VERSION 3.20)
project(ketchup_crmath C)

set(core "${CMAKE_CURRENT_SOURCE_DIR}/core-math/src")
set(functions
    binary64/sin/sin binary64/cos/cos binary64/tan/tan binary64/sincos/sincos
    binary64/asin/asin binary64/acos/acos binary64/atan/atan binary64/atan2/atan2
    binary64/sinh/sinh binary64/cosh/cosh binary64/tanh/tanh
    binary64/asinh/asinh binary64/acosh/acosh binary64/atanh/atanh
    binary64/exp/exp binary64/exp2/exp2 binary64/exp10/exp10
    binary64/log/log binary64/log2/log2 binary64/log10/log10 binary64/log1p/log1p
    binary64/pow/pow binary64/cbrt/cbrt binary64/hypot/hypot
    binary32/acos/acosf binary32/pow/powf binary32/tan/tanf)

add_library(ketchup_crmath STATIC ketchup_crmath.c)
foreach(function IN LISTS functions)
    get_filename_component(name "${function}" NAME)
    # Each CORE-MATH function is its own translation unit with private
    # helpers and headers of the same name, so each gets its own object
    # library and include directory.
    add_library(crmath_${name} OBJECT "${core}/${function}.c")
    get_filename_component(dir "${core}/${function}" DIRECTORY)
    target_include_directories(crmath_${name} PRIVATE "${dir}")
    target_compile_definitions(crmath_${name} PRIVATE CORE_MATH_SUPPORT_ERRNO)
    target_sources(ketchup_crmath PRIVATE $<TARGET_OBJECTS:crmath_${name}>)
    list(APPEND parts crmath_${name})
endforeach()

if(NOT CMAKE_C_COMPILER_ID MATCHES "Clang")
    message(FATAL_ERROR "CORE-MATH needs clang (clang-cl on Windows); found ${CMAKE_C_COMPILER_ID}")
endif()
# The shim defines sin, cos, ... itself; the compiler must not treat them as
# builtins or fold calls to them with the host libm. No contraction into FMA,
# so the code runs exactly as written on every target.
set(flags -fno-builtin -ffp-contract=off)
if(CMAKE_C_COMPILER_FRONTEND_VARIANT STREQUAL "MSVC")
    list(TRANSFORM flags PREPEND "/clang:")
endif()

foreach(target IN ITEMS ketchup_crmath ${parts})
    set_target_properties(${target} PROPERTIES
        C_STANDARD 11
        POSITION_INDEPENDENT_CODE ON
        # Each OCCT library gets its own private copy; nothing is exported.
        C_VISIBILITY_PRESET hidden)
    target_compile_options(${target} PRIVATE ${flags})
endforeach()

install(TARGETS ketchup_crmath ARCHIVE DESTINATION lib)
