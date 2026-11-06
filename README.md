# key-remap

Swap the roles of `` ` `` and `Alt+`` ` `` on a US keyboard under Windows + Japanese IME.

**Website: https://thanatos333.github.io/key-remap/**

## Download

**[key-remap.exe (latest)](https://github.com/thanatos333/key-remap/releases/latest/download/key-remap.exe)** — single-file binary, no installation needed. All releases: [Releases](https://github.com/thanatos333/key-remap/releases)

- `` ` `` alone → toggle IME on/off (acts as the 半角/全角 key)
- `Alt+`` ` `` → type the `` ` `` character itself
- `Shift+`` ` `` (~), `Ctrl+`` ` ``, `Win+`` ` `` … are left untouched

## Why

On a US keyboard, Windows Japanese IME toggles with `Alt+`` ` `` and the plain
key types a backquote. If you toggle IME far more often than you type a
backquote, swapping the two is much nicer.

## Implementation

A tiny, dependency-free Rust program (raw Win32 FFI):

- `WH_KEYBOARD_LL` low-level keyboard hook watches for `VK_OEM_3`
- handled presses are swallowed; the action is posted to the main thread
  and performed with `SendInput`
  - IME toggle: sends the Hankaku/Zenkaku key (`vkF3` / `sc029`)
  - backquote: sends the Unicode character, briefly masking Alt so apps
    don't treat it as an Alt shortcut
- no polling, no timers, no UI — the process idles in `GetMessage`
  (working set ≈ 1–2 MB)

## Build

```powershell
cargo build --release
# -> target\release\key-remap.exe
```

## Install

Just run `key-remap.exe`. On launch it registers itself in the
HKCU `Run` key, so from the next logon on it starts automatically.
Moving the exe is fine — the registered path is refreshed on every launch.

## Stop / uninstall

Run `key-remap.exe --uninstall` to remove the auto-start registration,
then end the process in Task Manager and delete the file.
Apart from that one Run value, nothing is written to the registry.

## Notes

- Like any user-level remapper, it does not intercept input on windows
  running elevated (as administrator) unless it also runs elevated.
- Auto-repeat is handled: holding `` ` `` toggles IME only once per press,
  while holding `Alt+`` ` `` keeps typing backquotes.
