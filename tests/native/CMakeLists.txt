cmake_minimum_required(VERSION 3.16)
project(ketchup_occt_smoke LANGUAGES CXX)

if(NOT DEFINED OCCT_ROOT)
    message(FATAL_ERROR "OCCT_ROOT must point to the frozen OCCT install tree")
endif()

add_executable(occt-smoke occt-smoke.cpp)
target_compile_features(occt-smoke PRIVATE cxx_std_17)
target_include_directories(occt-smoke PRIVATE "${OCCT_ROOT}/inc")
target_link_directories(occt-smoke PRIVATE "${OCCT_ROOT}/win64/vc14/lib")
target_link_libraries(
    occt-smoke
    PRIVATE
        TKernel
        TKMath
        TKG2d
        TKG3d
        TKGeomBase
        TKBRep
        TKGeomAlgo
        TKTopAlgo
        TKPrim
)

add_executable(r0-step-corpus r0-step-corpus.cpp)
target_compile_features(r0-step-corpus PRIVATE cxx_std_17)
target_include_directories(r0-step-corpus PRIVATE "${OCCT_ROOT}/inc")
target_link_directories(r0-step-corpus PRIVATE "${OCCT_ROOT}/win64/vc14/lib")
target_link_libraries(
    r0-step-corpus
    PRIVATE
        TKernel
        TKMath
        TKG2d
        TKG3d
        TKGeomBase
        TKBRep
        TKGeomAlgo
        TKTopAlgo
        TKPrim
        TKBO
        TKDE
        TKXSBase
        TKDESTEP
)
