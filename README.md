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

## Development

```bash
cargo build
cargo test                    # all tests, ~30 unit + integration
cargo clippy --all-targets    # lints
cargo fmt                     # formatting
```

## License

MIT.
