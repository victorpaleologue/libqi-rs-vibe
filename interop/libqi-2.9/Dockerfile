FROM ubuntu:18.04
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    clang build-essential cmake libboost-all-dev libssl-dev ca-certificates && rm -rf /var/lib/apt/lists/*
