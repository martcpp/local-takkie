#!/bin/bash
set -e  # Exit immediately on any error

if [[ "$OSTYPE" == "darwin"* ]]; then
    echo "Running on macOS"
    brew install cmake opus pkg-config
    cargo run --bin vl -- mac03 9002

elif [[ "$OSTYPE" == "linux-gnu"* ]]; then
    echo "Running on Linux"
    sudo apt-get update
    sudo apt-get install -y \
        pkg-config \
        cmake \
        libopus-dev \
        libglib2.0-dev \
        libatk1.0-dev \
        libgtk-3-dev \
        libasound2-dev \
        pulseaudio \
        alsa-utils \
        libwebkit2gtk-4.1-dev \
        libjavascriptcoregtk-4.1-dev \
        libsoup-3.0-dev

    # Start PulseAudio if not already running (fixes "DeviceNotAvailable" on headless)
    if ! pulseaudio --check 2>/dev/null; then
        echo "Starting PulseAudio..."
        pulseaudio --start --exit-idle-time=-1
    fi

    cargo run --bin vl -- mac03 9002

elif [[ "$OSTYPE" == "msys"* || "$OSTYPE" == "cygwin"* || "$OSTYPE" == "win32"* ]]; then
    echo "Running on Windows"
    # Opus is bundled on Windows — no extra install needed
    cargo run --bin vl -- mac53 9001

else
    echo "Unsupported OS: $OSTYPE"
    exit 1
fi