# Installing this custom version from a Mac

This is PocketCW 0.4.0 with these additions:

- **MANIA** (RECEIVE → MANIA): a listening game in the style of the Morse Mania app. Hear a character and pick it from the choices on F1–F4, or key it. Levels add characters in mirror pairs (A `.-` / N `-.`); 16 of 20 right opens the next level.
- **CHARS** (RECEIVE → CHARS): one character at a time. Key it back, or press **F2 (HINT)** to see it and its dits and dahs. Choose what it sends in SETTINGS → RECEIVE → **CHARS SET**.
- **GAMES** (menu → GAMES): DASH (a side-scroller), LADDER, INVADERS, SIMON, WORDLE and FIST. See the Games page of the manual.
- **Themes**: SETTINGS → THEME. **GRID** + **SYNTHWAVE** for retrowave, **TERM** + **PHOSPHOR** for a green-screen terminal, and the palettes NIGHT, DAYLIGHT, DMG, NIXIE and VAPOR.

## Install (one command, no setup)

GitHub builds the firmware every time a change lands in your fork, so your Mac only downloads it. Open **Terminal** and paste:

```bash
curl -fsSL https://raw.githubusercontent.com/jnunham/k5mme_pocketcw/main/get-mine.sh | sh
```

It asks whether to install the content pack (answer **N** if the device already runs PocketCW 0.4.0), then follow the steps it prints. Each build is on your fork's **Releases** page.

## Or build it on your own Mac

1. Unzip the folder you got from Claude.
2. Open **Terminal** (press ⌘-Space, type `Terminal`, press Enter).
3. Type `sh ` (with a space after it), drag **install-mine.sh** from the folder onto the Terminal window, and press Enter.

The script does everything:

- The first time, it installs Apple's command line tools. If a window pops up asking to install them, click **Install**, wait for it to finish, then run step 3 again.
- The first time, it installs Rust, the language the firmware is written in. That takes a few minutes.
- It builds your firmware.
- It asks whether to install the content pack too. If the device already runs PocketCW 0.4.0, answer **N**. On a device that has never had PocketCW, answer **y**.
- It installs the firmware the same way the official installer does: follow the steps it prints to put the device in update mode, and type your Mac password when it asks.

Your settings, Koch level and the content pack stay on the device.

After a change from Claude, unzip the new folder and run step 3 again; the setup steps are skipped once they're done, so it goes straight to building and installing.

To only build without installing: `sh install-mine.sh build`.

**If anything fails, copy everything Terminal printed and send it to Claude.**

## Going back

The official installer puts the official PocketCW back:

```bash
curl -fsSL https://kvmet.github.io/k5mme_pocketcw/install.sh | sh
```

The bootloader is never touched, so you can always install again.
