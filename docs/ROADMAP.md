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

- Rust workspace, seven library crates + one CLI binary.
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

- **CLI interface extraction** (`orbweaver-ingest::interfaces`): the
  first real *source*-level analysis, not just manifest facts. Scans
  `.rs` files for `#[derive(Subcommand)]` enums (text-pattern, no
  proc-macro expansion, no `cargo build`/execution) and reads off each
  variant as an `Interface` — name converted to clap's default
  kebab-case, description from its `///` doc comment. Always
  `ProbabilisticInference`; explicitly can't see builder-style clap,
  non-Rust CLIs, or `#[command(name = ...)]` renames. `orbweaver
  interfaces`.

  Caught and fixed two real correctness bugs via dogfooding (running it
  on Orbweaver's own source, where the right answer was known): (1) the
  brace/comma scanner didn't understand raw strings, so a `#[cfg(test)]`
  fixture containing lookalike CLI source as a string literal got parsed
  as if it were real code — fixed by stripping test modules before
  scanning, and by teaching the scanner about `r#"..."#` raw strings and
  the char-literal-vs-lifetime ambiguity (`'a'` vs `&'a str`) so it
  doesn't desync on either; (2) a comma *inside a `///` doc comment*
  (e.g. "Discover repositories under a root, extract deterministic
  evidence...") was splitting one variant into two garbage ones — real
  code has commas in doc comments constantly, this wasn't an edge case.
  Both are covered by regression tests. Verified against ground truth:
  `orbweaver interfaces --repo orbweaver` now reports exactly the 8 real
  subcommands with correct full descriptions.

- **Schema detection** (`orbweaver-ingest::schemas`): scans `.rs` files
  for `#[derive(Serialize/Deserialize)]` structs and reports each as a
  `Schema` with its fields (name + type as written) and struct-level doc
  comment. Built on the same scanning primitives as interface extraction
  — the raw-string/comment/`#[cfg(test)]` handling was factored out into
  a shared `orbweaver-ingest::rust_scan` module rather than duplicated,
  specifically so the bugs already found and fixed there couldn't be
  reintroduced here. Always `ProbabilisticInference`; only recognises
  `struct { field: Type, ... }` bodies (no tuple/unit structs, no fields
  contributed by other macros). `orbweaver schemas`. Verified against
  ground truth: `orbweaver schemas --repo orbweaver` reports exactly the
  11 real serde structs in this workspace (correctly excluding
  serde-derived *enums* like `CapabilityKind`/`Confidence`, which the
  struct-only matcher doesn't claim to handle) with accurate fields and
  doc comments.

## Remaining Phase II work

Still open before the Phase II gate is fully met: project-relationship
modelling beyond the depends_on graph Phase I already built, and latent
capability detection (directive: capabilities trapped behind interfaces/
schemas/missing adapters — this overlaps with Phase IV opportunity type
B and may be better done there, once there's an opportunity engine to
put the finding in). Capability *naming* is also still shallow — a
`Cli` capability's "description" is just whatever the manifest said,
not an aggregate of README content + interface names the way directive
section 12 describes; combining capability + interface + schema data
into richer capability descriptions is the natural next slice if it's
worth the added complexity before moving on to Phase III/IV.

## Phase III connector order

Not yet decided — the directive is explicit that connector order should
follow "actual discovered interfaces and leverage," not a fixed list.
First step when Phase III starts: run `orbweaver doctor`-style discovery
against each of Ontism, Padagonia, Kaptaind, Deckhand, Skillastic,
Switchboard, Cambrian, Mimic, Goglz, Hellhound, Isopod, Schem to find out
which actually expose a machine-readable CLI/API surface today, and
prioritise from that, not from the order they're listed in the directive.
