# Code-quality improvement tasks / コード品質改善タスク

A grounded, reviewed backlog of independent improvement tasks for this repository. Each task is designed to be picked up by a single implementer and shipped as **one task = one PR**. File names, line numbers and counts below reflect the state of the repository at the time of writing; re-confirm them before editing.

実装者が1人で着手し、**1タスク=1PR**で出せるよう設計した改善タスク一覧です。以下のファイル名・行番号・件数は作成時点の状態です。着手前に再確認してください。

## Scope and ground rules / 範囲と共通ルール

These tasks are review-approved and must honor the existing project rules without exception:

- Keep the device/build boundaries from [CONTRIBUTING.md](../../CONTRIBUTING.md) and [repository layout](repository-layout.md): measurement/analysis in Rust (`crates/`), SDK calls in `bridge/`, remote controller services in `servers/`, GUI automation in `bindings/`.
- Tests and CI must never connect instruments, home/move axes, open beams, or change drivers.
- Preserve scientific definitions, thresholds, stored raw data and historical evidence. Do not alter the saved-data contract in [measurement reference](../reference/measurement.md) or the [README saved-data table](../../README.md#saved-data--保存仕様).
- Do not commit real runs, tokens, private profiles or vendor DLLs. Keep upstream notices intact.
- Do not rewrite git history. Work on a fresh branch off `main` and open a PR into `main`.
- Respect the pinned CI action SHAs and `--locked` builds already in place.

共通ルール: CONTRIBUTING とリポジトリ構成の境界、実機非接続、科学的定義・保存仕様・履歴の保持、履歴の非改変を厳守します。

## Recommended order / 推奨着手順

Lowest risk and highest leverage first:

1. **Task 1 — Supply-chain audit in CI** (low risk, high value)
2. **Task 2 — Linux build/test coverage decision** (low risk)
3. **Task 3 — Split `backend.rs` by responsibility** (behavior-preserving move)
4. **Task 4 — Group `PleApp` GUI-layout state** (incremental)
5. **Task 5 — Introduce a run-context struct in `engine.rs`** (argument grouping only)
6. **Task 6 — Unify error types inside `ple-core`** (internal-only first)

Tasks 1 and 2 are independent of each other and of 3–6. Tasks 3–6 touch Rust source and should land in order to minimize rebase conflicts, but each remains a standalone PR.

---

## Task 1 — Add supply-chain auditing to CI

**Labels:** `github_actions`, `dependencies`, `maintenance`

### Background / 背景
There is no vulnerability/advisory gate in CI. Searching `.github/` for `audit`, `cargo-deny`, `tarpaulin`, `llvm-cov`, `coverage`, `codecov` returns zero matches. The desktop crate pulls many third-party dependencies (`crates/ple-desktop/Cargo.toml`: `eframe 0.34.3`, `egui_plot 0.35`, `read-fonts 0.38`, `getrandom 0.4`, `image 0.25`), and Dependabot opens update PRs weekly (`.github/dependabot.yml`), but nothing checks for known RUSTSEC advisories or disallowed/duplicate dependencies. Distribution packages are shipped unsigned on Windows and ad-hoc signed on macOS (see [README Downloads](../../README.md#downloads--ダウンロード)), which raises the value of a machine-checked advisory gate.

### Files and scope / 変更対象と範囲
- `.github/workflows/test.yml`: add one job (or step) that runs `cargo audit` (or `cargo-deny check advisories bans`) with a pinned action/tool version.
- Optionally add a `deny.toml` at the repo root if `cargo-deny` is chosen.
- Do not change existing jobs, the OS matrix, or the license test (`tests/test_collect_licenses.py`); the new gate complements, not replaces, the existing license check.

### Acceptance criteria / 受け入れ基準
- CI runs the advisory/supply-chain check on `pull_request` and `push` to `main`.
- The check fails the build when a known advisory is present and passes on the current dependency set (`cargo audit`/`cargo-deny` exits 0 on `main` today).
- Any new tool or action is pinned (version tag or commit SHA), consistent with existing pins in `test.yml`.
- `cargo test --workspace --locked`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, and `python scripts/check_docs.py` still pass.

### Guardrails / ガードレール
- Keep the immutable-license test (`tests.test_collect_licenses`) and `collect_licenses.py` behavior unchanged.
- Do not add network-dependent steps that would block device-free CI if an advisory DB is unreachable; prefer a resilient, pinned setup.
- No history rewrite; single branch off `main`, single PR.

---

## Task 2 — Decide and encode Linux build/test coverage

**Labels:** `github_actions`, `rust`, `maintenance`

### Background / 背景
The `test` and `release` verification matrices run only `windows-2025` and `macos-15` (`.github/workflows/test.yml` matrix; `.github/workflows/release.yml` verify matrix). However the desktop crate enables Linux display backends — `crates/ple-desktop/Cargo.toml` sets `eframe` features `["glow","default_fonts","x11","wayland"]` — so Linux-only code paths are never compiled or tested in CI. `release.yml` already uses `ubuntu-24.04` for the `source` and `publish` jobs, so a Linux runner is available.

The product is distributed for Windows x64 and macOS arm64 ([README Downloads](../../README.md#downloads--ダウンロード)), so this is a deliberate decision, not an automatic "add Linux". The implementer must pick one and document the rationale in the PR:

- **Option A:** Add a `ubuntu-24.04` entry to the `test.yml` matrix that at minimum runs `cargo build`/`cargo test` with the required system libraries for x11/wayland.
- **Option B:** If Linux is intentionally unsupported, drop or gate the `x11`/`wayland` eframe features so the enabled feature set matches the supported/tested platforms, and note the non-support.

### Files and scope / 変更対象と範囲
- `.github/workflows/test.yml` (matrix / new job), and possibly `.github/workflows/release.yml`.
- `crates/ple-desktop/Cargo.toml` only if Option B is chosen.
- Documentation note in the PR body; no change to the saved-data or device contracts.

### Acceptance criteria / 受け入れ基準
- Either (A) CI compiles and tests the crate on Linux and the job is green, or (B) the enabled eframe feature set no longer claims Linux backends and the PR explains the non-support decision.
- No regression on the existing Windows/macOS jobs.
- `cargo build --locked --bin kohdalab-ple` still succeeds on the supported platforms.

### Guardrails / ガードレール
- No instrument access; Linux CI stays device-free like the rest of the matrix.
- Do not weaken or remove the existing Windows installer / macOS signing verification steps.
- Keep action pins and `--locked`. Single branch off `main`, single PR.

---

## Task 3 — Split `backend.rs` by responsibility (behavior-preserving)

**Labels:** `rust`, `maintenance`

### Background / 背景
`crates/ple-desktop/src/backend.rs` is 1,985 lines and mixes three distinct responsibilities:
1. Python subprocess supervision — `RpcClient`/`RpcInner` (lines ~24–41) and `Drop for RpcInner` that kills the child (~32–40).
2. Newline-delimited JSON-RPC framing/parsing — `read_bounded_line` (~288), `log_bridge_stderr` (~256).
3. Three separate device clients — Spectrometer (~305), Stage/StageClient (~580/920), Power/PowerClient (~1085/1261), each with its own command enum (`StageCommand` ~572, `PowerCommand` ~1076) and worker thread.

It is the heaviest user of `Mutex` (~20 occurrences) and `json!` (~44), and has seen 7 commits of churn. This is the clearest candidate for module separation.

### Files and scope / 変更対象と範囲
- Split `crates/ple-desktop/src/backend.rs` into a `backend/` module (e.g. `backend/mod.rs`, `backend/rpc.rs`, `backend/spectrometer.rs`, `backend/stage.rs`, `backend/power.rs`).
- Update `mod backend;` wiring and any `use` paths in `main.rs`, `engine.rs`, `api_ui.rs`, `ui.rs` as needed.
- This is a **pure move/re-export refactor**: no behavioral changes to process supervision, RPC framing, Stop handling, or device commands.

### Acceptance criteria / 受け入れ基準
- Public items used by other modules keep identical paths or are re-exported so no call site changes behavior.
- The existing `#[cfg(test)]` tests from `backend.rs` (14 `#[test]`) move with their code and all pass unchanged.
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, and `cargo test --workspace --locked` pass.
- `git diff` shows moves/splits only — no change to control flow, timeouts, Stop semantics, or JSON wire shapes.

### Guardrails / ガードレール
- Preserve explicit connect/motion/Stop behavior exactly (see [ROADMAP](../../ROADMAP.md)); do not alter `Drop` cleanup or child-kill logic.
- Keep the `bridge/` protocol contract intact; this task does not touch `bridge/`.
- No `unsafe` (crate is `#![forbid(unsafe_code)]`). No history rewrite; single branch off `main`, single PR.

---

## Task 4 — Group `PleApp` GUI-layout state into a sub-struct

**Labels:** `rust`, `maintenance`

### Background / 背景
`PleApp` is a god object: the struct is defined in `crates/ple-desktop/src/main.rs` (lines ~54–118, ~65 fields) and its methods are spread across four `impl PleApp` blocks (`ui.rs` ~63–1516, `api_ui.rs` ~53–917, `presentation.rs` ~358–838, plus `main.rs`). Fields mix unrelated concerns: hardware/state (`controller`, `config`, `samples`) sit next to GUI-layout flags (`left_collapsed`, `right_was_maximized`, `device_expanded`, `rails_initialized`, `screenshot_requested`). `main.rs` has the highest churn (16 commits), concentrated on this struct.

### Files and scope / 変更対象と範囲
- `crates/ple-desktop/src/main.rs`: introduce a sub-struct (e.g. `struct UiLayout { ... }`) holding the GUI-layout-only flags, and reference it from `PleApp`.
- Update field accesses in `ui.rs`, `presentation.rs`, `api_ui.rs`, `main.rs`.
- Scope is deliberately narrow: start with the clearly GUI-layout fields listed above. Do **not** refactor hardware/measurement state in this task (YAGNI).

### Acceptance criteria / 受け入れ基準
- GUI-layout flags are grouped under one field; no behavioral change to layout or the `eframe::App::update` loop.
- All existing GUI-crate tests (e.g. `ui.rs` 18, `presentation.rs` 6, `main.rs` 9 `#[test]`) pass unchanged.
- `cargo fmt`, `cargo clippy ... -D warnings`, `cargo test --workspace --locked` pass.
- A manual Demo launch renders identically (collapsible rails, maximize, device panel, screenshot) — note this in the PR validation section.

### Guardrails / ガードレール
- Behavior-preserving only; do not change default layout values or persisted config fields (`~/.kohdalab/ple/config/config.json`) or their serde names.
- Keep the saved-data and settings contracts untouched. No `unsafe`.
- No history rewrite; single branch off `main`, single PR.

---

## Task 5 — Introduce a run-context struct in `engine.rs`

**Labels:** `rust`, `maintenance`

### Background / 背景
`crates/ple-desktop/src/engine.rs` carries three `#[allow(clippy::too_many_arguments)]` suppressions (lines ~621, ~675, ~877):
- `execute` (~622) and `execute_inner` (~676): 9 parameters each — `&Config, &Plan, &mut Spectrometer, &StageClient, &PowerClient, &AtomicBool (stop), &mpsc::Receiver<Command>, &mpsc::Sender<Event>, &mut Recording`.
- `acquire_point` (~878): 13 parameters.

The device trio (`spec`/`stage`/`power`), control-plane channels (`stop`/`commands`/`events`) and `recording` are threaded positionally and repeat in the sibling `move_point` (~844, 7 params). Grouping the stable context into one struct would let the suppressions be removed.

### Files and scope / 変更対象と範囲
- `crates/ple-desktop/src/engine.rs` only.
- Introduce a context struct (e.g. `RunContext`) holding the stable device handles + channels + recording; keep genuinely per-call values (`index`, `repeat`, `target`, `actual`, `bg`) as explicit arguments.
- Remove the three `#[allow(clippy::too_many_arguments)]` attributes once the argument counts drop below the clippy threshold.

### Acceptance criteria / 受け入れ基準
- The three `#[allow(clippy::too_many_arguments)]` attributes are removed and `cargo clippy --workspace --all-targets --locked -- -D warnings` passes without them.
- The acquisition sequence (background, first-point review, point × repeat loops), event ordering, Stop handling and recorded outputs are unchanged.
- The 10 `#[test]` functions in `engine.rs` pass unchanged; `cargo test --workspace --locked` passes.

### Guardrails / ガードレール
- Argument grouping only: do not change acquisition order, thresholds, power-sampling timing (power is sampled before exposure, not averaged — see [README saved-data](../../README.md#saved-data--保存仕様)), or what is written to disk.
- Do not change the `Event`/`Command` enum variants or their payloads (API/`bindings` contract).
- No history rewrite; single branch off `main`, single PR.

---

## Task 6 — Unify error types inside `ple-core` (internal-only first)

**Labels:** `rust`, `maintenance`, `compatibility`

### Background / 背景
There is no unified error type; `thiserror`/`anyhow` are not used. Three conventions coexist:
- `engine::RunError` (`crates/ple-desktop/src/engine.rs` ~53–79) — implements `Display` but not `std::error::Error`.
- `api::Error` (`crates/ple-desktop/src/api.rs` ~9–25) — a `{code, message}` struct with its own `Result` alias.
- Pervasive `Result<_, String>`: `backend.rs` ~33, `recording.rs` ~17, `ple-core/lib.rs` ~13, plus `ple-core` public API (`Config::plan`, `subtract_background`, `integrate_nm`, `validate`) all returning `Result<_, String>`. Events also carry stringly errors (`SpectrometerConfigured(Result<Value, String>)`, `Cleanup { result: Result<(), String> }`).

Because public-API and `Event` signatures are a compatibility contract with `bindings`/`api_ui`, this task is **scoped to `ple-core` internals only** in its first pass.

### Files and scope / 変更対象と範囲
- `crates/ple-core/src/lib.rs`, `recording.rs`, `display.rs`.
- Introduce a `ple-core` error enum implementing `std::error::Error + Display` (hand-rolled or via a small, pinned, already-vendored-friendly crate subject to the license-notice rules).
- Keep the **public** function signatures that cross crate boundaries stable, or provide `From`/`Display` so existing `Result<_, String>` call sites in the desktop crate compile unchanged. Converting the desktop crate and the `Event` enum is explicitly **out of scope** for this task.

### Acceptance criteria / 受け入れ基準
- `ple-core` has a dedicated error type used internally; `std::error::Error` is implemented.
- No change to the saved-data formats, numeric round-trip guarantees, or the `ple-core` public behavior; the 35 `ple-core` `#[test]` functions pass unchanged.
- `cargo test --workspace --locked`, `cargo clippy ... -D warnings`, `cargo fmt --all -- --check` pass across the whole workspace (the desktop crate must still build).
- If any new dependency is added, its license notice is handled per `scripts/collect_licenses.py` and `tests.test_collect_licenses` stays green.

### Guardrails / ガードレール
- Do not change scientific definitions, thresholds, or the exact-comparison/ULP tolerance semantics described in [measurement reference](../reference/measurement.md).
- Do not alter the `Event`/`Command`/API `Result` contracts consumed by `bindings` and `api_ui` in this task.
- Keep upstream notices intact; no vendor DLLs or real data. No history rewrite; single branch off `main`, single PR.

---

## One PR per task / 1タスク1PR

Each task above is a standalone PR into `main`, following [CONTRIBUTING.md](../../CONTRIBUTING.md) and the [pull request template](../../.github/pull_request_template.md): a concrete summary, the validation commands run (distinguishing simulated vs. device-free vs. runtime evidence), and a compatibility/safety note. Reference the matching issue from the PR.

各タスクは `main` 向けの独立したPRとし、PRテンプレートに従って概要・実行した検証・互換性/安全性の注記を記載し、対応するissueを参照します。
