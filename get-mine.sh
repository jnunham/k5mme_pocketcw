#!/bin/sh
# Installs this custom PocketCW on a HamCube from macOS, from the newest build GitHub made.
# Needs no Rust: the firmware and the install tool come prebuilt.
# usage: curl -fsSL https://raw.githubusercontent.com/jnunham/k5mme_pocketcw/main/get-mine.sh | sh
# The shell reads this script from its stdin, so prompts and pocketcw-flash read the terminal.
set -eu

release=https://github.com/jnunham/k5mme_pocketcw/releases/latest/download
# The content pack this firmware goes with: the upstream release it is built on.
pack=https://github.com/kvmet/k5mme_pocketcw/releases/download/v0.4.0/F000400.bin

fail() {
    echo "error: $*" >&2
    exit 1
}

# get URL FILE
get() {
    curl -fsSL -o "$2" "$1" || fail "could not download $1"
}

main() {
    [ "$(uname -s)" = Darwin ] || fail "this script runs only on macOS"
    work=$(mktemp -d -t pocketcw)
    trap 'rm -rf "$work"' EXIT
    get "$release/pocketcw-flash" "$work/pocketcw-flash"
    chmod +x "$work/pocketcw-flash"
    get "$release/A000014.bin" "$work/A000014.bin"
    echo
    echo "The content pack holds the voice and the words for the NINJA and HEAD drills."
    echo "You need it on the first PocketCW install on this device, or when NINJA or HEAD"
    echo "shows CONTENT PACK MISSING. If the device already runs PocketCW 0.4.0, skip it."
    echo "It is about 8 MB and takes several minutes to install."
    echo
    printf "Install the content pack too? [y/N] "
    read -r answer </dev/tty
    case $answer in
    [yY]*)
        get "$pack" "$work/F000400.bin"
        "$work/pocketcw-flash" "$work/F000400.bin" "$work/A000014.bin" </dev/tty
        ;;
    *)
        "$work/pocketcw-flash" "$work/A000014.bin" </dev/tty
        ;;
    esac
    echo "Then click the dial to turn it on."
}

# Called only once the whole script has arrived, so a cut-off download runs nothing.
main "$@"
