#!/usr/bin/env sh
set -eu

# scripts/cross_build.sh: build lazydns for a specific PLATFORM using cross/cargo
#
# Purpose:
#   - Map buildx TARGETPLATFORM (e.g. linux/arm64) to Rust target triples used in CI
#   - Ensure the Rust target is installed (using `cross rustup target add` or `rustup target add`)
#   - Build with `cross` if available (recommended), otherwise fall back to host `cargo`
#   - Copy the built binary into `target/bin/<PLATFORM>/lazydns` (adds `.exe` suffix for Windows)
#
# Usage:
#   scripts/cross_build.sh <PLATFORM>
#   scripts/cross_build.sh         # lists supported platforms
#
# Notes:
#   - Matches targets listed in `.github/workflows/release.yml`
#   - Uses `--profile minimal` to mirror CI builds
#
# Behavior:
#   - If run without arguments prints supported PLATFORM -> TRIPLE mappings and exits
#   - If `cross` is available it will attempt to run `cross rustup target add <TRIPLE>` (best-effort)
#   - If `cross` falls back to host `cargo`, the script will try to auto-install the host target via `rustup`

# If no args, list supported platforms and their Rust triples
if [ "$#" -eq 0 ]; then
  cat <<'EOF'
Supported PLATFORMS -> Rust triples:
  linux/amd64          -> x86_64-unknown-linux-musl
  linux/amd64-gnu      -> x86_64-unknown-linux-gnu
  linux/arm/v7         -> armv7-unknown-linux-musleabihf
  linux/arm/v6         -> arm-unknown-linux-gnueabi
  linux/arm64          -> aarch64-unknown-linux-musl
  linux/i686           -> i686-unknown-linux-musl
  freebsd/amd64        -> x86_64-unknown-freebsd
  darwin/amd64         -> x86_64-apple-darwin
  darwin/arm64         -> aarch64-apple-darwin
  windows/amd64        -> x86_64-pc-windows-msvc
  windows/i686         -> i686-pc-windows-msvc
  windows/arm64        -> aarch64-pc-windows-msvc

Usage: $0 <PLATFORM>
Example: $0 linux/arm64
EOF
  exit 0
fi

if [ "$#" -lt 1 ]; then
  echo "Usage: $0 <PLATFORM> [EXTRA_BUILD_ARGS]  (e.g. linux/arm/v7 \"--no-default-features --features log,cron\")" >&2
  exit 1
fi

ORIGINAL_PLATFORM="$1"
PLATFORM="$ORIGINAL_PLATFORM"
# Extra args to append to the underlying build command (can be used to pass flags like
# --no-default-features --features "log,cron" or --release etc.)
EXTRA_ARGS=""

# Special handling for ARM variants: preserve v6/v7 suffix while removing vendor/variant
# e.g. linux/arm/v7/something -> linux/arm/v7, but linux/arm -> stays as is
case "$PLATFORM" in
  linux/arm/v6/*)
    PLATFORM="linux/arm/v6"
    ;;
  linux/arm/v6)
    # already normalized
    ;;
  linux/arm/v7/*)
    PLATFORM="linux/arm/v7"
    ;;
  linux/arm/v7)
    # already normalized
    ;;
  */*/*)
    # For other three-segment platforms, trim to first two segments
    PLATFORM="$(printf '%s' "$PLATFORM" | cut -d/ -f1-2)"
    ;;
esac

# Normalize a few other common variants to canonical two-segment forms.
case "$PLATFORM" in
  linux/amd64|linux/x86_64|linux/amd64-gnu|linux/amd64_gnu)
    # keep as-is or map later in the triple mapping
    ;;
  linux/arm/v6|linux/arm/v7)
    # keep ARM variants with version info as-is
    ;;
  linux/arm64|linux/aarch64)
    ;;
  darwin/amd64|darwin/x86_64|macos/amd64)
    PLATFORM="darwin/amd64"
    ;;
  darwin/arm64|darwin/aarch64|macos/arm64)
    PLATFORM="darwin/arm64"
    ;;
  # leave other PLATFORM values untouched
esac

if [ "$PLATFORM" != "$ORIGINAL_PLATFORM" ]; then
  echo "Normalized PLATFORM: '$ORIGINAL_PLATFORM' -> '$PLATFORM'"
fi
if [ "$#" -ge 2 ]; then
  # Join all remaining args as the extra args (POSIX sh compatible)
  shift
  EXTRA_ARGS="$*"
fi
# Map buildx TARGETPLATFORM (e.g. linux/arm/v7) to Rust target triple
case "$PLATFORM" in
  linux/amd64|linux/x86_64|linux/amd64*)
    TRIPLE="x86_64-unknown-linux-musl"
    ;;
  linux/amd64-gnu|linux/amd64_gnu)
    TRIPLE="x86_64-unknown-linux-gnu"
    ;;
  linux/arm/v7|linux/arm/v7*)
    TRIPLE="armv7-unknown-linux-musleabihf"
    ;;
  linux/arm/v6|linux/arm/v6*)
    TRIPLE="arm-unknown-linux-gnueabi"
    ;;
  linux/arm64|linux/arm64|linux/aarch64|linux/aarch64*)
    TRIPLE="aarch64-unknown-linux-musl"
    ;;
  linux/i686|linux/i386|linux/i686*)
    TRIPLE="i686-unknown-linux-musl"
    ;;
  freebsd/amd64|freebsd/x86_64)
    TRIPLE="x86_64-unknown-freebsd"
    ;;
  darwin/amd64|darwin/x86_64|macos/amd64)
    TRIPLE="x86_64-apple-darwin"
    ;;
  darwin/arm64|darwin/aarch64|macos/arm64)
    TRIPLE="aarch64-apple-darwin"
    ;;
  windows/amd64|windows/x86_64|win/amd64)
    TRIPLE="x86_64-pc-windows-msvc"
    ;;
  windows/i686|win/i686)
    TRIPLE="i686-pc-windows-msvc"
    ;;
  windows/arm64|win/arm64)
    TRIPLE="aarch64-pc-windows-msvc"
    ;;
  *)
    echo "Unknown platform: $PLATFORM" >&2
    exit 1
    ;;
esac

echo "Building for $PLATFORM -> $TRIPLE"

# If cross is available use it, and try to ensure the target is added inside the cross container
if command -v cross >/dev/null 2>&1; then
  echo "Found 'cross' on PATH. Ensuring target exists inside cross and building..."
  # Best-effort: ask cross to add the target inside its build environment
  if cross rustup target add "$TRIPLE" >/dev/null 2>&1; then
    echo "(cross) rust target $TRIPLE added"
  else
    echo "(cross) rustup target add returned non-zero or is not supported in this environment - continuing and hoping the image already supports the target"
  fi
  cross build --target "$TRIPLE" --profile minimal ${EXTRA_ARGS}
else
  echo "'cross' not found: falling back to host cargo build"
  # Try to ensure the rust target is installed on the host
  if command -v rustup >/dev/null 2>&1; then
    if rustup target list --installed | grep -q "^$TRIPLE$$"; then
      echo "Host rust target $TRIPLE already installed"
    else
      echo "Installing host rust target $TRIPLE"
      rustup target add "$TRIPLE"
    fi
  else
    echo "rustup not found on host, can't auto-install host target - continuing and may fail"
  fi
  cargo build --target "$TRIPLE" --profile minimal ${EXTRA_ARGS}
fi

# For Windows triples, use .exe suffix
EXE_SUFFIX=""
case "$TRIPLE" in
  *windows-msvc)
    EXE_SUFFIX=".exe"
    ;;
esac

SRC="target/$TRIPLE/minimal/lazydns${EXE_SUFFIX}"
if [ ! -f "$SRC" ]; then
  echo "Error: build succeeded but binary not found at $SRC" >&2
  exit 1
fi

mkdir -p "target/bin/$PLATFORM"
cp "$SRC" "target/bin/$PLATFORM/lazydns${EXE_SUFFIX}"
chmod +x "target/bin/$PLATFORM/lazydns${EXE_SUFFIX}" 2>/dev/null || true
upx -q --best --lzma "target/bin/$PLATFORM/lazydns${EXE_SUFFIX}" || true

echo "Wrote target/bin/$PLATFORM/lazydns${EXE_SUFFIX}"