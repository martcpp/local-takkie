#!/bin/bash

if [[ "$OSTYPE" == "darwin"* ]]; then
    echo "Running on macOS"
    brew install cmake opus pkg-config
    cargo run --bin vl mac03 9002

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
        webkit2gtk-4.1 \
        javascriptcoregtk-4.1\
        libsoup-3.0\

    cargo run --bin vl mac03 9002

elif [[ "$OSTYPE" == "msys" || "$OSTYPE" == "cygwin" || "$OSTYPE" == "win32" ]]; then
    echo "Running on Windows"
    cargo run --bin vl mac03 9002

else
    echo "Unsupported OS: $OSTYPE"
    exit 1
fi
