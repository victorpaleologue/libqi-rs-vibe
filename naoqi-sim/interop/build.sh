#!/bin/sh
# Builds driver_smoke against a libqi/libqicore installation (the ros-naoqi forks by default).
# Usage: QI_PREFIX=/path/to/install BOOST_PREFIX=/path/to/boost OUT=./driver_smoke ./build.sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
QI_PREFIX=${QI_PREFIX:-/home/user/ros_ws/install}
BOOST_PREFIX=${BOOST_PREFIX:-/home/user/ros_env}
OUT=${OUT:-$here/driver_smoke}
exec g++ -std=gnu++17 -O1 -Wall -Wno-deprecated-declarations \
  -I"$QI_PREFIX/include" -I"$BOOST_PREFIX/include" \
  "$here/driver_smoke.cpp" -o "$OUT" \
  "$QI_PREFIX/lib/libqicore.a" -L"$QI_PREFIX/lib" -lqi \
  -L"$BOOST_PREFIX/lib" -lboost_thread -lboost_chrono -lboost_filesystem -lboost_regex \
  -lboost_program_options -lboost_date_time -lboost_random -lboost_atomic -lpthread \
  -Wl,-rpath,"$QI_PREFIX/lib" -Wl,-rpath,"$BOOST_PREFIX/lib"
