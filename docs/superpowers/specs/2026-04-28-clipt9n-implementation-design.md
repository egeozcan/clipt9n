# clipt9n — Implementation Design

**Date:** 2026-04-28
**Status:** Approved by user 2026-04-28; pending implementation plan
**Sources:** `clipboard-translator-spec.md.pdf` (technical spec), `clipt9n-handoff.zip` (visual design from Claude Design)

---

## Purpose of this document

The technical spec and the design handoff together define **what** clipt9n is. This document captures **how** we will execute the build:

- The scope we're targeting in this engagement (v0.1)
- Milestone decomposition with exit criteria
- Platform-abstraction discipline (so v1.0 cross-OS work is config-and-test, not a refactor)
- Accessibility baseline (the design did not get an a11y review; we fix as we build)
- Decisions made during brainstorming that supersede or clarify the spec

The spec PDF and design handoff remain authoritative for component behavior and visual treatment. This doc does not restate them — it directs how to assemble them.

---

## Scope: v0.1

**In scope:**

- Full feature set per the spec (clipboard hotkey, all 4 actions, glossary, encrypted history + viewer, setup wizard, tray, notifications, post-processing, last-action recall)
- macOS only as a tested, shipping target (universal binary: `x86_64-apple-darwin` + `aarch64-apple-darwin`)
- Anthropic provider (Claude Haiku 4.5 default) and OpenAI-compatible provider (works with OpenAI/Gemini/Ollama via base URL config); both required because the setup wizard offers all four
- WCAG 2.1 AA contrast and keyboard-accessibility baseline

**Deferred to v1.0 (post-v0.1):**

- Linux (X11 + Wayland) and Windows tested + signed builds
- macOS notarization (ad-hoc signing only in v0.1; documented in README)
- Manual cross-OS test matrix
- AppImage / MSI / cargo-wix packaging variants

**Out of scope per spec, confirmed:**

- In-app glossary editor (the design's faux-IDE `GlossaryWindow` is a stretch; v1 ships "Open glossary" → system editor only)
- Streaming responses
- First-class local model setup wizard (manual Ollama config works via OpenAI-compat)
- Browser extension, mobile, batch translation

---

## Milestones

Each milestone is its own implementation plan, written and executed in a separate session. Each milestone is independently shippable as a `0.0.x` build (we don't tag releases until v0.1 is complete, but each milestone leaves `main` in a working state).

### M1 — Walking skeleton

**Goal:** prove end-to-end wiring (config → clipboard → API → clipboard) without any UI.

**Deliverables:**

- Cargo workspace with one binary, dependencies pinned
- `src/config.rs` loads `<config_dir>/config.toml` per spec §6 schema; defaults applied for missing keys
- `src/clipboard.rs` — `arboard` wrapper with text-only filtering
- `src/secrets.rs` — env-var path only in M1 (`ANTHROPIC_API_KEY`); keychain comes in M6
- `src/llm/` — `LlmProvider` trait, `AnthropicProvider`, `OpenAiCompatibleProvider` (both); `reqwest` + `rustls-tls`, 30s timeout, retry policy on 5xx per resolution below
- `src/llm/prompts.rs` — built-in templates as `const &str` (translate / fix_grammar / rewrite / custom from spec §5.3)
- `src/llm/templates.rs` — minijinja rendering of built-in templates only. **No file-based override loading in M1** — that's M4's responsibility.
- `src/translator.rs` — selects template, renders, calls provider, post-processes (spec §5.6)
- `src/error.rs` — unified `TranslateError`
- CLI flags: `--translate-to=<code>`, `--fix-grammar`, `--rewrite`, `--custom="..."`
- `tracing` + `tracing-subscriber`; logs metadata only (never clipboard text, never API key)
- `zeroize::Zeroizing<String>` wraps API key and clipboard text in transit

**Exit criteria:**
1. `ANTHROPIC_API_KEY=… clipt9n --translate-to=de` reads system clipboard, returns German translation, writes to clipboard.
2. Same for `--fix-grammar`, `--rewrite`, `--custom="..."`.
3. Non-text clipboard exits cleanly with stderr message and non-zero status.
4. Anthropic 5xx triggers **two automatic retries** before erroring: sleep 1s before retry #1, sleep 2s before retry #2 (3 attempts total). Verified via wiremock returning 503 three times. (Resolves spec §8 ambiguity — "exponential backoff (1s, 2s)" implies two retry intervals.)
5. Unit tests pass for: post-processing (quote/preamble stripping), built-in template rendering with and without `glossary_block`, config loading defaults.
6. CI workflow in `.github/workflows/build.yml` builds all 5 targets (compile-only).

### M2 — Prompt window + global hotkey + design tokens

**Goal:** the app feels real for the first time. Cmd+Shift+T → prompt → translate to one of three languages → result on clipboard.

**Deliverables:**

- `src/main.rs` event loop with `eframe`, `accesskit` feature enabled
- `src/ui/theme.rs` — design tokens lifted from handoff palette, with **a11y-corrected `--ink-3` and disabled-state colors** (see §"A11y baseline" below). Exposes `egui::Visuals` builder + a `kbd()` widget and `WindowFrame` shell so all subsequent windows inherit the look.
- `src/ui/prompt.rs` — prompt window per design `prompt-window.jsx`: 460–520 px wide, source preview with lang badge + char count, 6 numbered slots (only 1–3 wired up in M2; 4–6 render but no-op with placeholder text in M3), footer keymap. Number keys 1–6 trigger; Enter repeats last action; Esc cancels.
- `src/platform/mod.rs` with `ensure_hotkey_permissions()` no-op default and `platform/macos.rs` providing the real impl (detect Accessibility permission, open System Settings + show modal if missing)
- `global-hotkey` registration with config-driven modifiers/key (cmd→ctrl mapping helper in `config.rs`)
- `notify-rust` "Translation copied" toast on success
- Always-on-top, no-decorations, centered-on-active-monitor window (spec §3 "Window behavior")
- `state.toml` write/read for last-action persistence (slots only; never custom prompts)

**Exit criteria:**
1. Hotkey opens window centered on the active display; window auto-focuses; keyboard shortcuts work without a click.
2. Pressing 1, 2, or 3 runs end-to-end translation against the user's real clipboard, replaces it, shows OS notification, closes window.
3. Pressing Enter on second invocation repeats slot from previous run.
4. Esc closes window, restoring focus to previous app.
5. Empty / non-text clipboard shows the empty state per design.
6. Visual matches design's `prompt-window.jsx` for a normal clipboard. (Pixel-faithful is the goal; minor egui-specific deviations documented in M2 plan.)
7. **Every interactive element has a visible focus ring** (M2-introduced a11y check; see §"A11y baseline").
8. `accesskit` reports labels for all 6 slots and the close button (verified by `cargo run --example dump_accesskit_tree` or VoiceOver smoke).

### M3 — All 6 actions + custom prompt window + post-processing UX

**Deliverables:**

- Slots 4 (fix grammar), 5 (rewrite) wired to existing translator
- Slot 6: opens custom prompt window — implemented in `src/ui/custom_prompt.rs`, modeled on the design's `custom-prompt.jsx`. Instruction textarea, preset chips, preview block, Cmd+Enter / Esc, primary "Run →" button.
- "Translating…" overlay window per design (`TranslatingWindow`): animated progress (with reduced-motion fallback), elapsed-time counter, cancel
- `confirm_size_threshold` guard before sending oversized clipboards (modal "this is X chars, send to API?")
- Source-preview truncation respects `[ui] show_preview = true`
- Glossary chip preview area exists in prompt window but renders empty (M4 fills it in)

**Exit criteria:**
1. All 4 actions produce correct outputs end-to-end.
2. Custom prompt accepts presets and free-form instruction; clears on close (does not persist).
3. Translating overlay appears for any action >150ms; cancellable with Esc.
4. Reduced-motion: when macOS "Reduce Motion" is on, progress is a static label.
5. Size confirmation: pasting 2500 chars triggers modal; 1500 chars does not.

### M4 — Templates with overrides + glossary

**Deliverables:**

- `src/glossary.rs` — load `glossary.toml`; pair-key matching (`*`, `de->en`, etc.); `auto`/`word_boundary`/`substring` matching strategies; `whatlang` for source-language detection (with `unknown` fallback rule per spec); glossary block formatting
- **Template override loader** added to `src/llm/templates.rs`: if `<config_dir>/templates/*.j2` exist, replace built-in for that action. Owns malformed-template (file+line) and missing-required-variable startup errors. (M1 only renders built-ins; M4 introduces user-overridable templating in full.)
- Glossary chip preview in prompt window (top of menu, below preview block, per design)
- SIGHUP handler in `platform/unix.rs` (Linux/macOS) to reload glossary; tray "Reload glossary" menu item placeholder (real menu in M7)

**Exit criteria:**
1. Glossary entries that match source text inject `{{ glossary_block }}` block correctly per spec §5.4.
2. Glossary scope `*` and `de->en` filter correctly.
3. Auto-matching uses word_boundary for de/en/tr/fr/es and substring for zho/jpn/tha (test cases for both).
4. Empty glossary block renders cleanly without trailing whitespace.
5. Malformed glossary disables glossary for the session, logs warn at startup, app continues.
6. Override `templates/translate.j2` replaces built-in. Malformed template aborts startup with file+line. Template referencing an unknown variable aborts startup with file+line.
7. SIGHUP reloads glossary without restart.

### M5 — Encrypted history + viewer

**Deliverables:**

- `src/history/crypto.rs` — Argon2 KDF + ChaCha20-Poly1305 AEAD; per-row nonce; `history-key` keychain account name (or `<config_dir>/.history-key` 0600 fallback with startup warning)
- `src/history/store.rs` — rusqlite (bundled feature); spec §7 schema; create-on-first-run; CRUD; insert is best-effort (failure logged, never blocks clipboard write)
- `src/ui/history.rs` per design `history-window.jsx`: 680 px, search input, scrollable list (truncated source), source/result detail block, footer keymap
- Hotkey `Cmd+Shift+H` (configurable, can be disabled)
- Real-time filter; arrow-key nav; Enter copy result; `s` copy source; `d` delete; Shift+Del clear-all with confirm modal; Esc close
- `[history] enabled = false` short-circuits all writes
- `[history] store_text = false` writes metadata-only rows

**Exit criteria:**
1. Translation persists across app restarts; viewer shows it after restart.
2. Wrong key (simulated by deleting `history-key`) leaves rows undecryptable; viewer shows "history unreadable" toast on startup, app continues.
3. Search latency p95 <50 ms on 100 entries with text.
4. Clear-all wipes rows but preserves the key.
5. Round-trip unit test: encrypt → store → load → decrypt for 100 random strings of varying length.
6. Argon2 derivation is deterministic for a given (secret, salt).

### M6 — Setup wizard + keychain

**Deliverables:**

- `src/secrets.rs` — keychain via `keyring` crate; resolution order keychain → env → setup wizard
- `src/ui/setup.rs` per design `setup-wizard.jsx`: provider grid (Anthropic/OpenAI/Gemini/Ollama), key entry with show/hide, storage radio (Keychain/env), test-translation checkbox, two CheckRow status dots
- Connectivity check: `GET /v1/models` for **all** providers (Anthropic, OpenAI, Gemini, Ollama). Free, idempotent, doesn't spend tokens, doesn't duplicate the sample-translation step. (This corrects spec §7's `POST /v1/messages` instruction — Anthropic now exposes a `GET /v1/models` endpoint suitable for auth verification.)
- Sample translation: `Hello, world.` → German via configured model
- Keychain-unavailable detection (any OS): setup wizard hides keychain option, shows env-only with explanation. Spec §7 lists Linux without Secret Service as the realistic case; the check is OS-agnostic.
- Failure recovery: stay open with key intact; "Open config" button for manual edits

(Accessibility-permission detection is owned by M2 — runs on every startup, not just first-run, since hotkey registration depends on it.)

**Exit criteria:**
1. First launch with no API key shows wizard.
2. Invalid key triggers connectivity-check failure with `401 Invalid API key` shown; user can fix without re-typing the whole key.
3. Sample translation can be unchecked and skipped (warning shown).
4. After Save and start, key persists in Keychain and `[provider.api_key] source = "keychain"` is written to `config.toml`.
5. Restart picks up keychain key without prompting.
6. macOS without Accessibility permission shows modal pointing to Settings.

### M7 — Tray + glossary launch + Accessibility polish

**Deliverables:**

- `src/tray.rs` — `tray-icon` crate; menu per design `desktop.jsx` `TrayMenu`: Translate / History / —— / Open glossary / Reload glossary / —— / Hide icon (with `--show-tray` recovery, hide-confirmation modal showing live hotkey from config) / Quit
- "Open glossary" launches `xdg-open` / `open` / `start` for the file
- Tray status pill: "ready" / "no API key" colors per design
- StatusNotifierItem support documented for Linux DE compatibility (no Linux-specific code; just relies on `tray-icon`)
- macOS menu-bar binding to app-bundle identifier
- Final pass on focus rings, AccessKit labels, reduced-motion behavior across all windows

**Exit criteria:**
1. Tray menu opens; all items dispatch correctly.
2. Hide icon → confirm modal shows actual hotkey from config.toml; tray hides; `--show-tray` flag re-enables on next launch.
3. macOS Accessibility permission revoked → tray icon shows warning state (per spec §8).
4. VoiceOver smoke test: prompt window + history window + setup wizard all readable; element roles announced; focus order matches visual order.
5. Tab + Shift+Tab navigates every window correctly.

### M8 — Tests, packaging, CI, polish

**Deliverables:**

- Comprehensive unit tests per spec §11 (templates, post-processing, config, glossary scoping/matching, crypto, Argon2 determinism)
- Integration tests with `wiremock`: provider clients, retry logic, timeout, rate-limit `Retry-After`
- Integration tests with in-memory SQLite: history end-to-end
- Manual test matrix in `TESTING.md` (9 lang combos × actions; edge cases; setup wizard failure modes)
- Latency benchmark script (`scripts/bench.sh`) targeting p50<800ms, p95<2000ms with Haiku 4.5 across 20 snippets
- Fuzz targets (`cargo-fuzz`) for glossary parser, template renderer, history decryption (must reject tampered ciphertext)
- `cargo-bundle` config for `.app`; ad-hoc signing in CI; README documents notarization for personal distribution
- GitHub Actions: build matrix (5 targets compile, macOS test); release workflow on `v*.*.*` tag
- README, LICENSE, CHANGELOG
- Cross-platform abstraction lint: a script that greps for `#[cfg(target_os` outside `platform/` and `secrets.rs` and fails CI if found
- Manual: VoiceOver pass on all windows; contrast verified with macOS Display Accommodations

**Exit criteria:**
1. `cargo test` green.
2. `cargo bundle --release` produces a runnable `.app` on a clean macOS.
3. CI matrix green for all 5 targets (compile).
4. Latency benchmark passes targets.
5. README documents: install, first-run, hotkey customization, glossary editing, Linux/Windows "untested in this release" notice, troubleshooting.

---

## Module layout

Inherits from spec §4 verbatim, with two additions:

```
src/
├── main.rs
├── config.rs
├── clipboard.rs
├── secrets.rs
├── translator.rs
├── error.rs
├── notify.rs
├── tray.rs
├── ui/
│   ├── mod.rs
│   ├── theme.rs        ← NEW: design tokens, a11y-fixed palette, kbd widget, WindowFrame
│   ├── prompt.rs
│   ├── custom_prompt.rs
│   ├── translating.rs  ← NEW: overlay surface (spec implies, design renders)
│   ├── history.rs
│   └── setup.rs
├── llm/
│   ├── mod.rs
│   ├── client.rs
│   ├── anthropic.rs
│   ├── openai.rs
│   ├── prompts.rs
│   └── templates.rs
├── glossary.rs
├── history/
│   ├── mod.rs
│   ├── store.rs
│   └── crypto.rs
└── platform/
    ├── mod.rs          ← NEW: trait surface (no-op defaults)
    ├── macos.rs        ← NEW: accessibility-permission check; reduced-motion query
    └── (linux.rs, windows.rs ship as stubs in M1; populated post-v0.1)
```

Two additions to spec §4:
1. `ui/theme.rs` and `ui/translating.rs` are new files (the spec's tree implies them but doesn't list them; the design makes their existence concrete).
2. `platform/` directory replaces the implicit "OS-specific bits live where they're used" assumption. **All `#[cfg(target_os = …)]` and `#[cfg(unix)]` blocks live here.** No exceptions — the SIGHUP handler from M4 also routes through `platform/unix.rs`.

---

## Cross-platform discipline

The user's directive: "make sure that it'll be possible to support other platforms eventually with no big refactor."

**Invariants held from M1:**

1. **All OS-specific code lives behind `platform/` APIs.** Cross-platform features (clipboard, hotkey, keychain, notification, tray, paths, GUI) go through wrapper crates: `arboard`, `global-hotkey`, `keyring`, `notify-rust`, `tray-icon`, `directories`, `eframe`. OS-only features the app genuinely needs — macOS Accessibility-permission detection (M2), reduced-motion query (M3), platform-native file launch for "Open glossary" (M7), Unix SIGHUP handler (M4) — live as functions in `platform/macos.rs`, `platform/linux.rs`, `platform/windows.rs`, `platform/unix.rs`, exposed through trait surfaces in `platform/mod.rs` with no-op or error-returning defaults for OSes that don't need them. **No direct OS API calls outside `platform/`.**
2. **One mapping function for hotkey modifiers.** `config::resolve_modifier(Modifier::Cmd)` returns `Cmd` on macOS, `Ctrl` on Linux/Windows. Used everywhere — including UI strings showing the active hotkey.
3. **`platform/` is the only place `#[cfg(target_os = …)]` and `#[cfg(unix)]` live.** Enforced by an M8 grep-lint in CI.
4. **CI builds all 5 targets from M1.** Compile-only on Linux/Windows in v0.1; runtime tests gated on macOS. This catches accidental macOS-only API use the day it's introduced.
5. **Linux Wayland note in README only:** the user-facing limitation (need `xdg-desktop-portal-gnome ≥46` or KDE equivalent for global hotkeys) is documented, not coded around. v1.0 does the manual cross-OS smoke test that confirms this.
6. **No conditional UI code.** Every window is identical on every OS. Differences (Cmd vs Ctrl in keymap displays) come from the modifier-mapping helper, not from `#[cfg]`.
7. **Test gating:** v0.1 ships macOS-only as a *tested* target. Linux/Windows binaries from CI exist but the README labels them "untested in this release."

What this buys you: making v1.0 cross-OS-tested becomes a milestone of *manual* testing + bug-fixing + signing/notarization workflow setup. No code reorganization.

---

## A11y baseline

The design did not get an accessibility review. Issues identified during brainstorm and the corresponding fixes:

### Contrast fixes (applied in M2's `ui/theme.rs`)

| CSS var | Original | Issue | Replacement |
|---|---|---|---|
| `--ink-3` | `#80869294` (8-char hex, alpha 58%, ~3.5:1 vs `#0e1014`) | Used for **every** footer kbd hint, lang code, preview meta label, glossary arrow, "last used" badge text — fails WCAG AA for normal text | Solid `#9ca3b1` — ~5.1:1, AA pass |
| Disabled fg on `--panel-3` | `--ink-3` (above) on `#23272f` ≈ 2.5:1 | Below 3:1 disabled-text floor | `#7a818d` solid on `--panel-3` — ~3.2:1 |
| `--muted` `#6c727d` (3.6:1) | Used for line-number gutter only | Acceptable as decorative chrome | No change. Comment in code documents this. |

All other palette values (ink, ink-2, accent, warn, bad, good) already meet AA.

### Keyboard accessibility

- **Visible focus indicators required on every interactive element.** The design uses `outline: "none"` everywhere — egui doesn't have this problem by default but we explicitly verify M2's `theme.rs` sets `visuals.widgets.active.bg_stroke` and `visuals.selection.bg_fill` to use the lime accent, and that focused buttons / inputs render a 2px lime ring.
- Tab order verified to match visual order in M2 (prompt window) and re-verified on every new window.
- Esc cancels everywhere (already in spec; verified per window).

### Screen reader support

- `eframe`'s `accesskit` feature enabled from M2.
- Custom widgets (`WindowFrame`, slot rows, kbd component) attach `WidgetInfo` with role + label.
- VoiceOver smoke test in M7 against all windows; documented bugs filed for v1.0 if any non-blocking.

### Motion

- Translating overlay's animated lime bar respects macOS Reduce Motion (queried at startup via `defaults read -g NSReduceMotionEnabled`).
- When reduced-motion is on, the overlay shows a static "Translating…" string.
- All animations stay <200ms, no flashing >3 Hz (WCAG 2.3.1 trivially satisfied).

### Color-only meaning audit

Pair colors in history list (lime/blue/purple/orange) appear to be color-only at a glance, but every row already shows the textual pair label (`grammar` / `rewrite` / `custom` / `EN → DE`) next to the colored text. Color is reinforcement, not the only channel. **No change required.**

### Status

- **WCAG 2.1 AA target.** AAA where it falls out for free; not chased.
- A11y is not an M8 polish pass — every milestone that introduces a new window owns its own focus-indicators-and-AccessKit verification before it's marked complete.

---

## Decisions made during brainstorming

These are clarifications and choices that go beyond what the spec specifies:

1. **`ui/theme.rs` and `ui/translating.rs`** are explicit modules (spec implied them; design materialized them).
2. **`platform/`** is a new top-level module to constrain OS-specific code. Spec did not call this out; cross-platform discipline requirement does.
3. **CI builds all 5 targets from M1** (compile-only on Linux/Windows). Spec §10 says "GitHub Actions matrix builds all five targets on push to main" — this happens at M1, not at the end.
4. **Provider scope:** both Anthropic and OpenAI-compat ship in M1. Setup wizard requires both for the provider grid to work.
5. **`--ink-3` palette correction** (`#80869294` → `#9ca3b1`) is a deliberate deviation from the design handoff to satisfy WCAG AA. Documented; aesthetically very close.
6. **In-app glossary editor (`GlossaryWindow` from design):** out of scope for v0.1 per spec §3 ("No in-app editor in v1"). The design's faux-IDE editor is a v2.0+ feature.
7. **Reduced-motion** support is in scope; spec did not call it out but the motion in the design (translating overlay) needs it.
8. **Anthropic connectivity check uses `GET /v1/models`** (not `POST /v1/messages` per spec §7). Anthropic now exposes a `GET /v1/models` endpoint usable for auth verification — free, idempotent, doesn't duplicate the sample-translation step.
9. **5xx retry policy** clarified to **two retries** (1s sleep, then 2s sleep; 3 attempts total) — resolves spec §8's ambiguous "One automatic retry with exponential backoff (1s, 2s)" wording.
10. **Template file loading owned by M4, not M1.** M1 renders built-in `const &str` templates only — the override loader, malformed-template error, and missing-variable check all land together in M4 alongside the rest of user-config-driven templating (which sits naturally next to the glossary loader, also user-config-driven).

---

## Open implementation-level questions (deferred to plan)

These are decisions best made when writing the M-specific plan:

- Exact `whatlang` confidence threshold below which to default to `unknown`. Spec §13 calls this out.
- Whether to bundle a small example glossary file in the install. Spec leans no; v0.1 confirms no.
- Setup wizard's connectivity-check retry policy. Spec leans one auto-retry, then surface; v0.1 confirms.
- egui font loading: bundle Inter + JetBrains Mono via `include_bytes!` (spec §2 "All assets bundled via include_bytes!").

---

## Next step

Hand this design off to the **superpowers:writing-plans** skill to produce the M1 implementation plan as the first executable artifact.
