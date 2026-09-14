## Release triplet

- **shell:** 0.1.3
- **backend tested against:** upstream `master` @ `c291e79` (`0.1.5-rc.2`)
- **bundled dsh-browser:** 0.1.0 (`dsh.engines.backend >=0.1.5-rc.1`)

Requires Git, Node.js 22+, and pnpm on PATH (first-run provisioning and
backend updates shell out to them), plus the WebView2 runtime
(preinstalled on Windows 11 and current Windows 10).

## Installer

`dsh-desktop_0.1.3_x64-setup.exe` — per-user, no admin. First run offers
**Install bundled dsh** (recommended — clones upstream, installs, builds)
or **Use my own dsh** (your checkout, your updates, the shell never
touches it).

## Portable

`dsh-desktop_0.1.3_portable_x64.zip` — extract anywhere and run
`dsh-desktop.exe`; nothing is installed and nothing lands in your user
profile. The copy keeps its backend, settings, logs and browser data in a
`data\` folder beside the exe, so moving or deleting that folder moves or
resets the whole app. The exe is the same binary as the installer's; the
`portable.txt` marker in the zip is what selects the portable layout.

Run the portable copy and an installed copy one at a time — both listen on
the same loopback control port.

Unsigned build: Windows SmartScreen shows an "unknown publisher"
click-through on first run. Signing is a later milestone.
