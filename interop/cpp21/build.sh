#!/bin/bash
# Builds the libqi 2.1 interoperability harness in the Ubuntu 14.04 container
# produced by build-libqi-2.1.sh (the 2.1 stack must already be installed in
# $LIBQI21_ROOT/install21). The binaries end up in interop/cpp21/build and run
# on the host through the RPATH embedded at configure time.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
LIBQI21_ROOT=${LIBQI21_ROOT:-/home/user/aldebaran}
exec docker run --rm -v "$HERE/../..:$HERE/../.." -v "$LIBQI21_ROOT:$LIBQI21_ROOT" libqi21-build bash -c "
  set -ex
  mkdir -p $HERE/build && cd $HERE/build
  cmake $HERE -DLIBQI21_INSTALL_DIR=$LIBQI21_ROOT/install21 \
        -DLIBQI21_DEPS_DIR=$LIBQI21_ROOT/install21/lib-deps
  make -j4
"
