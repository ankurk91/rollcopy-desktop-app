#!/usr/bin/env bash
# System packages needed to build Unspool on Ubuntu 24.04 (CI and fresh machines).
set -euo pipefail
sudo apt-get update
sudo apt-get install -y --no-install-recommends \
    build-essential pkg-config cmake clang \
    libxkbcommon-dev libxkbcommon-x11-dev libx11-xcb-dev libxcb1-dev libwayland-dev \
    libfontconfig-dev libfreetype-dev libdbus-1-dev libzstd-dev \
    libde265-dev
