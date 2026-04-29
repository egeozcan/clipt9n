# clipt9n — clipboard translator

A keyboard-driven clipboard translator. Press a hotkey, pick an action, get the result back on your clipboard.

> **Status: M3 — all 6 actions + custom prompt + translating overlay.**
> CLI mode still works (M1 behavior). GUI mode summons the prompt window on Cmd+Shift+T; all six slots work end-to-end with an in-flight overlay and large-clipboard confirmation. macOS tested. Linux/Windows binaries from CI but untested. See `docs/superpowers/specs/2026-04-28-clipt9n-implementation-design.md` for the full milestone roadmap.

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
- **Bundled fonts** (Inter / JetBrains Mono per the design handoff) are deferred to M8 polish; M3 still uses egui's default Hack/Ubuntu fonts.
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

## Development

```bash
cargo build
cargo test                    # all tests, ~30 unit + integration
cargo clippy --all-targets    # lints
cargo fmt                     # formatting
```

## License

MIT.
