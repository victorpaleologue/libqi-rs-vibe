FROM ubuntu:14.04
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    build-essential cmake libboost-all-dev libssl-dev python-dev libgtest-dev && rm -rf /var/lib/apt/lists/*
