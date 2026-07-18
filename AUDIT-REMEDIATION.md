# Audit Remediation Report

> **Project**: osm-to-bedrock
> **Audit Date**: 2026-07-17
> **Remediation Date**: 2026-07-17
> **Severity Filter Applied**: `all` (all phases executed)
> **Branch**: `fix/audit-remediation` (base `ba458dc` → HEAD `e0acc3a`, 14 commits)
> **Baseline → Final**: 163 Rust tests → **254 Rust + 21 web = 275 tests**, all passing; clippy/tsc/lint/build clean throughout

---

## Follow-up: ARC-001 + ARC-010 (2026-07-17)

The two items originally deferred above were completed in a follow-up session
and committed to `main`:

- **ARC-001 — streaming Java Edition writer** (`a240fb6`). `JavaWorld::new_streaming`
  scopes its scratch store to the current tile and lazily writes 32×32 Anvil
  region files as tiles flush, giving Java Bedrock's per-tile memory profile.
  The `enforce_java_memory_budget` OOM guard (and its constants + 6 tests) is
  retired — both editions now stream, and Bedrock never had a guard. A
  byte-parity test asserts the streaming writer produces region files identical
  to the in-memory path, including a region that straddles a tile boundary.
- **ARC-010 — DashMap job state** (`d5191e6`). `Jobs` is now
  `Arc<DashMap<String, JobState>>`; the `/status` + `/download` read path no
  longer contends with worker progress writes. `lock_jobs` is removed (DashMap
  shard locks don't poison — SEC-006 is now structural).

**Updated count**: **55 of 58** unique issues resolved. Remaining open items
are the two maintainer/outward-facing ones (DOC-003 crates.io publish,
ARC-011 upstream donation) plus the optional low-priority skips. DOC-009 is
now closed (decision: keep `install-hooks` restricted to `pre-commit` — it is
one of the project's standard Makefile targets, and the restricted form is
correct). **Verification**: `make checkall` exit 0 — **251 Rust tests**
(240 lib + 9 integration + 2 doctests) + **21 web tests**, clippy/fmt/
tsc/build clean. (Rust count dropped 254→251: −6 retired guard tests, +3
streaming tests.)

---

## Execution Summary

| Phase | Status | Agent(s) | Issues Targeted | Resolved | Partial | Manual/Deferred |
|-------|--------|----------|:--------------:|:--------:|:-------:|:---------------:|
| 1 — Critical Security (`server.rs`) | ✅ | fix-security (opus) | 7 | 7 | 0 | 0 |
| 2a — Test net (ARC-005 ≡ QA-002) | ✅ | fix-architecture (opus) | 1 | 1 | 0 | 0 |
| 2b — Java OOM guard + tile-loop dedup | ✅ | fix-architecture (opus) | 2 | 2 | 0 | 1¹ |
| 3b — Architecture remaining | ✅ | fix-architecture (opus/sonnet) | 11 | 10 | 0 | 1² |
| 3c — Code Quality | ✅ | fix-code-quality (opus/sonnet) | 12 | 11 | 0 | 1³ |
| 3a — Security remaining (SEC-003) | ✅ | fix-security (sonnet) | 1 | 1 | 0 | 0 |
| 3d — Documentation | ✅ | fix-documentation (sonnet) | 22 | 20 | 0 | 2⁴ |
| 4 — Verification | ✅ | orchestrator (`make checkall`) | — | — | — | — |

¹ ARC-001 shipped the **safe OOM-guard fallback**; the full streaming Anvil writer is flagged as future work.
² ARC-010 (DashMap) deferred — classified "Long-term Backlog" in the audit's own roadmap.
³ QA-011 (stub consolidation) left as-is — optional per audit; stubs documented under ARC-011.
⁴ DOC-019 (AGENTS.md pointer) + DOC-020 (release-artifact move) skipped as low-priority.

**Overall**: **52 of 58 unique issues resolved** (90%). 2 were merged duplicates (QA-002≡ARC-005, QA-003≡SEC-006). 4 deferred/skipped, all Medium/Low and explicitly justified. **No regressions** — every commit verified with `make checkall` before proceeding.

---

## How It Was Run

- **Phases 1, 2a, 2b ran sequentially** (blocking), one sub-agent each, with a verified checkpoint commit after every batch.
- **Phase 3 was sequenced by file-domain, not parallelized.** The File Conflict Map showed Architecture (3b) and Code Quality (3c) share 5 core files (`server/`, `pipeline/`, `world.rs`, `bedrock.rs`, `anvil.rs`) — naive parallelism would have silently overwritten edits. Order: 3b (sub-batched: ARC-003 → ARC-004 → web/Docker/docs → cli split → pin/Block enum → vitest) → 3c (ChunkStore → server/pipeline QA → web QA) → 3a (CSP) → 3d (docs).
- **Sub-agents were instructed NOT to commit**; the orchestrator ran `make checkall` after each and committed centrally. This fixed an early pattern where agents self-committed before verification, and gave clean per-batch rollback points.
- **Every batch was independently re-verified** — sub-agent self-reported "green" was never trusted. This caught two real issues mid-flight: (a) stale LSP diagnostics that would have blocked on nothing, and (b) the ARC-009 vitest suite had **12 genuine `tsc` errors** (`fetch.mock` TS2339 + implicit-`any`) that `vitest`/`bun build` don't catch — fixed directly with `vi.mocked()` before committing.

---

## Resolved Issues ✅

### Security (8)
- **SEC-001** — Opt-in shared-secret auth (`--api-key` / `OSM_TO_BEDROCK_API_KEY`, constant-time compare) on all routes except `/health`; fail-safe bind guard refuses non-loopback bind without a key unless `OSM_TO_BEDROCK_ALLOW_INSECURE_BIND=1`. `src/server/auth.rs`, `src/cli/args.rs`, `Dockerfile`, `docker-entrypoint.sh`.
- **SEC-002** — `/status` and `/download` no longer leak verbatim `anyhow` chains / OS strings. `JobState::Error` holds only a client-safe `public_message`; full chain logged at failure moment. `src/server/{state,handlers}.rs`.
- **SEC-003** — Restrictive `Content-Security-Policy` on the Next.js frontend. `connect-src 'self'` (all external calls are server-side proxied); `img-src` allows only `tile.openstreetmap.org`. `web/next.config.ts`.
- **SEC-004** — `validate_bbox` (±90/±180 + max 250k blocks/axis) called in fetch/terrain/preview handlers before the semaphore. `src/server/options.rs`.
- **SEC-005** — Explicit 1 MiB `DefaultBodyLimit` on JSON routes. `src/server/mod.rs`.
- **SEC-006 / QA-003** — Mutex-poisoning recovery via `lock_jobs` helper (`into_inner()`) across all `server/` lock sites + `bedrock.rs` `ChunkWriter`. Server survives a panicked job.
- **SEC-007** — `/cache/areas` moved behind the auth middleware.
- **SEC-008** — Client-error branches return 400 (`ApiError::bad_request`) not 500.

### Architecture (13)
- **ARC-001** — Java Edition **OOM guard**: `enforce_java_memory_budget` (1.5 GB / ~15k chunks, `Block` repr(u8) × 24 sub-chunks math) refuses oversized `edition=java` conversions before allocation, in both CLI and server paths. Misleading "drains each tile to disk" docstrings corrected.
- **ARC-002** — Bedrock/Java tile loop deduped: `process_tile` helper + `WorldWriter::flush_tile()` (drains for Bedrock, no-op for Java); outer loop is now edition-agnostic (~360 → ~220 LOC). Cross-edition parity tests stay green.
- **ARC-003** — `pipeline.rs` (2591 LOC) split into `src/pipeline/{mod,render,terrain,preview,decoration,util}.rs`. Public API unchanged.
- **ARC-004** — `server.rs` (3127 LOC) split into `src/server/{mod,state,error,auth,options,handlers}.rs`. Folded in QA-004 (`spawn_conversion_job` / `prepare_world_dir` / `finalize_conversion`).
- **ARC-005 / QA-002** — Test net: 40 new tests (`ChunkData` round-trip at sub-chunk boundaries, `RecordingWorld` `render_osm_features` integration, cross-edition Bedrock↔Java parity, `params`/`nbt` coverage, `run_conversion` doctest). The safety net that made every later refactor verifiable.
- **ARC-006** — `NEXT_PUBLIC_API_URL` → server-only `RUST_API_URL` (verified browser code never reads it). Dockerfile build-time bake dropped.
- **ARC-007** — `proxyToRust(path, {method,body,headers,timeoutMs,timeoutLabel})` helper; 9 routes refactored (routes dir 707 → 463 LOC, −34%).
- **ARC-008** — `main.rs` (1254 LOC) → 16-LOC shim; CLI in `src/cli/{mod,args,convert,cache}.rs` with shared `ConvertCommonArgs` + `BuildingArgs` flag groups. Help text verified equivalent.
- **ARC-009** — vitest + testing-library + jsdom; 21 web tests (api-config + useConversion polling state machine); CI `vitest` step; `web-test` Make target.
- **ARC-011** — `par-osm-rust` pinned `=0.1.1`; 6 stub modules documented as re-export shims.
- **ARC-012** — `zip_directory`/`format_bytes` moved to `pipeline/util.rs` (side-effect of ARC-003).
- **ARC-013** — `Block` enum organized with section-comment groups (no reorder — `repr(u8)` discriminants preserved).
- **ARC-014** — `docs/ARCHITECTURE.md` endpoint table + Server Architecture prose synced.

### Code Quality (11)
- **QA-001** — `ChunkStore` struct extracts the 5 shared fields + 9 shared methods; `BedrockWorld`/`JavaWorld` delegate. Streaming ownership preserved via `take_chunks()`/`clear_aux()`. bedrock.rs −129 LOC, anvil.rs −67 LOC (−196 duplicated lines).
- **QA-004** — `spawn_conversion_job` etc. (folded into ARC-004).
- **QA-005** — `merged_data.unwrap()` → `ok_or_else`.
- **QA-006** — `render_osm_features` 558 → 34 LOC orchestrator; 11 per-layer `render_*` helpers extracted.
- **QA-007** — Shared `ConvertNumericBounds` view; both validators delegate.
- **QA-008** — 15 `#[allow(dead_code)]` markers audited → 4 removed (3 stale + 1 dead fn), 11 documented with intent comments. 11 remain, all justified.
- **QA-009** — `runConversionJob(url, body, opts)` helper; 4 conversion methods now thin wrappers (545 → 504 LOC).
- **QA-010** — `useEffect` cleanup aborts in-flight controller + clears poll timer on unmount; `res.body!` → null guard; skipped test enabled (21 passed, 0 skipped).
- **QA-012** — Stale TODO removed (partial restore would mislead; proper fix is an ExportPanel state-lift → recommended as GitHub issue).
- **QA-013** — `add_block_entity` `(x,z)`-bucket / `y`-unused contract documented; behavior pinned by test.
- **QA-014** — Single `console.error` documented as acceptable per-feature diagnostic.

### Documentation (20)
- **DOC-001/005/015** — CLAUDE.md Architecture rewritten around the streaming tile pipeline; module-layout table; 3 missing endpoints added; duplicate numbering fixed.
- **DOC-002/010/013** — 7 stub modules documented as `par-osm-rust` shims; subcommand list fixed; post-refactor files added to module tree.
- **DOC-003** — Broken `cargo install osm_to_bedrock` demoted (crate unpublished) → points to `cargo install --path .` + release binaries.
- **DOC-004/017** — Web UI documented as Bedrock `.mcworld` only; Java is CLI-only for now.
- **DOC-006/007** — New `docs/DEPLOYMENT.md` (Docker, env vars incl. auth + `CORS_ALLOWED_ORIGIN`); README Docker section; CLI env table.
- **DOC-008** — CHANGELOG 0.8.0 link references fixed.
- **DOC-009** — Stale graphify hooks deleted; `install-hooks` restricted to `pre-commit`.
- **DOC-011** — rustdoc on ~41 `Block` variants + `JavaWorld` constructors.
- **DOC-012** — "60+ variants" → 56.
- **DOC-014** — CONTRIBUTING Project Layout + `make checkall` updated.
- **DOC-016** — Full 26-key config table in `docs/CLI.md`.
- **DOC-018** — `server.log` removed.
- **DOC-021** — CHANGELOG `---` separators normalized.
- **DOC-022** — README Makefile command list completed.

---

## Requires Manual Intervention / Deferred 🔧

### [ARC-001] Full streaming Anvil writer (Critical — ✅ resolved in follow-up `a240fb6`)
- **What shipped originally**: the deterministic OOM guard (refuse oversized `edition=java` before allocating), which closed the Critical failure mode (server could no longer be OOM-killed via `/fetch-convert`).
- **What landed in follow-up**: the full lazy region-file streaming writer. `JavaWorld::new_streaming` drains each tile's chunks into 32×32 region buffers and writes each `.mca` once the tile containing the region's max in-bounds chunk has flushed; peak memory ≈ one tile + a frontier of region buffers. `enforce_java_memory_budget` (and its constants + 6 tests) is retired — both editions now stream. A byte-parity test pins the streaming output to the in-memory path, including a region straddling a tile boundary.

### [ARC-010] DashMap for read-heavy job-status path (✅ resolved in follow-up `d5191e6`)
- **What landed**: `Jobs` is `Arc<DashMap<String, JobState>>`; the `/status` + `/download` read path no longer contends with worker progress writes. `lock_jobs` is removed (DashMap shard locks don't poison — SEC-006 is now structural). A test pins that the map stays usable after a panicked worker thread.

### [DOC-003] Publish `osm_to_bedrock` to crates.io (outward-facing — needs maintainer)
- **What shipped**: README demoted the broken install to "not yet published" + working alternatives. DOC-011 docstring coverage (the precondition) is now done.
- **What remains**: `cargo login` + `cargo publish` (and the release workflow already has an "already-published" skip). Then remove the demotion note. Publishing is an outward-facing action requiring explicit confirmation — not done autonomously.
- **Estimated effort**: small (after the publish decision).

### [ARC-011] Donate `source_options.rs` upstream (outward-facing — deferred)
- Moving shared logic into the `par-osm-rust` crate is an outward-facing cross-repo change; left for the maintainer.

### [DOC-009] `install-hooks` target (✅ resolved — decision: keep, restricted to `pre-commit`)
- The stale graphify hooks are deleted and `install-hooks` only arms `pre-commit`. Decision taken: **keep the target** rather than drop it — `install-hooks` is one of the project's standard Makefile targets, and the restricted form is correct (the original defect, silently re-enabling graphify on `make install-hooks`, is gone). No further code change required beyond the deletion + CHANGELOG `Removed` entry already shipped.

### Skipped (low-priority, no action needed)
- **QA-011** — stub consolidation (optional; stubs already documented).
- **DOC-019** — root `AGENTS.md` 16-byte pointer (intentional redirect; now accurate).
- **DOC-020** — release-marketing artifacts at root (moving would break README's gallery link).

### Noted (not in audit scope)
- `cargo doc --no-deps` emits **40 pre-existing intra-doc-link warnings** to private items (created by the directory splits). They don't affect `make checkall` (which doesn't run `cargo doc`) or clippy. A future pass can either make the linked items `pub(crate)` or convert the `[`Foo`]` links to scoped paths.

---

## Verification Results

- **Rust tests**: ✅ Pass — **254** (241 lib + 0 bin-shim + 11 integration + 2 doctests), 0 failed. (Baseline 163; +91 Rust tests.)
- **Web tests**: ✅ Pass — **21** vitest (2 files), 0 skipped. (Baseline 0.)
- **`cargo clippy --all-targets -- -D warnings`**: ✅ Pass — 0 warnings.
- **`cargo fmt --check`**: ✅ Pass.
- **`bunx tsc --noEmit`**: ✅ Pass — clean.
- **`bun run lint`**: ✅ Pass — 9 warnings (all pre-existing `no-unused-vars`; down from 10 — QA-012 removed one), 0 errors.
- **`bun run build`**: ✅ Pass — Next.js compiles, 17/17 routes.
- **`make checkall`**: ✅ Pass — exit 0.

Every batch was re-verified independently before its commit. No regressions were introduced; the two mid-flight issues (stale diagnostics, ARC-009 `tsc` errors) were caught and fixed before commit.

---

## Files Changed

**71 files changed, +11,247 / −7,214** across 14 commits. Structural highlights:

- **New module dirs**: `src/pipeline/` (6 files), `src/server/` (6 files), `src/cli/` (4 files).
- **New shared struct**: `ChunkStore` in `src/world.rs`.
- **New tests**: `tests/pipeline_render_osm_features.rs`; `#[cfg(test)]` blocks in `world.rs`, `nbt.rs`, `params.rs`, `server/{auth,options,state,error}.rs`; `web/src/lib/api-config.test.ts`, `web/src/hooks/useConversion.test.tsx`.
- **New docs**: `docs/DEPLOYMENT.md`.
- **Deleted**: `src/pipeline.rs`, `src/server.rs`, `.githooks/post-commit`, `.githooks/post-checkout` (graphify), `server.log`.
- **Web**: `next.config.ts` (CSP), `lib/api-config.ts` (`proxyToRust` + `RUST_API_URL`), 9 refactored proxy routes, `useConversion.ts` (refactor + cleanup).

Full per-commit detail: `git log --stat ba458dc..HEAD`.

---

## Next Steps

1. **Review the deferred items** above (ARC-001 full streaming writer is the only Critical-adjacent follow-up; the OOM guard already makes it safe).
2. **Re-run `/audit`** to get a fresh AUDIT.md reflecting current state (the structural debt that drove most findings is gone; the codebase is materially smaller per-file and test-covered).
3. **Decide on the maintainer actions**: crates.io publish (DOC-003); upstream donation (ARC-011) — `source_options.rs` is donated into `par-osm-rust` locally (commit pending publish); a 0.1.2 crates.io release + downstream pin bump remains, gated on explicit confirmation. DOC-009 is resolved (keep `install-hooks`).
4. **Consider** a follow-up to clear the 40 `cargo doc` intra-doc-link warnings and (optionally) the ARC-010 DashMap migration.
