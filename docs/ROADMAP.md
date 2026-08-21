# Roadmap status

Phases as defined in the original directive (`orbweaver` project brief).
Each gate is the directive's own acceptance criterion for the phase.

| Phase | Gate | Status |
|---|---|---|
| I — Foundation | Orbweaver can produce a reproducible ecosystem snapshot | **Done** |
| II — Capability intelligence | Orbweaver can explain what each repository *does* | **In progress** |
| III — ELCI integration | Orbweaver can enrich its model using real ELCI tool output | Not started |
| IV — Opportunity engine | Orbweaver can produce non-obvious cross-project opportunities | Not started |
| V — Leverage reasoning | Orbweaver can answer "why this rather than that" | Not started |
| VI — Reasoning/evaluation | Recommendations outperform naive repo prioritisation | Not started |
| VII — Outcome learning | Forecasting accuracy improves measurably over time | Not started |
| VIII — Enterprise service | Postgres/Axum/RBAC/policy/web UI | Not started |

## Phase I deliverables (this repo, now)

- Rust workspace, six library crates + one CLI binary.
- Repository discovery with no hard-coded inventory.
- Deterministic manifest parsing: Cargo, npm, Poetry/PEP 621, Go modules.
- Deterministic git history inspection.
- Dependency-edge resolution between repositories in the same scan
  (path-match and name-match, both deterministic, both evidence-tagged).
- Evidence model with explicit confidence classes.
- Immutable snapshots in local SQLite.
- CLI: `scan`, `status`, `snapshots`, `graph`, `doctor`.

## Phase II deliverables so far

- **Capability extraction** (`orbweaver-ingest::capabilities`): a repo
  with a declared bin/script entry point (Cargo `[[bin]]`/`src/main.rs`,
  npm `bin`, PEP 621/Poetry `scripts`, Go `package main`) becomes a `Cli`
  capability; everything else with a manifest becomes a weaker `Library`
  capability. `evidence_sources` counts how many independent signals
  agree (entry point + description + README) instead of collapsing that
  into a bool. `orbweaver capabilities`.
- **Duplicate/shared-infrastructure detection** (`orbweaver-analysis`):
  finds external dependencies declared by more than one repository,
  filtered to exclude both one-offs and dependencies so common within
  their ecosystem (default >20%) that sharing them says nothing (i.e.
  `tokio`/`serde`-class crates get excluded, narrower shared choices
  don't). Explicitly labelled `ProbabilisticInference` — a shared
  dependency is a candidate for review, not proof of duplication.
  `orbweaver duplicates`. On the real estate this surfaced a real
  signal: `ratatui`+`crossterm` shared across `{goblin, jeenome, marty,
  scotia, spotcheck}` (five independent TUI implementations — a
  plausible shared-component-library opportunity), and `git2`+`semver`
  shared across `{kaptaind, npxr, lwoodz, amber, snakepit, skillastic}`
  (several tools independently doing git-based version logic, which is
  literally kaptaind's job).

## Remaining Phase II work

Still open before the Phase II gate is fully met: interface extraction
(what does a capability actually take/return — needs real source
analysis, not just manifest facts), schema detection, and
project-relationship modelling beyond the depends_on graph Phase I
already built. Capability *naming* is also still shallow — a `Cli`
capability's "description" is just whatever the manifest said, not an
aggregate of README content + test names the way directive section 12
describes; that's the natural next slice if it's worth the added
complexity before moving on to Phase III/IV.

## Phase III connector order

Not yet decided — the directive is explicit that connector order should
follow "actual discovered interfaces and leverage," not a fixed list.
First step when Phase III starts: run `orbweaver doctor`-style discovery
against each of Ontism, Padagonia, Kaptaind, Deckhand, Skillastic,
Switchboard, Cambrian, Mimic, Goglz, Hellhound, Isopod, Schem to find out
which actually expose a machine-readable CLI/API surface today, and
prioritise from that, not from the order they're listed in the directive.
