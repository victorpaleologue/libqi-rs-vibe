cmake_minimum_required(VERSION 3.16)
project(libqi_interop_cpp LANGUAGES CXX)

# Standalone C++ interoperability harness built against the reference libqi
# (Aldebaran/SoftBank "qi" framework, tag qi-framework-v4.0.5).
#
# Configure with:
#   cmake -S interop/cpp -B interop/cpp/build \
#         -DCMAKE_PREFIX_PATH=/home/user/aldebaran/libqi/install
#   cmake --build interop/cpp/build
#
# qi-dump-values needs one *private* libqi header (src/messaging/message.hpp,
# src/messaging/streamcontext.hpp) to build qi::Message objects. Point
# LIBQI_SOURCE_DIR at the libqi source tree if it is not at the default place.

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
# libqi's qiLog* macros use GNU variadic-macro extensions (libqi builds with CXX_EXTENSIONS TRUE).
set(CMAKE_CXX_EXTENSIONS ON)

if(NOT CMAKE_BUILD_TYPE)
  set(CMAKE_BUILD_TYPE RelWithDebInfo CACHE STRING "Build type" FORCE)
endif()

find_package(qi 4.0 REQUIRED)

set(LIBQI_SOURCE_DIR "/home/user/aldebaran/libqi" CACHE PATH
    "libqi source tree (private headers under src/ are needed by qi-dump-values)")
if(NOT EXISTS "${LIBQI_SOURCE_DIR}/src/messaging/message.hpp")
  message(FATAL_ERROR
    "LIBQI_SOURCE_DIR=${LIBQI_SOURCE_DIR} does not contain src/messaging/message.hpp")
endif()

add_library(interop_common INTERFACE)
target_include_directories(interop_common INTERFACE ${CMAKE_CURRENT_SOURCE_DIR}/common)
target_link_libraries(interop_common INTERFACE qi::qi)
target_compile_options(interop_common INTERFACE -Wall -Wextra -Wno-unused-parameter)

add_executable(qi-dump-values dump_values.cpp)
target_link_libraries(qi-dump-values PRIVATE interop_common)
target_include_directories(qi-dump-values PRIVATE ${LIBQI_SOURCE_DIR}/src)

add_executable(qi-cpp-service service.cpp)
target_link_libraries(qi-cpp-service PRIVATE interop_common)

add_executable(qi-cpp-client client.cpp)
target_link_libraries(qi-cpp-client PRIVATE interop_common)

add_executable(qi-cpp-sd sd.cpp)
target_link_libraries(qi-cpp-sd PRIVATE interop_common)

add_executable(qi-cpp-echo-client echo_client.cpp)
target_link_libraries(qi-cpp-echo-client PRIVATE interop_common)
