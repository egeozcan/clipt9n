# clipt9n — clipboard translator

A keyboard-driven clipboard translator. Press a hotkey, pick an action, get the result back on your clipboard.

> **Status: M1 — walking skeleton.**
> CLI only. macOS tested. Linux/Windows binaries from CI but untested. No GUI yet — that lands in M2. See `docs/superpowers/specs/2026-04-28-clipt9n-implementation-design.md` for the full milestone roadmap.

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

## Use (M1)

Copy text to your clipboard, then run one of:

```bash
clipt9n --translate-to=de             # translate clipboard to Deutsch
clipt9n --fix-grammar                 # fix grammar in source language
clipt9n --rewrite                     # rewrite for clarity
clipt9n --custom "make this formal"   # apply an arbitrary instruction
```

The translated/edited text replaces your clipboard contents.

## Limitations in M1

- **CLI only.** Global hotkey + GUI window land in M2.
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
