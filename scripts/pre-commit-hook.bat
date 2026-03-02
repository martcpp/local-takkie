@echo off
REM Pre-commit hook for VideoLAN Audio Streamer (Windows)
REM Install: copy scripts\pre-commit-hook.bat .git\hooks\pre-commit.bat

echo 🧪 Running pre-commit checks...

REM Run tests
echo 📋 Running tests...
cargo test --lib --quiet
if errorlevel 1 (
    echo Tests failed!
    exit /b 1
)

REM Check formatting
echo 📐 Checking rust format...
cargo fmt -- --check
if errorlevel 1 (
    echo  Formatting issues found. Run 'cargo fmt' to fix.
    exit /b 1
)

REM Run clippy
echo 🔍 Running clippy...
cargo clippy --all --all-targets -- -D warnings >nul 2>&1
if errorlevel 1 (
    echo Clippy issues found. Run 'cargo clippy --all' for details.
    exit /b 1
)

echo ✅ All pre-commit checks passed!
exit /b 0
