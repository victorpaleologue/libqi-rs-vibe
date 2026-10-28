#!/bin/bash
# Builds the libqi of NAOqi 2.9 (the release-2.9 branch of libqi), the real qilaunch and the example
# module with its libqi entry points against it, in an Ubuntu 18.04 container (Clang: GCC 7
# rejects the ka/uri headers of that branch), for running the module with that qilaunch:
#
#   LIBQI29_ROOT/install29                      libqi 2.9
#   interop/libqi-2.9/build/qilaunch            qilaunch (libqicore d1ba0a4^, patched as in ../qilaunch)
#   interop/libqi-2.9/build/librustexample.so   the example module (feature cpp)
#
# The programs run in the container (`run.sh`), against a space of the host (--network host). The
# container must share the machine identifier of the host (~/.config/qimessaging is mounted):
# processes of different machine identifiers do not use each other's loopback endpoints.
#
# Variables: LIBQI29_ROOT (default /home/user/aldebaran: libqi-src, qibuild-3.15 and libqicore
# are cloned there if missing).
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
ROOT=${LIBQI29_ROOT:-/home/user/aldebaran}
[ -d "$ROOT/libqi-src" ] || git clone https://github.com/aldebaran/libqi.git "$ROOT/libqi-src"
[ -d "$ROOT/qibuild-3.15" ] || git clone --depth 1 --branch v3.15 https://github.com/aldebaran/qibuild.git "$ROOT/qibuild-3.15"
[ -d "$ROOT/libqicore" ] || git clone https://github.com/aldebaran/libqicore.git "$ROOT/libqicore"
if [ ! -d "$ROOT/libqi-2.9" ]; then
  git -C "$ROOT/libqi-src" fetch origin release-2.9
  git -C "$ROOT/libqi-src" worktree add -f "$ROOT/libqi-2.9" FETCH_HEAD
  # Its tests need gmock: not built.
  sed -i 's/^add_subdirectory("tests")/# add_subdirectory("tests")/' "$ROOT/libqi-2.9/CMakeLists.txt"
fi
mkdir -p "$HERE/build"
git -C "$ROOT/libqicore" show 'd1ba0a4^:bin/qilaunch/qilaunch.cpp' > "$HERE/build/qilaunch.cpp"
patch --quiet "$HERE/build/qilaunch.cpp" < "$REPO/interop/qilaunch/qilaunch-without-qt-breakpad-qicore.patch"
docker build -q --network host --build-arg http_proxy="$HTTP_PROXY" --build-arg https_proxy="$HTTPS_PROXY" \
  -t libqi29-build "$HERE" > /dev/null
docker run --rm --network host -v "$HOME/.cargo:/root/.cargo" -v "$HOME/.rustup:/root/.rustup" \
  -v "$REPO:$REPO" -v "$ROOT:$ROOT" libqi29-build bash -c "
    set -ex
    P=$ROOT/install29
    if [ ! -f \$P/lib/libqi.so ]; then
      mkdir -p $ROOT/build29 && cd $ROOT/build29
      CC=clang CXX=clang++ cmake $ROOT/libqi-2.9 -DCMAKE_BUILD_TYPE=RelWithDebInfo \
        -Dqibuild_DIR=$ROOT/qibuild-3.15/cmake/qibuild -DCMAKE_INSTALL_PREFIX=\$P \
        -Dqiprobes_DIR=$REPO/interop/cpp21/libqi-2.1/stubs/qiprobes -DWITH_SYSTEMD=OFF
      make -j\$(nproc) install
    fi
    clang++ -std=gnu++14 -O1 $HERE/build/qilaunch.cpp -o $HERE/build/qilaunch -I\$P/include -L\$P/lib \
      -Wl,-rpath,\$P/lib -lqi -lboost_program_options -lboost_thread -lboost_system \
      -lboost_filesystem -lboost_chrono -lssl -lcrypto -lpthread
    cd $REPO
    export PATH=/root/.cargo/bin:\$PATH
    CXX=clang++ QI_MODULE_CPP_SDK=\$P CARGO_TARGET_DIR=target/cpp29 \
      cargo build --offline -p qi-module-example --features cpp
    cp target/cpp29/debug/librustexample.so $HERE/build/
  "
echo "built in $HERE/build"
