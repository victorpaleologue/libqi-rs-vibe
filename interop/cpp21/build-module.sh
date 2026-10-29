#!/bin/bash
# Builds the example module (examples/rust-module) with its libqi entry points against the libqi of
# NAOqi 2.1, in the Ubuntu 14.04 container of libqi-2.1/build-libqi-2.1.sh, for the module tests
# of qi/tests/interop_cpp21_module.rs: interop/cpp21/build/modules/librustexample.so.
#
# The Rust toolchain of the host (~/.cargo, ~/.rustup) is mounted in the container, whose GCC 4.8
# and Boost 1.54 build the C++ shim with the ABI of that libqi.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
LIBQI21_ROOT=${LIBQI21_ROOT:-/home/user/aldebaran}
mkdir -p "$HERE/build/modules"
docker run --rm --network host -e http_proxy="$HTTP_PROXY" -e https_proxy="$HTTPS_PROXY" \
  -e HTTPS_PROXY -e SSL_CERT_FILE -e CARGO_HTTP_CAINFO \
  -v "$HOME/.cargo:/root/.cargo" -v "$HOME/.rustup:/root/.rustup" \
  -v "$REPO:$REPO" -v "$LIBQI21_ROOT:$LIBQI21_ROOT" ${SSL_CERT_FILE:+-v "$(dirname "$SSL_CERT_FILE"):$(dirname "$SSL_CERT_FILE")"} \
  -w "$REPO" libqi21-build bash -c "
    set -e
    export PATH=/root/.cargo/bin:\$PATH
    QI_MODULE_CPP_SDK=$LIBQI21_ROOT/install21 CARGO_TARGET_DIR=target/cpp21 \
      cargo build --offline -p qi-module-example --features cpp
  "
cp "$REPO/target/cpp21/debug/librustexample.so" "$HERE/build/modules/"
echo "built $HERE/build/modules/librustexample.so"
