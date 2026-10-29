#!/bin/bash
# Runs the qilaunch of libqi 2.9 built by build.sh, with the example module staged in an SDK prefix,
# in the container: `run.sh --qi-url tcp://127.0.0.1:9559 -n demo -o rustexample.RustGreeter`.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
PREFIX="$HERE/build/prefix"
mkdir -p "$PREFIX/lib" "$PREFIX/share/qi/module"
cp "$HERE/build/librustexample.so" "$PREFIX/lib/"
echo cpp > "$PREFIX/share/qi/module/rustexample.mod"
mkdir -p "$HOME/.config/qimessaging"
ROOT=${LIBQI29_ROOT:-/home/user/aldebaran}
exec docker run --rm --network host -v "$HOME/.config/qimessaging:/root/.config/qimessaging" \
  -v "$HERE:$HERE" -v "$ROOT:$ROOT" libqi29-build "$HERE/build/qilaunch" --qi-sdk-prefix "$PREFIX" "$@"
