cmake_minimum_required(VERSION 3.16)
project(quicmedia LANGUAGES CXX VERSION 0.1.0)

# Directory holding the prebuilt native library (libquicmedia.so/.dylib/.a or
# quicmedia.dll) produced by `cargo build -p quicmedia-ffi`. Defaults to the
# workspace release output.
set(QUICMEDIA_LIB_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../rust/target/release"
    CACHE PATH "Directory containing the prebuilt quicmedia native library")

# The C ABI header lives in the Rust crate; the C++ wrapper sits beside this file.
set(QUICMEDIA_C_HEADER_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../rust/include")
set(QUICMEDIA_CXX_HEADER_DIR "${CMAKE_CURRENT_SOURCE_DIR}/include")

find_library(QUICMEDIA_LIBRARY NAMES quicmedia PATHS "${QUICMEDIA_LIB_DIR}" REQUIRED)

# Imported target consumers link against: quicmedia::quicmedia.
add_library(quicmedia_native SHARED IMPORTED GLOBAL)
set_target_properties(quicmedia_native PROPERTIES IMPORTED_LOCATION "${QUICMEDIA_LIBRARY}")
target_include_directories(quicmedia_native INTERFACE
    "${QUICMEDIA_CXX_HEADER_DIR}" "${QUICMEDIA_C_HEADER_DIR}")
add_library(quicmedia::quicmedia ALIAS quicmedia_native)

option(QUICMEDIA_BUILD_EXAMPLES "Build the C++ examples" ON)
if(QUICMEDIA_BUILD_EXAMPLES)
    add_executable(quicmedia_demo examples/quicmedia_demo.cpp)
    target_compile_features(quicmedia_demo PRIVATE cxx_std_17)
    target_link_libraries(quicmedia_demo PRIVATE quicmedia::quicmedia)
endif()

# ---- Install a relocatable package (headers + config), analogous to the NuGet. ----
include(GNUInstallDirs)
install(FILES
    "${QUICMEDIA_CXX_HEADER_DIR}/quicmedia/quicmedia.hpp"
    "${QUICMEDIA_C_HEADER_DIR}/quicmedia.h"
    DESTINATION "${CMAKE_INSTALL_INCLUDEDIR}/quicmedia")
install(FILES "${QUICMEDIA_LIBRARY}" DESTINATION "${CMAKE_INSTALL_LIBDIR}")
install(FILES "${CMAKE_CURRENT_SOURCE_DIR}/packaging/quicmediaConfig.cmake"
    DESTINATION "${CMAKE_INSTALL_LIBDIR}/cmake/quicmedia")
