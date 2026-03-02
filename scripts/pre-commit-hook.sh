#!/usr/bin/env bash
# Pre-commit hook for VideoLAN Audio Streamer
# Install: cp scripts/pre-commit-hook.sh .git/hooks/pre-commit
# Make executable: chmod +x .git/hooks/pre-commit

echo "🧪 Running pre-commit checks..."

# Run tests
echo "📋 Running tests..."
cargo test --lib --quiet
if [ $? -ne 0 ]; then
    echo "❌ Tests failed!"
    exit 1
fi

# Check formatting
echo "📐 Checking rust format..."
cargo fmt -- --check
if [ $? -ne 0 ]; then
    echo "❌ Formatting issues found. Run 'cargo fmt' to fix."
    exit 1
fi

# Run clippy
echo "🔍 Running clippy..."
cargo clippy --all --all-targets -- -D warnings > /dev/null 2>&1
if [ $? -ne 0 ]; then
    echo "❌ Clippy issues found. Run 'cargo clippy --all' for details."
    exit 1
fi

echo "✅ All pre-commit checks passed!"
exit 0
