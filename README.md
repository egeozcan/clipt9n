# clipt9n — clipboard translator

A keyboard-driven clipboard translator. Press a hotkey, pick an action, get the result back on your clipboard.

> **Status: M8 / v0.1.0 — release readiness.**
> CLI mode still works (M1 behavior). GUI mode summons the prompt window on Cmd+Shift+T; all six slots work end-to-end with an in-flight overlay and large-clipboard confirmation. M5 encrypted history; M6 setup wizard + keychain; M7 menu-bar tray, status pill, hide-icon confirm modal, live provider rebuild, accessibility polish. M8 adds comprehensive unit/integration coverage, `cargo-fuzz` targets, a real-provider latency benchmark, packaging scripts (`scripts/package-{macos,linux}.sh`), GitHub Actions release workflow, and the manual QA log in `TESTING.md`. macOS Day-1 + VoiceOver pass tracked in `TESTING.md`. See `docs/superpowers/specs/2026-04-28-clipt9n-implementation-design.md` for the full milestone roadmap.

## Install (M1)

Build from source:

```bash
git clone https://github.com/<you>/clipt9n.git
cd clipt9n
cargo build --release
cp target/release/clipt9n /usr/local/bin/   # or your bin dir of choice
```

## Configure

Set your API key in your shell:

```bash
export ANTHROPIC_API_KEY=sk-ant-…
```

Optional: write a config file at `~/Library/Application Support/clipboard-translator/config.toml` (macOS):

```toml
[provider]
type = "anthropic"
model = "claude-haiku-4-5"
base_url = "https://api.anthropic.com/v1"
timeout_seconds = 30

[provider.api_key]
source = "env"
env_var = "ANTHROPIC_API_KEY"

[languages.slot_1]
label = "English"
code = "en"

[languages.slot_2]
label = "Deutsch"
code = "de"

[languages.slot_3]
label = "Türkçe"
code = "tr"
```

The defaults shown above are applied if no config file exists.

## Use — CLI mode

Copy text to your clipboard, then run one of:

```bash
clipt9n --translate-to=de             # translate clipboard to Deutsch
clipt9n --fix-grammar                 # fix grammar in source language
clipt9n --rewrite                     # rewrite for clarity
clipt9n --custom "make this formal"   # apply an arbitrary instruction
```

The translated/edited text replaces your clipboard contents.

## Use — GUI mode (M2)

When invoked with no action flag, `clipt9n` launches in GUI mode:

```bash
ANTHROPIC_API_KEY=sk-ant-... clipt9n
```

The app stays running in the background. Press **Cmd+Shift+T** (default; configurable in `[hotkey]`) to summon the prompt window. Pick a numbered slot:

- **1 / 2 / 3** — translate to the language in `[languages.slot_N]` (default: English / Deutsch / Türkçe)
- **4** — fix grammar in the source language
- **5** — rewrite for clarity in the source language
- **6** — open the custom prompt window (free-form instruction; presets included)

`Enter` repeats your last action. `Esc` dismisses the window.

### macOS Accessibility permission

Global hotkey registration on macOS requires Accessibility permission. On first launch, clipt9n triggers a one-time grant flow:

1. macOS shows the system permission dialog.
2. System Settings opens to Privacy & Security → Accessibility.
3. Toggle **clipt9n** (or your terminal, if running from terminal in dev) on.
4. Relaunch the binary.

Without this permission, `Cmd+Shift+T` will not be detected and the app exits with `AccessibilityPermissionDenied`.

### Configurable hotkey

Override the default in `config.toml`:

```toml
[hotkey]
modifier = "cmd"   # "cmd" (→ Ctrl on Linux/Windows), "ctrl", "alt", "super"
shift = true
key = "T"          # single uppercase letter A–Z
enabled = true     # set false to disable hotkey entirely
```

### Slot 6 — custom prompt

Pressing **6** opens a small editor where you type a free-form instruction. Five built-in preset chips ("translate to formal Spanish", "make this sound more diplomatic", "explain like I'm five", "summarize in one sentence", "convert to bullet points") fill the textarea on click. `⌘+↵` runs the translation; `Esc` cancels. Custom instructions are never persisted.

### Translating overlay

While a translation is in flight, the prompt window switches to an overlay showing the action label, the active provider model, an animated lime sweep bar, and an elapsed-time counter. Press `Esc` or click **Cancel** to drop the in-flight result. The HTTP request continues to its 30-second natural timeout but its outcome is discarded silently — no notification fires for cancelled work.

### Reduced motion

When macOS Reduce Motion is enabled (System Settings → Accessibility → Display → Reduce Motion), the overlay's animated bar is replaced with a static "Translating…" label, per WCAG 2.3.3. The setting is read once at app launch — toggle requires a restart.

### Large-clipboard confirmation

To prevent surprise API costs on accidental large pastes, clipboards exceeding `[ui].confirm_size_threshold` characters (default `2000`) trigger a confirmation modal showing the character count and a 300-character preview before the request is sent. To raise or lower this threshold, edit `config.toml`:

```toml
[ui]
confirm_size_threshold = 5000
show_preview = true            # hides the prompt-window preview block when false
```

Set the threshold to `0` to confirm any non-empty clipboard (useful for debugging).

## Limitations in M3

- **Cancellation drops the *result*, not the in-flight HTTP request.** The provider call continues to its 30-second timeout — a billing nuance, not a UX bug.
- **`reduced_motion` is read at startup only.** Toggling macOS Reduce Motion mid-session has no effect until the next launch.
- **Bundled fonts** (Inter / JetBrains Mono per the design handoff) are not yet bundled; the app uses egui's default Hack/Ubuntu fonts. Out of scope for v0.1.
- **macOS tested only.** Linux/Windows binaries build in CI but have not been manually verified.
- **Env-var API key only.** Keychain support lands in M6.
- **No glossary, no history, no setup wizard.** All later milestones.
- **Built-in templates only.** User-overrideable templates land in M4.

### M4: Glossary + custom templates + SIGHUP reload

#### Glossary file

Place a `glossary.toml` in your config dir (macOS:
`~/Library/Application Support/clipboard-translator/glossary.toml`). Each
entry pins a source term to a fixed translation, optionally scoped to
language pairs:

```toml
[[entry]]
source = "Smart Table"
target = "Smart Table"
languages = ["*"]            # applies to every language pair

[[entry]]
source = "Vorgang"
target = "case"
languages = ["de->en"]       # only when translating German → English

[[entry]]
source = "GIP"
target = "GIP"
languages = ["*"]
note = "Always preserve as-is"
```

When the prompt window opens, matched terms appear in a chip strip above
the slot list ("GLOSSARY WILL INJECT: ..."). At translation time, the
pair-scoped subset is rendered into the system prompt as:

```
GLOSSARY — these terms MUST be translated exactly as specified:
- "Smart Table" → "Smart Table"
- "GIP" → "GIP" (Always preserve as-is)
```

Configure matching strategy via `[glossary]` in `config.toml`:

```toml
[glossary]
enabled = true
file = "glossary.toml"
case_sensitive = false
matching = "auto"            # auto | word_boundary | substring
```

`auto` (default): word_boundary for whitespace-using languages; substring
for `zho`, `jpn`, `tha`, `lao`, `mya`, `khm`. If your source-language
detection lands below the confidence threshold, `auto` falls back to
word_boundary (the safer choice for most target languages).

If `glossary.toml` is malformed, the app logs a warning at startup and
continues with no glossary. Editing the file and sending `SIGHUP` to the
running process reloads it without a restart:

```bash
pkill -HUP clipt9n
```

(SIGHUP reload is Linux + macOS only. The M7 tray menu's "Reload
glossary" item will provide a cross-platform alternative.)

#### Custom template overrides

The four built-in prompt templates are overridable via files in your
config dir's `templates/` folder. To override one, create the file at the
path listed in `[templates]` (defaults are `templates/<action>.j2`):

```
~/Library/Application Support/clipboard-translator/
└── templates/
    ├── translate.j2     ← overrides the built-in translate template
    ├── fix_grammar.j2
    ├── rewrite.j2
    └── custom.j2
```

Available variables:
- `{{ source_language }}` — auto-detected via whatlang; may be `unknown`
- `{{ target_language }}` — human-readable name (e.g., `"Deutsch"`)
- `{{ user_instruction }}` — only set in the `custom` template
- `{{ glossary_block }}` — pre-rendered glossary directives, or empty

Malformed templates abort startup with a `<file>:<line>` error.
References to undeclared variables likewise abort startup. Templates
are NOT reloaded on SIGHUP — restart the app after editing.

To force a built-in for a specific action, set its path to `""` in
`config.toml`:

```toml
[templates]
translate = ""               # use built-in regardless of file presence
custom = "templates/custom.j2"
```

#### M4 limitations (carried forward)

- whatlang's confidence threshold is hard-coded at 0.5. Misclassification
  on very short clipboards is best-effort; pair-scoped glossary entries
  may not fire when the language is low-confidence.
- The chip strip preview is pair-agnostic — it shows every term that
  matches the clipboard regardless of whether the pair will scope it
  out at translation time. This is informational; the translator
  applies pair scoping correctly.
- Templates can't be reloaded without a restart (only the glossary is
  hot-reloadable). M8 may add a tray-menu "Reload templates" action if
  there's demand.

### Encrypted history (M5)

clipt9n persists every successful translation to a local SQLite
database at `<config_dir>/history.db`. Source and result text are
encrypted at the application layer with ChaCha20-Poly1305; metadata
(timestamp, action, language pair, character count) is plaintext for
search. The encryption key is derived via Argon2id from a 32-byte
secret stored at `<config_dir>/.history-key` (mode `0600` on Unix).

#### Opening the viewer

Press **Cmd+Shift+H** (macOS) / **Ctrl+Shift+H** (Linux/Windows) to
open the history viewer. The viewer is a 680px window with:

- Top search input (filter as you type — matches source, result,
  action, and language pair)
- Scrollable list of recent entries (newest first; capped at
  `[history] max_entries`, default 100)
- Detail block showing source and result side-by-side for the
  selected row
- Footer keymap:
  - `↵` (Enter) — copy the result back to clipboard, close viewer
  - `s` — copy the original source instead
  - `d` — delete the selected entry
  - `⇧+Del` — clear all entries (key file preserved)
  - `Esc` — close

#### `[history]` configuration

```toml
[history]
enabled = true            # set false to disable the database entirely
max_entries = 100         # older rows pruned at insert time
store_text = true         # set false to record metadata only (no source/result)
confirm_clear = true      # ⇧+Del prompts before wiping; set false for instant clear

[hotkey.history]
modifier = "cmd"          # "cmd" → Cmd on macOS, Ctrl on Linux/Windows
shift = true
key = "H"
enabled = true            # set false to skip registering the history hotkey
```

#### Key file caveats (M5 → M6 migration)

The current keyfile fallback (`<config_dir>/.history-key`) is the M5
implementation of spec §7's key storage. M6 adds OS-keychain support
via the `keyring` crate; on first run after upgrading, the keyfile
will be migrated into Keychain (macOS) / Credential Manager (Windows)
/ Secret Service (Linux) and the file will be left in place for safety.

The keyfile mode is less secure than the keychain mode because:
- A reader of `<config_dir>` with sufficient permissions can read it
  (mitigated by `0600` on Unix; subject to user-profile ACL on Windows).
- It doesn't benefit from the OS keychain's per-app sandboxing.

Treat M5 as appropriate for personal use; production use should wait
for M6.

#### Failure modes

| Condition | Behavior |
|---|---|
| Keyfile or DB missing | Created on first translation; no error. |
| DB corruption | Toast on viewer open: "History database unreadable. New history will not be saved." App continues to function. Delete `history.db` manually to reset. |
| Wrong key (e.g., keyfile regenerated) | Existing rows can't be decrypted; viewer shows them as skipped. New rows write fine. |
| Disk full / write failure | Logged; clipboard write succeeds; only the history record is lost. |

### Setup wizard + keychain (M6)

On first launch, clipt9n shows a 3-step setup wizard if no API key is
found in the keychain and no environment variable is set. The wizard
covers:

1. **Provider** — Anthropic (Claude, recommended), OpenAI, Google
   Gemini (via OpenAI-compat shim), or Ollama (local).
2. **Key** — paste your API key with show/hide toggle. Storage radio:
   System Keychain (default — bound to clipt9n; other apps prompted
   on read) or Environment variable (the wizard shows the variable
   name; user must export it before next launch).
3. **Verify** — connectivity check (`GET /v1/models`, no token spend)
   and an optional sample translation (`Hello, world.` → German). One
   auto-retry per check; failure shows a `401 Invalid API key`-style
   error inside the wizard, with an "Open config" recovery button.

After "Save and start ✓", the wizard:
- Writes the key to the OS keychain (macOS Keychain Services, Windows
  Credential Manager, Linux Secret Service) via the `keyring` crate.
- Rewrites `<config_dir>/config.toml` with `[provider.api_key] source
  = "keychain"` so subsequent launches resolve the key from there.
- Closes the wizard and lands in the normal idle state.

#### Migration from M5

On the first M6 launch with a keychain available, the existing
`<config_dir>/.history-key` (M5's encryption-key fallback) is COPIED
into a `history-key` keychain entry. **The file is left in place.**
After verifying that history works (Cmd+Shift+H opens the viewer with
your existing entries), you can `rm <config_dir>/.history-key`. Don't
delete it before verifying — there is no rollback.

#### Keychain unavailable (Linux without Secret Service)

If the keychain probe fails (typical: a Linux desktop without a
Secret Service provider running, e.g., a headless server with a
DISPLAY exported), the wizard hides the Keychain radio and forces
storage = Environment variable. The user must `export ANTHROPIC_API_KEY=...`
(or the equivalent) before the next launch. clipt9n logs a warning at
startup explaining this.

#### Resolution order

```
keychain (if cfg.provider.api_key.source = "keychain")
    ↓ (NoEntry / unavailable)
environment variable (cfg.provider.api_key.env_var)
    ↓ (var missing)
setup wizard (re-run via M7 tray menu)
```

The CLI mode (`clipt9n --action translate ...`) inherits this same
resolution order automatically — no separate keychain wiring needed.

#### Failure modes

| Condition | Behavior |
|---|---|
| First launch, keychain available, no env var | Wizard opens; user enters key; saved to keychain. |
| First launch, keychain unavailable, no env var | Wizard opens with env-only mode; user is told to set env var. |
| Existing keychain entry, normal launch | No wizard; key resolved from keychain; normal startup. |
| Keychain returns a stale/wrong key (revoked at provider) | First translation hits 401; M7's tray menu offers "re-run wizard". |
| Save-and-start fails to write keychain | Wizard surfaces the error, stays open with key intact. |

#### Manual smoke matrix (M6)

This matrix is the human verification of the setup wizard's full flow.
The M8 macOS Day-1 + VoiceOver pass tracked in `TESTING.md` owns the
release-blocking checklist; the steps below are a fuller walk-through
of the wizard for diagnostic use. Steps:

1. **First-launch with no key**
   - Delete `<config_dir>/.history-key` and any `clipt9n` keychain
     entries from Keychain Access.app.
   - Unset the env var: `unset ANTHROPIC_API_KEY`.
   - Launch the binary. Wizard opens with viewport size 580×640.
   - Verify the provider grid shows all 4 options; default selected
     is "anthropic".

2. **Invalid-key error path**
   - Type `sk-ant-bad-key`. Click Verify. Connectivity row turns red
     with "401 Invalid API key" in the err-box.
   - Click into the key field, replace with a real key. Verify the
     err-box dismisses and the connectivity row resets to idle.

3. **Sample-translation skip warning**
   - Uncheck the "Test with a real translation" checkbox. Click Verify.
   - Connectivity row turns green; sample-translation row is hidden.
   - Phase advances to Done; "Save and start ✓" button appears.

4. **Keychain-unavailable fallback (Linux)**
   - On a Linux box without Secret Service: launch the binary.
   - The Keychain radio is hidden; Env-only mode is forced; the
     wizard explains in the storage row.

5. **Restart picks up keychain key**
   - After Save-and-start with a real key, quit the app.
   - Relaunch — the wizard does NOT open. The prompt window summons
     normally on Cmd+Shift+T and the translation completes.

6. **macOS Accessibility-permission revoked**
   - In System Settings → Privacy & Security → Accessibility, remove
     clipt9n. Relaunch. The pre-existing M2 modal points the user to
     re-grant permission. (This is M2-owned; M6 just doesn't break it.)

7. **Migration: keyfile → keychain**
   - On a fresh M6 install with an existing M5 `<config_dir>/.history-key`,
     launch. Tracing logs show: `M5 keyfile migrated to keychain`.
   - In Keychain Access.app, search for the service name (e.g.,
     `clipboard-translator`); a `history-key` entry is present.
   - The file is still on disk. After Cmd+Shift+H confirms history
     works, `rm <config_dir>/.history-key` is safe.

8. **Cross-platform discipline (one more check)**
   - The grep from Step 11.4 still returns empty (M6 added no new
     `cfg(target_os)` outside `platform/`).

## M7 — tray icon + accessibility polish

### Tray menu

Default-on. The icon lives in the macOS menu bar / Linux StatusNotifierItem
/ Windows shell tray. The menu has seven items:

- **Translate clipboard** — equivalent to pressing the prompt hotkey
- **Open history** — equivalent to the history hotkey (M5)
- **Open glossary** — opens `glossary.toml` in the system default editor (via `Platform::open_path`, M6)
- **Reload glossary** — re-reads the file without restart (re-uses M4's SIGHUP channel)
- **Re-run setup wizard** — re-enters the M6 wizard (use after key rotation, or when stale-key 401s auto-open it)
- **Hide icon** — confirms via modal showing the live hotkey, then persists `state.tray.visible = false`
- **Quit clipt9n** — clean shutdown

The icon's bottom-right corner has a colored dot — the *status pill*:

| Color | Meaning | Tooltip |
|-------|---------|---------|
| Green | ready | clipt9n — ready |
| Red | no API key | clipt9n — no API key; click to run setup wizard |
| Amber | warning | clipt9n — \<reason\> (hover for the specific reason) |

Amber pill triggers, mapped to spec §8:
- *hotkey already in use* — another app claimed Cmd+Shift+T; tray menu remains the entry point
- *glossary malformed* — file parsed as Err at startup; running without it
- *accessibility permission revoked* (macOS) — global hotkey can't register; click for help
- *API key invalid* — translation 401'd; setup wizard auto-opens

### Recovering from "Hide icon"

Two paths, both documented in the hide-confirm modal:

1. **Re-run with `--show-tray`** — `clipt9n --show-tray` forces the tray on for this launch and persists `[tray] visible = true` for subsequent launches.
2. **Edit state.toml** — find the file at:
   - macOS: `~/Library/Application Support/clipboard-translator/state.toml`
   - Linux: `~/.config/clipboard-translator/state.toml`
   - Windows: `%APPDATA%\clipboard-translator\state.toml`
   
   Set `[tray] visible = true` and relaunch.

### Live provider rebuild

When the wizard's "Save and start" succeeds, the running provider is
rebuilt in place — no restart required. The next translation uses the
just-saved key. (Pre-M7 behavior required a restart.)

### Tray-construction failure is non-fatal

If the OS tray API fails (rare — minimal Linux DEs without
StatusNotifierItem support, sandboxed environments, etc.) the
constructor returns `Err`, the app logs warn, and continues without
a tray. The hotkey path still works. The same fallback applies if the
tray thread panics — `TrayHandle::build_with_panic_isolation` wraps
the macOS / Windows / Linux `tray-icon` build in
`std::panic::catch_unwind` so a tray-side panic doesn't crash the
prompt window.

### Accessibility (M7.B)

- **Focus rings** — every interactive widget renders the design's accent
  focus stroke when focused.
- **AccessKit labels** — every clickable surface has an explicit
  accessible name. Setup wizard provider cards now expose `Role::Button`
  via `WidgetInfo::labeled` (was `Frame + Sense::click`); show/hide key
  button announces "Show key (reveal as plain text)" / "Hide key (mask as
  password)" instead of the short toggle token; history search field
  carries an explicit "Search history" label; tray hide-confirm modal
  disables egui's default `fade_in` for reduced-motion compliance.
- **Reduced motion** — the translating-overlay spinner respects the
  cached macOS `NSReduceMotionEnabled` flag (M3). Other surfaces
  (size-confirm modal, tray-confirm modal) have no animations.
- **Tab / Shift+Tab** — egui's default tab order matches the visual
  top-to-bottom-left-to-right flow per design. kittest assertions cover
  AccessKit role + label exposure for each interactive surface; manual
  VoiceOver smoke is M8 scope.

### Manual smoke matrix (M7)

These flows must be exercised on real hardware before declaring v1.0.
The release-blocking version of this matrix lives in `TESTING.md`;
the table below is the M7-era reference list of expected behaviors.

| OS | Surface | Expected | Tested? |
|----|---------|----------|---------|
| macOS | Tray icon appears in menu bar at startup | Visible | ☐ |
| macOS | Click tray → menu drops down | Menu visible | ☐ |
| macOS | Translate clipboard menu item | Prompt window appears | ☐ |
| macOS | Open history menu item | History window appears | ☐ |
| macOS | Open glossary menu item | Default editor opens glossary.toml | ☐ |
| macOS | Reload glossary menu item | Glossary re-reads without restart | ☐ |
| macOS | Re-run setup wizard menu item | Wizard window appears | ☐ |
| macOS | Hide icon → confirm | Tray disappears; relaunch w/o flag still hidden | ☐ |
| macOS | Hide icon → cancel | Tray remains; no state.toml change | ☐ |
| macOS | Relaunch with --show-tray | Tray reappears; subsequent launches show it | ☐ |
| macOS | Stale API key 401 | Wizard auto-opens; tray pill flips amber → red | ☐ |
| macOS | Accessibility permission revoked | Tray pill amber; tooltip says "permission needed" | ☐ |
| macOS | Glossary malformed | Tray pill amber; app still translates | ☐ |
| macOS | Hotkey already in use | Tray pill amber; tray menu remains the entry point | ☐ |
| macOS | Wizard Save-and-start | Next translation uses new key with no restart | ☐ |
| macOS | VoiceOver pass on prompt / history / wizard / tray-confirm | Roles + labels announced; focus order matches visual flow | ☐ |
| Linux | Tray icon in supported DE (GNOME/KDE) | Visible via StatusNotifierItem | ☐ |
| Linux | Tray icon in headless DE (no SNI) | Logs warn; hotkey still works | ☐ |
| Linux | Open glossary launches via xdg-open | Default editor opens | ☐ |
| Windows | Tray icon in shell tray | Visible | ☐ |
| Windows | Right-click tray → menu | All 7 items present | ☐ |
| Windows | Open glossary via cmd /C start | Default editor opens | ☐ |

## Troubleshooting

| Symptom | Where to look |
|---------|---------------|
| Hotkey does nothing on macOS | [macOS Accessibility permission](#macos-accessibility-permission) — re-grant in System Settings → Privacy & Security → Accessibility. |
| Setup wizard says "keychain unavailable" | [Keychain unavailable (Linux without Secret Service)](#keychain-unavailable-linux-without-secret-service) — env-only mode is the documented fallback. |
| Tray pill is amber | Hover the tray icon. Likely *glossary malformed*, *hotkey already in use*, or *Accessibility permission revoked*. App still works via the tray menu. |
| Translation 401s after a known-good key | Stale-key flow: the wizard auto-opens; re-enter or rotate the key; "Save and start" rebuilds the provider in place — no restart needed. |
| History viewer says disabled | Either the `.history-key` is unreadable or `history.db` is corrupted. Delete `<config_dir>/history.db` to reset (decrypted contents are unrecoverable). |
| Tray icon is missing on Linux | Check the desktop environment supports StatusNotifierItem (KDE, GNOME with the AppIndicator extension). Hotkey path still works without a tray. |
| Tray hidden after "Hide icon" | Relaunch with `clipt9n --show-tray` or set `[tray] visible = true` in `state.toml` (paths in [Recovering from "Hide icon"](#recovering-from-hide-icon)). |

## Packaging

### macOS `.app`

```bash
cargo install cargo-bundle
scripts/package-macos.sh
```

The script runs `cargo bundle --release --format osx`, patches `Info.plist` so `LSUIElement = true` (menu-bar-only — no Dock icon), and ad-hoc-signs the bundle (`codesign --sign -`). Result: `target/release/bundle/osx/clipt9n.app`.

For personal distribution: ship the `.app` directly. Recipients open with **right-click → Open → Open Anyway** (Gatekeeper bypass for unnotarized apps). Apple notarization requires a paid Developer account and is out of scope for v0.1.

### Linux binary + `.desktop`

```bash
scripts/package-linux.sh
```

Produces a staged tree under `target/release/package-linux/clipt9n/` with `bin/clipt9n`, an icon at `share/icons/hicolor/256x256/apps/`, and a `dev.egecan.clipt9n.desktop` launcher. Tray support depends on a working StatusNotifierItem host (KDE, GNOME with the AppIndicator extension, etc.).

### Windows `.exe`

```bash
cargo build --release
```

Ship `target/release/clipt9n.exe`. The tray icon lives in the system tray; right-click for the menu.

## Development

```bash
cargo build
cargo test                    # all tests, ~30 unit + integration
cargo clippy --all-targets    # lints
cargo fmt                     # formatting
```

### Latency benchmark

Run `scripts/bench.sh` with a real provider key. The script writes a Markdown report to `docs/benchmarks/<date>.md`. M8's release target is p50 < 800 ms and p95 < 2000 ms with Anthropic Haiku 4.5 on the maintainer's macOS dev hardware.

## License

MIT.
