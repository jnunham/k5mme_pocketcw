# Building this custom version on a Mac

This is PocketCW with three additions:

- **CHARS drill** (RECEIVE → CHARS): one character at a time. Key back what you hear; press **F2 (HINT)** to see the character and its dits and dahs, hear it again, and move on (it counts as a miss, so you'll get it more often). Pick the characters in SETTINGS → RECEIVE → **CHARS SET**: A-Z, A-Z 0-9, ALL, or KOCH LEVEL.
- **GRID style + SYNTHWAVE palette**: retrowave.
- **TERM style + PHOSPHOR palette**: green-screen hacker terminal.

Pick the looks in SETTINGS → THEME → STYLE and PALETTE.

## 1. One-time setup

Open Terminal and run these one at a time:

```bash
xcode-select --install
```

(If it says the tools are already installed, that's fine.)

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Press Enter to accept the defaults, then close and reopen Terminal.

```bash
rustup target add thumbv7em-none-eabihf
rustup component add llvm-tools
```

## 2. Build

Unzip the source, then `cd` into the folder (type `cd `, then drag the folder onto the Terminal window and press Enter).

```bash
(cd logic && cargo test)
sh tools/mkimage.sh
```

The tests should end with `test result: ok`. `mkimage.sh` ends with a line like
`target/A000014.bin: pocketcw-firmware, 1xxxxx bytes`.

**If the build stops with an error, copy the whole error and send it to Claude.**

## 3. Install it on the device

Build the install tool once:

```bash
clang -O2 -arch arm64 -arch x86_64 -mmacosx-version-min=10.15 -o tools/pocketcw-flash tools/pocketcw-flash.c -framework DiskArbitration -framework CoreFoundation
```

Then install:

```bash
tools/pocketcw-flash firmware/target/A000014.bin
```

Follow the steps it prints (it puts the device in update mode and asks for your Mac password). Your settings, Koch level and the content pack are kept.

## Going back

To return to the official PocketCW, install the `A000014.bin` from the newest release the same way, or follow the user manual's install page. The bootloader is never touched, so you can always reinstall.
