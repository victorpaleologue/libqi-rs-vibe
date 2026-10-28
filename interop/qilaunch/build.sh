#!/bin/bash
# Builds the real qilaunch against libqi 4.0.5, for the module tests (qi/tests/interop_qilaunch.rs).
#
# qilaunch lived in libqicore from January 2015 to December 2016, before it moved to the closed
# qi-tools repository: its last version is `d1ba0a4^:bin/qilaunch/qilaunch.cpp` in the history of
# https://github.com/aldebaran/libqicore. It is extracted from there and patched to build without
# Qt, Breakpad and the LogProvider of libqicore (qilaunch-without-qt-breakpad-qicore.patch), which
# only affect -q, crash dumps and remote logging.
#
# Variables: LIBQICORE_SOURCE_DIR (a clone of libqicore, default /home/user/aldebaran/libqicore),
# LIBQI_PREFIX (the libqi 4.0.5 install, default /home/user/aldebaran/libqi/install), CXX.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
LIBQICORE=${LIBQICORE_SOURCE_DIR:-/home/user/aldebaran/libqicore}
LIBQI_PREFIX=${LIBQI_PREFIX:-/home/user/aldebaran/libqi/install}
[ -d "$LIBQICORE/.git" ] || git clone https://github.com/aldebaran/libqicore.git "$LIBQICORE"
mkdir -p "$HERE/build"
git -C "$LIBQICORE" show 'd1ba0a4^:bin/qilaunch/qilaunch.cpp' > "$HERE/build/qilaunch.cpp"
patch --quiet "$HERE/build/qilaunch.cpp" < "$HERE/qilaunch-without-qt-breakpad-qicore.patch"
# GNU mode: the log macros of libqi 4.0.5 need it.
${CXX:-g++} -std=gnu++17 -O1 "$HERE/build/qilaunch.cpp" -o "$HERE/build/qilaunch" \
  -I"$LIBQI_PREFIX/include" -L"$LIBQI_PREFIX/lib" -Wl,-rpath,"$LIBQI_PREFIX/lib" \
  -lqi -lboost_program_options -lboost_thread -lboost_system -lssl -lcrypto -lpthread
echo "qilaunch built in $HERE/build/qilaunch"
