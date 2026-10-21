# quicmediaConfig.cmake — find_package config for an installed quicmedia C++ package.
#
# Usage in a consumer project:
#   find_package(quicmedia REQUIRED)
#   target_link_libraries(myapp PRIVATE quicmedia::quicmedia)
#
# Expects this file at <prefix>/lib/cmake/quicmedia/, headers at
# <prefix>/include/quicmedia/, and the native library at <prefix>/lib/.
get_filename_component(_qm_prefix "${CMAKE_CURRENT_LIST_DIR}/../../.." ABSOLUTE)

find_library(QUICMEDIA_LIBRARY NAMES quicmedia PATHS "${_qm_prefix}/lib" NO_DEFAULT_PATH)
if(NOT QUICMEDIA_LIBRARY)
    find_library(QUICMEDIA_LIBRARY NAMES quicmedia)
endif()

if(NOT QUICMEDIA_LIBRARY)
    set(quicmedia_FOUND FALSE)
    if(quicmedia_FIND_REQUIRED)
        message(FATAL_ERROR "quicmedia native library not found under ${_qm_prefix}/lib")
    endif()
    return()
endif()

if(NOT TARGET quicmedia::quicmedia)
    add_library(quicmedia::quicmedia SHARED IMPORTED)
    set_target_properties(quicmedia::quicmedia PROPERTIES
        IMPORTED_LOCATION "${QUICMEDIA_LIBRARY}"
        INTERFACE_INCLUDE_DIRECTORIES "${_qm_prefix}/include")
endif()

set(quicmedia_FOUND TRUE)
