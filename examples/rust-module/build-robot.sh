#!/bin/bash
# Builds the example module for robots, in dist/rustexample-robot: the library for 32-bit (i686:
# NAO V4 and V5) and 64-bit (x86_64) systems, linked against glibc 2.13 so that it loads on the
# old C libraries of the robots, and the declaration of the module, with an autoload.ini.
#
# The libraries have no C++ bridge: NAOqi loads them from its autoload.ini (the RustGreeter and
# RustDcm services), and so does qi-launch, but qilaunch needs the libqi entry points (feature
# `cpp`) and the DCM plugin RustEyes the NAOqi bridge (feature `naoqi`): both must be built with
# the cross toolchain of the robot (see docs/modules.md).
#
# Needs cargo-zigbuild (`pip install ziglang && cargo install cargo-zigbuild`) and the Rust
# targets (`rustup target add i686-unknown-linux-gnu x86_64-unknown-linux-gnu`).
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
GLIBC=${GLIBC:-2.13}
DIST="$REPO/dist/rustexample-robot"
rm -rf "$DIST"
for arch in i686 x86_64; do
  target=$arch-unknown-linux-gnu
  (cd "$REPO" && cargo zigbuild --release -p qi-module-example --target "$target.$GLIBC")
  mkdir -p "$DIST/$arch/lib" "$DIST/$arch/share/qi/module"
  cp "$REPO/target/$target/release/librustexample.so" "$DIST/$arch/lib/"
  echo cpp > "$DIST/$arch/share/qi/module/rustexample.mod"
  printf '[user]\n/home/nao/rust/lib/librustexample.so\n' > "$DIST/$arch/autoload.ini"
  echo "$arch: newest glibc symbol version required:" \
    "$(objdump -T "$DIST/$arch/lib/librustexample.so" | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1)"
done
cp "$HERE/robot-README.md" "$DIST/README.md"
echo "built in $DIST"
