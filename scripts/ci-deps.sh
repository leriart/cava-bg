#!/usr/bin/env bash
# Single source of truth for the system packages CI needs to build cava-bg.
#
# This exists because the list was copy-pasted into four jobs and one of them
# silently lost the FFmpeg headers, which cost three CI round-trips to find: the
# build dies in ffmpeg-sys-next's build script with
#
#     error: FFmpeg library libavcodec not found
#
# and the only visible symptom is a job that fails at `cargo build`.
#
# `ffmpeg-sys-next` resolves libavutil, libavformat, libavfilter, libavdevice,
# libswscale, libswresample and libavcodec through pkg-config, so the `-dev`
# packages are what matter, not the runtime libraries. `libclang` is for the
# bindgen step.
#
# Extra packages can be appended per job:
#     bash scripts/ci-deps.sh rpm fakeroot
set -euo pipefail

sudo apt-get update
sudo apt-get install -y --no-install-recommends \
    pkg-config \
    clang \
    libclang-dev \
    libavcodec-dev \
    libavformat-dev \
    libavutil-dev \
    libavfilter-dev \
    libavdevice-dev \
    libswscale-dev \
    libswresample-dev \
    libpostproc-dev \
    libwayland-dev \
    libxkbcommon-dev \
    libegl1-mesa-dev \
    libgl1-mesa-dev \
    libdbus-1-dev \
    "$@"
