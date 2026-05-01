# clipt9n manual testing

## Release target

M8 release readiness requires:
- macOS Day-1 pass complete.
- VoiceOver pass complete for prompt, history, setup wizard, and tray-confirm modal.
- Latency benchmark recorded in `docs/benchmarks/<date>.md`.
- Linux and Windows smoke rows attempted or explicitly marked blocked with environment details.

## Translation matrix

| Source | Action | Target/instruction | Expected | Result |
|---|---|---|---|---|
| EN | Translate | EN | unchanged | [ ] |
| EN | Translate | DE | German output | [ ] |
| EN | Translate | TR | Turkish output | [ ] |
| DE | Translate | EN | English output | [ ] |
| DE | Translate | DE | unchanged | [ ] |
| DE | Translate | TR | Turkish output | [ ] |
| TR | Translate | EN | English output | [ ] |
| TR | Translate | DE | German output | [ ] |
| TR | Translate | TR | unchanged | [ ] |
| EN | Fix grammar | - | minimal edits, stays English | [ ] |
| DE | Fix grammar | - | minimal edits, stays German | [ ] |
| TR | Fix grammar | - | minimal edits, stays Turkish | [ ] |
| EN | Rewrite | - | clearer, stays English | [ ] |
| DE | Rewrite | - | clearer, stays German | [ ] |
| TR | Rewrite | - | clearer, stays Turkish | [ ] |
| EN | Custom | make formal | follows instruction | [ ] |
| DE | Custom | summarize | follows instruction | [ ] |
| TR | Custom | bullet list | follows instruction | [ ] |

## Setup wizard matrix

| Scenario | Expected | Result |
|---|---|---|
| No key, keychain available | wizard opens | [ ] |
| Invalid key | 401 shown, key retained | [ ] |
| Network down | network error shown, key retained | [ ] |
| Sample translation unchecked | save allowed after connectivity warning | [ ] |
| Provider switch mid-wizard | rows reset, key retained | [ ] |
| Keychain unavailable | env-only mode | [ ] |
| Stale key during translation | wizard auto-opens | [ ] |

## Tray and history matrix

Mark each row with `OS / date / result`. Rows mirror the M7 README smoke matrix.

| OS | Surface | Expected | Result |
|----|---------|----------|--------|
| macOS | Tray icon appears in menu bar at startup | Visible | [ ] |
| macOS | Click tray → menu drops down | Menu visible | [ ] |
| macOS | Translate clipboard menu item | Prompt window appears | [ ] |
| macOS | Open history menu item | History window appears | [ ] |
| macOS | Open glossary menu item | Default editor opens glossary.toml | [ ] |
| macOS | Reload glossary menu item | Glossary re-reads without restart | [ ] |
| macOS | Re-run setup wizard menu item | Wizard window appears | [ ] |
| macOS | Hide icon → confirm | Tray disappears; relaunch w/o flag still hidden | [ ] |
| macOS | Hide icon → cancel | Tray remains; no state.toml change | [ ] |
| macOS | Relaunch with --show-tray | Tray reappears; subsequent launches show it | [ ] |
| macOS | Stale API key 401 | Wizard auto-opens; tray pill flips amber → red | [ ] |
| macOS | Accessibility permission revoked | Tray pill amber; tooltip says "permission needed" | [ ] |
| macOS | Glossary malformed | Tray pill amber; app still translates | [ ] |
| macOS | Hotkey already in use | Tray pill amber; tray menu remains the entry point | [ ] |
| macOS | Wizard Save-and-start | Next translation uses new key with no restart | [ ] |
| macOS | History encryption round-trip | Insert → Cmd+Option+H → row decrypts | [ ] |
| macOS | History viewer search | Search field filters rows after decrypt | [ ] |
| macOS | History corruption recovery | Random bytes in history.db → app starts with history disabled | [ ] |
| Linux | Tray icon in supported DE (GNOME/KDE) | Visible via StatusNotifierItem | [ ] |
| Linux | Tray icon in headless DE (no SNI) | Logs warn; hotkey still works | [ ] |
| Linux | Open glossary launches via xdg-open | Default editor opens | [ ] |
| Windows | Tray icon in shell tray | Visible | [ ] |
| Windows | Right-click tray → menu | All 7 items present | [ ] |
| Windows | Open glossary via cmd /C start | Default editor opens | [ ] |

## Accessibility matrix

| Surface | Check | Result |
|---|---|---|
| Prompt | VoiceOver announces slots and focus order | [ ] |
| History | VoiceOver announces search/list/detail/buttons | [ ] |
| Setup wizard | VoiceOver announces provider cards as buttons | [ ] |
| Tray hide confirm | VoiceOver announces confirm/cancel and hotkey | [ ] |
| macOS Display Accommodations | contrast remains readable | [ ] |
