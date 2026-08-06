# Changelog

## Unreleased

- Settings window: edit provider, API key, languages, hotkeys, and behavior
  from the GUI. Reachable from the menu bar or with `--settings`. Saving
  validates, persists, and rebuilds the provider in place.
- Fix: `--config <path>` with a filename other than `config.toml` is now
  honored on write. The setup wizard and "Open config" previously derived
  `config.toml` from the state-file directory, so saves went to a file the
  app never reads back.
- Fix: an unregisterable prompt-hotkey key (anything but `A`–`Z`) now warns
  and disables the hotkey instead of aborting startup, which left no window
  and no tray icon to correct it from.

## 0.1.0 - 2026-04-29

- M1: CLI walking skeleton and provider abstraction.
- M2: prompt window, global hotkey, state file.
- M3: all actions, custom prompt, translating overlay, size confirm.
- M4: glossary, template overrides, SIGHUP reload.
- M5: encrypted history and viewer.
- M6: setup wizard, keychain, kittest infrastructure.
- M7: tray icon, glossary launch, live provider rebuild, accessibility polish.
- M8: comprehensive tests, fuzz/bench harnesses, packaging, CI release workflow, manual QA docs.
