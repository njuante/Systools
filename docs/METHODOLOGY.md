# SysTUI — Development Methodology

How we build SysTUI: branching, sessions, commits, and quality gates.
The phase/version map lives in [`ROADMAP.md`](ROADMAP.md).

---

## 1. Phases, versions, sessions

- Development is organized into **phases**. Each phase ships a **version** (`v0.1` … `v1.0`).
  Phase 0 (Foundation) is the substrate for `v0.1`.
- Each phase is split into **development sessions** — small, self-contained units of work.
- **The first session of every phase creates that phase's context file** in
  `docs/phases/` *before* any code is written. No coding starts until the context
  file is committed.

---

## 2. Branching model

`main` + one release branch per version.

```
main                       # only stable, tagged releases live here
 └─ release/v0.1           # all sessions of phase 0 + phase 1
      ├─ commit (S0.1 context)
      ├─ commit (S0.2 ...)
      └─ commit (S1.8 ...)
          ↓  merge --no-ff + tag v0.1
main (v0.1)
 └─ release/v0.2 ...
```

Rules:
- A version is developed entirely on its `release/vX.Y` branch.
- When a version meets its Definition of Done, merge into `main` with `--no-ff`
  and create an annotated tag `vX.Y`.
- `main` must always build and pass quality gates.
- Branches are created and committed by the assistant; the human reviews.

---

## 3. Sessions

A session is a focused chunk of work that ends in at least one commit and leaves
the branch in a buildable state.

Session lifecycle:
1. (Phase's first session only) Write the phase context file.
2. Implement the session's scope.
3. Run quality gates (`fmt`, `clippy`, `test`).
4. Commit (one or more commits) with a clear message.

---

## 4. Phase context file

Created in the first session of each phase as `docs/phases/phase-NN-vX.Y-slug.md`.
It freezes scope and decisions so later sessions stay aligned. Template:

```markdown
# Phase NN — vX.Y <Theme>

## Goal
One paragraph: what this version delivers and why it matters.

## In scope
- ...

## Out of scope (deferred)
- ...

## Key decisions
- Decision + rationale.

## Sessions
- SNN.1 — Context (this file)
- SNN.2 — ...

## Definition of Done
- Measurable criteria copied/refined from ROADMAP.

## Risks & open questions
- ...
```

---

## 5. Commit conventions

- **Language: English.** Everything — code, identifiers, app UI text, commit messages,
  docs — is in English. (`Product.md` is the only Spanish artifact.)
- **Conventional Commits**: `type(scope): subject`.
  - Types: `feat`, `fix`, `docs`, `refactor`, `test`, `chore`, `perf`, `build`, `ci`.
  - Scope = crate or module, e.g. `feat(transport): add MockTransport`.
- Imperative subject, < 72 chars. Body explains the *why* when non-obvious.
- **Never reference Claude, AI, or any assistant in commit messages or anywhere in
  the repo.** No co-author trailers.

---

## 6. Quality gates

Every session must leave the branch green:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Parsers of command output are always covered by fixture/golden-file tests
(`Product.md` §12) — text parsing without tests is forbidden.

---

## 7. Architecture guardrails (from `Product.md`)

- The UI never executes commands; it requests actions. The action engine decides.
- No free-form command strings — use `CommandSpec { program, args, requires_privilege }`.
- Every module works identically over `LocalTransport`, `SshTransport`, `MockTransport`.
- Security model (read-only / safe / privileged, confirmations, backups, audit)
  exists from Phase 0, not bolted on later.
- Missing permissions degrade gracefully ("partial data"), never crash.
