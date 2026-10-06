#!/bin/bash
# Builds the libqi stack of NAOqi 2.1 (2014) in an Ubuntu 14.04 container, whose GCC 4.8 and
# Boost 1.54 are what that code was written for. Needs Docker. The sources are cloned and built
# under $LIBQI21_ROOT (default /home/user/aldebaran) and installed in $LIBQI21_ROOT/install21;
# the harness is then built with ../build.sh.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
LIBQI21_ROOT=${LIBQI21_ROOT:-/home/user/aldebaran}
mkdir -p "$LIBQI21_ROOT"
docker build --network host --build-arg http_proxy="$HTTP_PROXY" --build-arg https_proxy="$HTTPS_PROXY" \
  -t libqi21-build "$HERE"
exec docker run --rm --network host -e LIBQI21_ROOT="$LIBQI21_ROOT" \
  -e http_proxy="$HTTP_PROXY" -e https_proxy="$HTTPS_PROXY" \
  -v "$HERE/../../..:$HERE/../../.." -v "$LIBQI21_ROOT:$LIBQI21_ROOT" libqi21-build \
  "$HERE/build-in-container.sh"
