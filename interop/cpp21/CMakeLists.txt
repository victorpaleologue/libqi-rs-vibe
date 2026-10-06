cmake_minimum_required(VERSION 2.8.12)
project(libqi21_interop_cpp CXX)

# Interoperability harness built against the libqi of NAOqi 2.1 (libqi 2.1.3,
# libqitype and libqimessaging at the 2.1 fork point, June 2014). See README.md:
# build-libqi-2.1.sh builds that stack and this harness in an Ubuntu 14.04
# container.
#
#   cmake -S interop/cpp21 -B interop/cpp21/build -DLIBQI21_INSTALL_DIR=/path/to/install21
#
# The programs mirror those of interop/cpp with the 2014 API. qi-dump-values

set(LIBQI21_INSTALL_DIR "" CACHE PATH "libqi 2.1 install prefix (include/, lib/)")
set(LIBQI21_DEPS_DIR "${LIBQI21_INSTALL_DIR}/lib-deps" CACHE PATH
    "directory of the Boost/ICU shared libraries the 2.1 stack was built with")
if(NOT EXISTS "${LIBQI21_INSTALL_DIR}/include/qimessaging/session.hpp")
  message(FATAL_ERROR "LIBQI21_INSTALL_DIR=${LIBQI21_INSTALL_DIR} has no include/qimessaging/session.hpp")
endif()

if(NOT CMAKE_BUILD_TYPE)
  set(CMAKE_BUILD_TYPE RelWithDebInfo CACHE STRING "Build type" FORCE)
endif()
set(CMAKE_CXX_FLAGS "${CMAKE_CXX_FLAGS} -std=gnu++11 -Wall -Wno-unused-parameter -Wno-deprecated-declarations")
# The programs run outside the build container: embed the library paths.
set(CMAKE_SKIP_BUILD_RPATH FALSE)
set(CMAKE_BUILD_RPATH "${LIBQI21_INSTALL_DIR}/lib;${LIBQI21_DEPS_DIR}")
set(CMAKE_INSTALL_RPATH "${LIBQI21_INSTALL_DIR}/lib;${LIBQI21_DEPS_DIR}")
set(CMAKE_EXE_LINKER_FLAGS "${CMAKE_EXE_LINKER_FLAGS} -Wl,-rpath,${LIBQI21_INSTALL_DIR}/lib -Wl,-rpath,${LIBQI21_DEPS_DIR}")

include_directories(${LIBQI21_INSTALL_DIR}/include ${CMAKE_CURRENT_SOURCE_DIR}/common ${CMAKE_CURRENT_SOURCE_DIR}/../cpp/common)
link_directories(${LIBQI21_INSTALL_DIR}/lib)
set(QI21_LIBS qimessaging qitype qi boost_system boost_thread boost_filesystem boost_chrono boost_program_options boost_regex pthread)

add_executable(qi-dump-values dump_values.cpp)

target_link_libraries(qi-dump-values ${QI21_LIBS})

add_executable(qi-cpp-service service.cpp)
target_link_libraries(qi-cpp-service ${QI21_LIBS})

add_executable(qi-cpp-client client.cpp)
target_link_libraries(qi-cpp-client ${QI21_LIBS})

add_executable(qi-cpp-sd sd.cpp)
target_link_libraries(qi-cpp-sd ${QI21_LIBS})

add_executable(qi-cpp-echo-client echo_client.cpp)
target_link_libraries(qi-cpp-echo-client ${QI21_LIBS})

add_executable(qi-cpp-naoqi-probe naoqi_probe.cpp)
target_link_libraries(qi-cpp-naoqi-probe ${QI21_LIBS})
