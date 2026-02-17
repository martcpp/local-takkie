#!/bin/bash
if [[ "$OSTYPE" == "darwin"* ]]; then
    echo "Running on macOS"
    brew install cmake opus
    cargo run --bin vl mac03 9002

elif [[ "$OSTYPE" == "linux-gnu"* ]]; then
    echo "Running on Linux"
    cargo run --bin vl linux03 9002

elif [[ "$OSTYPE" == "msys" || "$OSTYPE" == "cygwin" || "$OSTYPE" == "win32" ]]; then
    echo "Running on Windows"
    cargo run --bin vl win03 9002

else
    echo "Unsupported OS: $OSTYPE"
    exit 1
fi