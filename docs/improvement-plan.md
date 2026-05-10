# clipt9n Improvement Plan

Derived from the May 2026 codebase review. Each section is self-contained
enough to be tackled in one session, in priority order.

---

## 1. Split `src/app.rs` into smaller modules

**Priority**: High  
**Risk**: Medium (large refactor, need to keep tests green)  
**Estimated effort**: 2–3 sessions

### Problem

`src/app.rs` is ~2300 lines mixing state-machine transitions, channel draining,
history viewer, setup wizard orchestration, clipboard handling, keyboard input,
async spawning, and pure helpers. It's the single largest file and hard to
navigate.

### Approach

Extract one module per major concern. Do this incrementally — one module per
commit, ensuring `cargo test` and `cargo build` stay green after each move.

**Step 1 — Extract pure helpers** (lowest risk, no state dependency):
- `app/pure.rs` (or keep them in `app.rs` if they're small enough — but these
  are testable without `ClipApp`):
  - `requires_size_confirm`
  - `selected_text_after_copy`
  - `next_gen`
  - `reset_focus_loss_latch` / `update_focus_loss_latch`
  - `action_kind_str`
  - `target_lang_for`
  - `overlay_label_for`
  - `action_label_for`
  - `decide_intent` / `translate_intent`
  - `Intent` enum

**Step 2 — Extract translation worker**:
- `app/translation.rs`:
  - `start_translation` (the tokio spawn + worker + watcher)
  - `handle_translation_done` (clipboard write, notification, history insert)
  - `schedule_history_insert`
  - `dispatch_translate` (size-confirm gate → `start_translation`)
  - `TranslationOutcome` struct

**Step 3 — Extract history viewer**:
- `app/history.rs`:
  - `summon_history`
  - `update_showing_history`
  - `handle_keys_history`
  - `refresh_history_model`
  - `dismiss_history_to_idle`

**Step 4 — Extract setup wizard**:
- `app/setup.rs`:
  - `update_setup_wizard`
  - `spawn_connectivity_check` + `run_connectivity_check`
  - `spawn_sample_translation_check`
  - `persist_setup_completion`
  - `dismiss_setup_to_idle`

**Step 5 — Extract prompt windows**:
- `app/prompt.rs`:
  - `show_window` / `show_window_from_selection` / `show_window_with_current_prompt_text`
  - `update_showing` / `handle_keys_showing`
  - `update_entering_custom` / `handle_keys_entering_custom`
  - `update_confirming_size`
  - `update_translating`

**Step 6 — Extract channel + tray + state helpers**:
- `app/channels.rs`:
  - `drain_channels`
  - `drain_tray_events`
  - `reload_glossary`
  - The various `dispatch_*` tray handlers
- `app/tray.rs`:
  - `compute_tray_status` / `refresh_tray_status`
  - `dispatch_hide_tray_request` / `update_confirming_tray_hide`
  - `dismiss_tray_modal_to_idle`

**Step 7 — Keep `app.rs` as the orchestrator**:
- After extraction, `app.rs` should contain only:
  - `ClipApp` struct definition
  - `ClipApp::new` constructor
  - `impl eframe::App for ClipApp` (the `update` method)
  - `dispatch` method
  - `dismiss_to_idle` / `dismiss_history_to_idle` (or delegate these)
  - `capture_previous_app` / `snapshot_clipboard` / `copy_to_clipboard`
  - `install_glossary_reload` / `attach_tray` / `set_glossary_malformed` / `with_initial_state` / `is_setup_wizard`

### Acceptance criteria

- `cargo test` passes with all existing tests
- `cargo build` succeeds
- Each extracted module is under 500 lines
- `app.rs` itself is under 800 lines
- No `pub(crate)` leakage — keep visibility minimal within each module

### Files to create/modify

- Create: `src/app/pure.rs`, `src/app/translation.rs`, `src/app/history.rs`,
  `src/app/setup.rs`, `src/app/prompt.rs`, `src/app/channels.rs`, `src/app/tray.rs`
- Modify: `src/app.rs`, `src/lib.rs` (add `mod` declarations)
- Existing tests in `app.rs`: move to corresponding module files

### Notes

- The `#[cfg(test)] mod tests` block at the bottom of `app.rs` tests pure
  functions (`decide_intent`, `requires_size_confirm`, etc.) — these move
  cleanly to `pure.rs`.
- The `Intent` enum and its constructors have no `ClipApp` dependency, so
  they move first.
- Be careful with `use` statements: many current imports in `app.rs` will
  need to move to the submodules.

---

## 2. Unify provider construction in the setup wizard's sample-translation check

**Priority**: Medium  
**Risk**: Low  
**Estimated effort**: 1 session

### Problem

`app.rs::spawn_sample_translation_check` bypasses `factory::build_provider` and
manually constructs providers by matching on `provider_kind.as_str()`. This
duplicates the factory logic and will silently miss new provider types added
to the factory.

The blocker was that the wizard wants each provider's *default* base URL
(e.g., `https://api.anthropic.com/v1` for Anthropic) rather than
`cfg.provider.base_url` (which may not be persisted yet). The code comment
acknowledges this and suggests an `Option<&str>` base-URL override.

### Approach

1. Add an optional `base_url_override: Option<&str>` parameter to
   `factory::build_provider`:
   ```rust
   pub fn build_provider(
       cfg: &Config,
       key: Zeroizing<String>,
       base_url_override: Option<&str>,
   ) -> Result<Arc<dyn LlmProvider>, TranslateError>
   ```
2. When `base_url_override` is `Some(url)`, use that instead of `cfg.provider.base_url`.
3. Update all existing call sites to pass `None`:
   - `main.rs` (startup provider)
   - `lib.rs::run` (CLI mode)
   - `app.rs::persist_setup_completion` (live rebuild after wizard)
4. Rewrite `spawn_sample_translation_check` to use `factory::build_provider`
   with `Some(default_base_url(&provider_kind))`.
5. Remove the manual `match provider_kind.as_str()` block inside
   `spawn_sample_translation_check`.

### Acceptance criteria

- `cargo test` passes, including `factory::tests` and kittest tests that
  exercise the wizard path
- `grep -n "match provider_kind" src/app.rs` returns zero results for the
  sample-translation-check block
- New `build_provider(_, _, Some("https://example.com"))` uses the override URL

### Files to modify

- `src/llm/factory.rs`: add parameter, update logic
- `src/main.rs`: pass `None`
- `src/lib.rs`: pass `None` in `run()`
- `src/app.rs`: pass `None` in `persist_setup_completion`; rewrite
  `spawn_sample_translation_check`

---

## 3. SQLite connection mutex hardening

**Priority**: Medium  
**Risk**: Low  
**Estimated effort**: 1 session

### Problem

`History` wraps its `rusqlite::Connection` in `std::sync::Mutex`. If a panic
occurs while the lock is held, the mutex is poisoned and all subsequent
`.expect("history mutex poisoned")` calls panic — crashing the app. In
practice this is unlikely (the locked sections are short and don't panic),
but it's a sharp edge.

Additionally, `std::sync::Mutex` is held across synchronous I/O on a tokio
runtime. While the hold durations are brief (<1ms typical), it's not
idiomatic in an async codebase.

### Approach

Replace `std::sync::Mutex<Connection>` with `std::sync::Mutex<Connection>`
but wrap in a helper that catches poison:

```rust
fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> Result<T, TranslateError>)
    -> Result<T, TranslateError>
{
    match self.conn.lock() {
        Ok(guard) => f(&guard),
        Err(poisoned) => {
            tracing::error!("history mutex poisoned; attempting recovery");
            // The inner Connection is still valid; use the poisoned guard.
            f(&poisoned.into_inner())
        }
    }
}
```

Then replace all direct `.conn.lock().expect(...)` calls with `self.with_conn(|conn| ...)`.

Alternatively, if you prefer a simpler fix: just change `.expect("history mutex poisoned")`
to `.unwrap_or_else(|e| e.into_inner())` at each lock site. That's less
ceremonial but requires touching each call site.

### Acceptance criteria

- `cargo test` passes (history store tests exercise locking heavily)
- A simulated poison (in a unit test) is recovered rather than panicked
- All `lock().expect(...)` calls in `store.rs` are replaced with the
  poison-safe helper

### Files to modify

- `src/history/store.rs`: add `with_conn` helper, update all lock sites
  (`insert`, `insert_with_cap`, `query`, `delete`, `clear_all`, `count`)

---

## 4. Notification initialization resilience

**Priority**: Low  
**Risk**: Low  
**Estimated effort**: 1 session

### Problem

`ensure_notification_application()` in `notify.rs` uses `OnceLock` — if the
first call to `configure_notifications()` fails, that error is permanently
cached. All subsequent notification attempts silently no-op. The user gets
no feedback that notifications are broken.

### Approach

Option A (simpler): Log a prominent warning when the cached result is an
error, once per app session:

```rust
static NOTIFICATION_WARNED: AtomicBool = AtomicBool::new(false);

fn ensure_notification_application() -> Result<(), TranslateError> {
    let result = NOTIFICATION_APPLICATION_RESULT.get_or_init(|| {
        crate::platform::current()
            .configure_notifications()
            .map_err(|e| e.to_string())
    });
    match result {
        Ok(()) => Ok(()),
        Err(e) => {
            if !NOTIFICATION_WARNED.swap(true, Ordering::Relaxed) {
                tracing::warn!(error = %e, "notifications unavailable for this session");
            }
            Err(notification_error(e.clone()))
        }
    }
}
```

Option B (more robust): Don't cache the error at all — retry on each
notification attempt. This trades a platform call per notification for
recovery from transient failures. Since `configure_notifications` typically
just calls into `notify-rust`, the cost is negligible.

### Acceptance criteria

- On a system where notifications fail, the warn log appears exactly once
  per session
- Notification attempts after the first failure still attempt to deliver
  (or at minimum log why they can't)
- `cargo test` passes

### Files to modify

- `src/notify.rs`: `ensure_notification_application`, add `NOTIFICATION_WARNED`

---

## 5. Glossary RwLock poisoning recovery

**Priority**: Low  
**Risk**: Low  
**Estimated effort**: 1 session

### Problem

The glossary uses `std::sync::RwLock<Glossary>`. Every read site calls
`.read().expect("glossary RwLock poisoned")`. If a panic occurs while holding
a write lock (only happens during SIGHUP reload, which is a simple file read
→ assign — extremely unlikely to panic), the lock is poisoned and every
subsequent translation crashes with a panic.

### Approach

Add a helper that recovers from poison:

```rust
// In glossary.rs or a shared utility
impl Glossary {
    pub fn read_shared(inner: &std::sync::RwLock<Glossary>) -> std::sync::RwLockReadGuard<Glossary> {
        match inner.read() {
            Ok(guard) => guard,
            Err(poisoned) => {
                tracing::error!("glossary RwLock poisoned; recovering with possibly-stale data");
                poisoned.into_inner()
            }
        }
    }
}
```

Then replace all `.read().expect(...)` calls with `Glossary::read_shared(&self.glossary)`.

For the write site (SIGHUP reload), do the same with `.write()`.

### Acceptance criteria

- `cargo test` passes
- A `#[test]` that intentionally poisons the lock and recovers
- No bare `.expect("glossary RwLock poisoned")` in the codebase

### Files to modify

- `src/glossary.rs`: add `read_shared` / `write_shared` helpers
- `src/app.rs`: replace direct `.read()` calls
- `src/translator.rs` (tests): replace in test helper if needed

---

## 6. Add rate limiting on translation dispatch

**Priority**: Medium  
**Risk**: Low  
**Estimated effort**: 1 session

### Problem

`start_translation` fires a network request with no debounce. While the
`AppState::Translating` state gate prevents concurrent translations, a user
with a fast provider could still fire one translation per few hundred
milliseconds by alternating between hotkey presses and dismissals. There's
no explicit rate limit.

### Approach

Add a cooldown to `dispatch_translate`:

1. Add `last_translation_at: Option<Instant>` to `ClipApp`.
2. At the top of `dispatch_translate` (or `start_translation`), check:
   ```rust
   const MIN_TRANSLATION_INTERVAL: Duration = Duration::from_millis(500);
   if let Some(last) = self.last_translation_at {
       if last.elapsed() < MIN_TRANSLATION_INTERVAL {
           // Drop silently or show a brief toast
           return;
       }
   }
   self.last_translation_at = Some(Instant::now());
   ```

### Acceptance criteria

- `cargo test` passes
- Rapid repeated hotkey presses within 500ms only trigger one translation
- Normal usage (pressing keys at human speed) is unaffected

### Files to modify

- `src/app.rs`: add `last_translation_at` field, add guard in
  `dispatch_translate`/`start_translation`

---

## 7. Validate templates at load time

**Priority**: Low  
**Risk**: Low  
**Estimated effort**: 1 session

### Problem

`Templates::load` checks that override files exist but doesn't compile the
Jinja2 templates. A syntax error in a custom `templates/translate.j2` only
surfaces when the user tries to translate — giving a poor error experience.

### Approach

After loading each override template, call `minijinja::Environment::new()
.add_template_owned(...)` with a synthetic context to validate compilation:

```rust
fn validate_template(name: &str, source: &str) -> Result<(), TranslateError> {
    let mut env = minijinja::Environment::new();
    env.add_template_owned(name, source)
        .map_err(|e| TranslateError::Template(format!(
            "template '{name}' failed to compile: {e}"
        )))?;
    // Optionally also render with a dummy context to catch runtime errors
    let tmpl = env.get_template(name).unwrap();
    let ctx = minijinja::value::Value::from_serializable(
        &TemplateContext::for_translate("Test", "")
    ).map_err(|e| TranslateError::Template(format!("template render test: {e}")))?;
    tmpl.render(&ctx).map_err(|e| TranslateError::Template(format!(
        "template '{name}' render test failed: {e}"
    )))?;
    Ok(())
}
```

Call this for each loaded template override in `Templates::load`. If a
template fails validation, return `Err` (the caller already aborts startup
on template errors — see `main.rs` "strict load" comment).

### Acceptance criteria

- `cargo test` passes
- Adding a syntactically invalid override file causes a startup error
  (not a mid-translation error)
- The error message includes the template name and the minijinja error

### Files to modify

- `src/llm/templates.rs`: add `validate_template`, call in `load`

---

## 8. Clean up repo root artifacts

**Priority**: Trivial  
**Risk**: None  
**Estimated effort**: 10 minutes

### Problem

Two large files in the repo root that don't belong there:
- `clipboard-translator-spec.md.pdf` (281 KB) — spec document
- `clipt9n-handoff.zip` (547 KB) — likely a handoff artifact

### Approach

1. Move `clipboard-translator-spec.md.pdf` → `docs/spec.pdf`
2. Add `clipt9n-handoff.zip` to `.gitignore` and delete it
3. Optionally add `*.zip` to `.gitignore`

### Acceptance criteria

- `ls *.pdf *.zip` in repo root returns nothing
- `docs/spec.pdf` exists
- `.gitignore` contains `clipt9n-handoff.zip`

### Files to modify

- `.gitignore`
- Move: `clipboard-translator-spec.md.pdf` → `docs/spec.pdf`
- Delete: `clipt9n-handoff.zip`

---

## 9. Add integration tests for selected-text capture path

**Priority**: Medium  
**Risk**: Low (test-only)  
**Estimated effort**: 1 session

### Problem

`snapshot_selected_text` in `app.rs` does real clipboard manipulation with
timing-sensitive behavior (save clipboard → simulate Cmd+C → sleep {delay} →
read selection → restore clipboard). This is the most fragile code path but
has no dedicated tests.

### Approach

Existing kittest infrastructure is already in place (`tests/kittest_*.rs`).
The `snapshot_selected_text` path is hard to test end-to-end (it calls
`platform.copy_selection_to_clipboard()` which sends real keystrokes), but
the core logic can be tested:

1. Unit test `selected_text_after_copy` more thoroughly (already has basic
   tests in `app.rs`)
2. Add a kittest that:
   - Sets up the app with config including `[hotkey.selection]`
   - Simulates a hotkey event for the selection hotkey ID
   - Verifies the prompt window appears
3. Test the clipboard-restore logic: when `before` is non-empty, it must
   be restored after capture. Test via `MockClipboard`.

### Acceptance criteria

- `cargo test` passes
- At least one new test in `tests/kittest_smoke.rs` or a new
  `tests/kittest_selection.rs` that exercises the selection hotkey path
- `selected_text_after_copy` tests cover all branches (same text + change
  count, different text, empty after, changed count flag)

### Files to modify

- `tests/kittest_smoke.rs` or new `tests/kittest_selection.rs`
- `src/app.rs` tests block (expand `selected_text_after_copy` tests)

---

## Order of work

Recommended order for a new session:

1. **Session 1**: Item 8 (repo cleanup, 10 min warmup) → Item 2 (unify provider construction) → Item 6 (rate limiting)
2. **Session 2**: Item 1 Step 1 (extract pure helpers) → Step 2 (extract translation worker)
3. **Session 3**: Item 1 Step 3 (extract history viewer) → Step 4 (extract setup wizard)
4. **Session 4**: Item 1 Step 5 (extract prompt) → Step 6 (extract channels/tray) → Step 7 (trim app.rs)
5. **Session 5**: Item 3 (mutex hardening) → Item 5 (RwLock recovery) → Item 7 (template validation)
6. **Session 6**: Item 4 (notification resilience) → Item 9 (selection capture tests)

Items 3, 4, 5, and 7 are independent and can be done in any order.
Item 1 is the largest and should be done incrementally to avoid merge
conflicts with other work.
