#!/bin/bash
# Runs the whole libqi 2.1 C++-vs-C++ chain: service directory, service, echo
# client and scenario client. Exits 0 only if everything passes.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
B=$HERE/build
$B/qi-cpp-sd --qi-listen-url tcp://127.0.0.1:0 > /tmp/qi21-sd.out 2>/tmp/qi21-sd.err &
SD=$!
trap 'kill $SD $SVC 2>/dev/null; wait 2>/dev/null' EXIT
for i in $(seq 50); do grep -q LISTENING /tmp/qi21-sd.out 2>/dev/null && break; sleep 0.1; done
URL=$(sed -n 's/^LISTENING //p' /tmp/qi21-sd.out)
echo "sd: $URL"
$B/qi-cpp-service --qi-url "$URL" --qi-listen-url tcp://127.0.0.1:0 > /tmp/qi21-svc.out 2>/tmp/qi21-svc.err &
SVC=$!
for i in $(seq 50); do grep -q READY /tmp/qi21-svc.out 2>/dev/null && break; sleep 0.1; done
$B/qi-cpp-echo-client --qi-url "$URL"
$B/qi-cpp-client --qi-url "$URL" --scenarios all
