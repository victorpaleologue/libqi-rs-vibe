#!/bin/bash
# Builds the libqi stack of NAOqi 2.1 inside the Ubuntu 14.04 container (see build-libqi-2.1.sh):
# the libqi core at tag v2.1.3, and libqitype and libqimessaging at the commits that fed the
# 2.1 release branch (June 2014, right before the three repositories were merged into libqi).
# Everything is installed in $ROOT/install21, with the Boost and ICU libraries of the container
# copied to $ROOT/install21/lib-deps so that the binaries run on the host.
set -ex
ROOT=${LIBQI21_ROOT:?}
STUBS=$(cd "$(dirname "$0")" && pwd)/stubs
P=$ROOT/install21
Q=$ROOT/qibuild/cmake/qibuild

clone() {
  name=$1; ref=$2
  if [ ! -d "$ROOT/$name" ]; then
    git -C "$ROOT/libqi-src" worktree add -f "$ROOT/$name" "$ref"
  fi
}
[ -d "$ROOT/libqi-src" ] || git clone https://github.com/aldebaran/libqi.git "$ROOT/libqi-src"
[ -d "$ROOT/qibuild" ] || git clone --branch v3.5.3 --depth 1 https://github.com/aldebaran/qibuild.git "$ROOT/qibuild"
clone libqi-2.1.3 v2.1.3        # the core: 2014-11-13, hotfix/release-2.1-348
clone qitype-2.1 f0b299a1       # libqitype: 2014-06-12, team/platform/dev-50
clone qimessaging-2.1 91217eb4  # libqimessaging: 2014-06-12, last commit before the merge

build() {
  name=$1; src=$2; shift 2
  mkdir -p "$ROOT/build21/$name" && cd "$ROOT/build21/$name"
  cmake "$src" -DCMAKE_BUILD_TYPE=RelWithDebInfo -Dqibuild_DIR="$Q" -DCMAKE_INSTALL_PREFIX="$P" \
    -DCMAKE_PREFIX_PATH="$P" -Dqiprobes_DIR="$STUBS/qiprobes" -DQI_WITH_TESTS=OFF -DBUILD_TESTING=OFF "$@"
  make -j"$(nproc)" install
}
build libqi "$ROOT/libqi-2.1.3"
build libqitype "$ROOT/qitype-2.1"
build libqimessaging "$ROOT/qimessaging-2.1" -DWITH_PYTHON=OFF -DWITH_QIPERF=OFF

# The shared libraries of the container the stack links to, for running on the host.
mkdir -p "$P/lib-deps"
for lib in $(ldd "$P"/lib/libqi.so "$P"/lib/libqitype.so "$P"/lib/libqimessaging.so \
             | grep -o '/usr/lib[^ ]*' | sort -u); do
  cp -L "$lib" "$P/lib-deps/"
done
echo "libqi 2.1 installed in $P"
