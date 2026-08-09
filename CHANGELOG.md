# Changelog

## Unreleased

- Prompt-template editor: **Edit prompt templates…** in the menu bar
  opens the four action templates for editing, with a per-action
  variable list and a preview rendered from sample values. Save
  validates every template through the same loader startup uses, writes
  atomically, and swaps the live templates in place — no restart.
  **Reset to default** restores the built-in text, and saving then
  removes the override file rather than writing a copy of it. Templates
  are no longer fixed for the lifetime of the process; **Reload prompt
  templates** picks up edits made outside the app, keeping the previous
  templates if the new ones fail to parse. Also reachable with
  `--templates`.
- Glossary editor: **Edit glossary…** in the menu bar opens a table of
  glossary entries with add, edit, delete, and search. Save validates
  before writing, writes atomically, and reloads the live glossary. A
  malformed `glossary.toml` still opens so it can be repaired in-app.
  The editor rewrites the file from its entries, so it warns up front
  when the existing file has comments it cannot preserve; the previous
  menu item is now **Open glossary file** for editing the TOML directly.
  Also reachable with `--glossary`, for when the menu bar icon is hidden.
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

## 0.0.1 - 2026-04-29

- M1: CLI walking skeleton and provider abstraction.
- M2: prompt window, global hotkey, state file.
- M3: all actions, custom prompt, translating overlay, size confirm.
- M4: glossary, template overrides, SIGHUP reload.
- M5: encrypted history and viewer.
- M6: setup wizard, keychain, kittest infrastructure.
- M7: tray icon, glossary launch, live provider rebuild, accessibility polish.
- M8: comprehensive tests, fuzz/bench harnesses, packaging, CI release workflow, manual QA docs.
