#!/bin/sh
# Builds this custom PocketCW from source and installs it on a HamCube, from macOS. Sets up
# everything the build needs the first time: Apple's command line tools and Rust.
# usage: sh install-mine.sh
#        sh install-mine.sh build    builds the firmware only, and installs nothing
set -eu

# The content pack this source goes with: the upstream release it is built on.
upstream=https://github.com/kvmet/k5mme_pocketcw/releases/download/v0.4.0

fail() {
    echo "error: $*" >&2
    exit 1
}

step() {
    printf '\n==> %s\n' "$*"
}

here=$(cd "$(dirname "$0")" && pwd)

check_mac() {
    [ "$(uname -s)" = Darwin ] || fail "this script runs only on macOS"
    [ -f "$here/tools/mkimage.sh" ] || fail "keep this script in the unzipped PocketCW folder"
}

# Apple's command line tools: the C compiler and linker that Rust uses on the Mac.
setup_apple_tools() {
    if xcode-select -p >/dev/null 2>&1; then
        return
    fi
    step "Installing Apple's command line tools"
    xcode-select --install >/dev/null 2>&1 || true
    echo "A window opened to install them. Click Install, wait for it to finish,"
    echo "then run this script again."
    exit 0
}

setup_rust() {
    export PATH="$HOME/.cargo/bin:$PATH"
    if ! command -v rustup >/dev/null 2>&1; then
        step "Installing Rust (a few minutes, once)"
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
    fi
    step "Getting Rust ready for the device"
    rustup update stable --no-self-update
    rustup default stable >/dev/null
    rustup target add thumbv7em-none-eabihf
    rustup component add llvm-tools
}

build() {
    step "Building your firmware (the first build takes a few minutes)"
    (cd "$here" && sh tools/mkimage.sh) || fail "the build failed: copy the messages above and send them to Claude"
    [ -f "$here/firmware/target/A000014.bin" ] || fail "the build made no firmware/target/A000014.bin"
}

install() {
    step "Building the install tool"
    clang -O2 -mmacosx-version-min=10.15 -o "$here/tools/pocketcw-flash" "$here/tools/pocketcw-flash.c" \
        -framework DiskArbitration -framework CoreFoundation
    work=$(mktemp -d -t pocketcw)
    trap 'rm -rf "$work"' EXIT
    echo
    echo "The content pack holds the voice and the words for the NINJA and HEAD drills."
    echo "You need it on the first PocketCW install on this device, or when NINJA or HEAD"
    echo "shows CONTENT PACK MISSING. If you already run PocketCW 0.4.0, skip it."
    echo "It is about 8 MB and takes several minutes to install."
    echo
    printf "Install the content pack too? [y/N] "
    read -r answer </dev/tty
    step "Installing on the device"
    case $answer in
    [yY]*)
        curl -fsSL -o "$work/F000400.bin" "$upstream/F000400.bin" || fail "could not download the content pack"
        "$here/tools/pocketcw-flash" "$work/F000400.bin" "$here/firmware/target/A000014.bin" </dev/tty
        ;;
    *)
        "$here/tools/pocketcw-flash" "$here/firmware/target/A000014.bin" </dev/tty
        ;;
    esac
    echo "Then click the dial to turn it on."
}

main() {
    check_mac
    setup_apple_tools
    setup_rust
    build
    case ${1:-} in
    "") install ;;
    build) echo "Built firmware/target/A000014.bin. Run without 'build' to install it." ;;
    *) fail "unknown option '$1'; use no option to build and install, or 'build' to build only" ;;
    esac
}

main "$@"
