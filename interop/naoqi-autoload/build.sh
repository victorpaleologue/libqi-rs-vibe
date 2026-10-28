#!/bin/bash
# Builds the programs loading module libraries the way NAOqi does, for the module tests
# (qi/tests/interop_naoqi.rs), in interop/naoqi-autoload/build:
#
# - fake-naoqi: `_createModule(boost::shared_ptr<AL::ALBroker>)` with a stand-in broker, as
#   NAOqi calls the libraries of its autoload.ini;
# - sdk/: the stub of the NAOqi C++ SDK of ../naoqi-sdk-stub (headers and libnaoqistub.so), to
#   build the DCM bridge of the feature `module-naoqi` against;
# - fake-dcm: `_createModule` with a broker of the stub, then cycles of the loop of the DCM.
#
# Needs a C++ compiler and the Boost headers (signals2). Variable: CXX.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
STUB="$HERE/../naoqi-sdk-stub"
OUT="$HERE/build"
CXX=${CXX:-g++}
mkdir -p "$OUT/sdk/lib" "$OUT/sdk/include"
cp -r "$STUB/include/." "$OUT/sdk/include/"
$CXX -shared -fPIC -O1 -I"$STUB/include" "$STUB/stub.cpp" -o "$OUT/sdk/lib/libnaoqistub.so"
$CXX -O1 "$HERE/fake_naoqi.cpp" -o "$OUT/fake-naoqi" -ldl
$CXX -O1 -I"$STUB/include" "$HERE/fake_dcm.cpp" -o "$OUT/fake-dcm" \
  -L"$OUT/sdk/lib" -lnaoqistub -ldl -Wl,-rpath,"$OUT/sdk/lib"
echo "built in $OUT"
