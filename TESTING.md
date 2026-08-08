# clipt9n manual testing

## Release target

M8 release readiness requires:
- macOS Day-1 pass complete.
- VoiceOver pass complete for prompt, history, setup wizard, and tray-confirm modal.
- Latency benchmark recorded in `docs/benchmarks/<date>.md`.
- Linux and Windows smoke rows attempted or explicitly marked blocked with environment details.

## Automated evidence (2026-08-08)

The commands below were run in the `fix/release-ci` worktree on Apple Silicon macOS. Results are updated only from command output; environment-bound checks remain blocked below.

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | **PASSED** |
| `cargo clippy --all-targets --all-features -- -D warnings` | **PASSED**; upstream `block 0.1.6` future-incompatibility warning remains visible |
| `cargo test --all-features` | **PASSED**; 363 passed, 1 intentionally ignored keychain integration test |
| `scripts/lint-platform-discipline.sh` | **PASSED** |
| `cargo package --allow-dirty --list` | **PASSED**; 103 entries, no prohibited secret/local paths |
| `cargo build --release` | **PASSED** |
| `cargo +1.88.0 check --locked` | **PASSED** on rustc 1.88.0 |
| `cargo +nightly fuzz run history_decrypt_fuzz -- -runs=100` | **PASSED**; 100 runs completed without a failing input |
| `scripts/package-macos.sh` plus `lipo -info` | **PASSED**; bundle version 0.0.1, x86_64 + arm64, ad-hoc signature only |

The locked dependency graph requires Rust 1.88 (`image 0.25.10`); CI installs exactly Rust 1.88.0 and runs `cargo check --locked` as the MSRV gate.

`block 0.1.6` remains in the macOS Metal backend through `eframe 0.31.1` → `wgpu 24` → `metal 0.31`. The nearest egui updates retain `block`; removing it requires the breaking eframe/wgpu generation that uses wgpu-hal 29 or newer. That upgrade is outside this release-fix scope because the public egui interfaces are not compatible. CI therefore runs `cargo check --locked --future-incompat-report` followed by `cargo report future-incompatibilities` so the upstream warning remains visible rather than suppressed.

## Environment-bound release evidence

| Requirement | Result | Exact environment needed to unblock |
|---|---|---|
| VoiceOver prompt/history/setup/tray checks | **BLOCKED** | A macOS 13+ interactive desktop with VoiceOver enabled and a human operator able to verify announcements and focus order. |
| Intel Mac runtime of universal app | **BLOCKED** | Physical x86_64 Intel Mac running macOS 13+; build, launch, and exercise the packaged `.app`. |
| Linux runtime smoke | **BLOCKED** | Interactive x86_64 Linux desktop with a StatusNotifierItem-capable GNOME/KDE session, system keyring, clipboard, and global-hotkey access. |
| Windows runtime smoke | **BLOCKED** | Interactive x86_64 Windows desktop with Explorer shell tray, clipboard, notification, and global-hotkey access. |
| Real-provider latency report | **BLOCKED** | Approved provider credentials, permission to incur API charges, network access, and a representative machine/network with conditions recorded. |
| Developer ID signing | **BLOCKED** | Apple Developer ID Application certificate and private key installed in the build keychain, with CI secret access. Current packaging is ad-hoc signed only. |
| Apple notarization and stapling | **BLOCKED** | Apple Developer account/team, notarytool credentials, signed bundle, network access to Apple's notary service, and successful notarization/stapling verification. |

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
| Linux | Tray icon in supported DE (GNOME/KDE) | Visible via StatusNotifierItem | **BLOCKED** — requires interactive x86_64 Linux GNOME/KDE with StatusNotifierItem, clipboard, and hotkey access |
| Linux | Tray icon in headless DE (no SNI) | Logs warn; hotkey still works | **BLOCKED** — requires interactive x86_64 Linux session without an SNI host but with clipboard and hotkey access |
| Linux | Open glossary launches via xdg-open | Default editor opens | **BLOCKED** — requires interactive x86_64 Linux desktop with xdg-open and a configured editor |
| Windows | Tray icon in shell tray | Visible | **BLOCKED** — requires interactive x86_64 Windows desktop with Explorer shell |
| Windows | Right-click tray → menu | All 7 items present | **BLOCKED** — requires interactive x86_64 Windows desktop with Explorer shell |
| Windows | Open glossary via cmd /C start | Default editor opens | **BLOCKED** — requires interactive x86_64 Windows desktop with a configured editor |

## Accessibility matrix

| Surface | Check | Result |
|---|---|---|
| Prompt | VoiceOver announces slots and focus order | **BLOCKED** — requires macOS 13+ interactive desktop, VoiceOver, and human verification |
| History | VoiceOver announces search/list/detail/buttons | **BLOCKED** — requires macOS 13+ interactive desktop, VoiceOver, and human verification |
| Setup wizard | VoiceOver announces provider cards as buttons | **BLOCKED** — requires macOS 13+ interactive desktop, VoiceOver, and human verification |
| Tray hide confirm | VoiceOver announces confirm/cancel and hotkey | **BLOCKED** — requires macOS 13+ interactive desktop, VoiceOver, and human verification |
| macOS Display Accommodations | contrast remains readable | **BLOCKED** — requires macOS 13+ interactive desktop, Display Accommodations, and human visual verification |
