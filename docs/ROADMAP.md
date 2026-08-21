# Roadmap status

Phases as defined in the original directive (`orbweaver` project brief).
Each gate is the directive's own acceptance criterion for the phase.

| Phase | Gate | Status |
|---|---|---|
| I — Foundation | Orbweaver can produce a reproducible ecosystem snapshot | **Done** |
| II — Capability intelligence | Orbweaver can explain what each repository *does* | **In progress** |
| III — ELCI integration | Orbweaver can enrich its model using real ELCI tool output | **In progress** |
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

## Phase III deliverables so far

- **ELCI tool discovery** (`orbweaver-connectors`): implements the
  directive's discovery doctrine (sections 26–27) directly — for each
  known tool name, checks whether a binary actually exists on PATH, and
  only ever runs `--help`/`--version` (plus a self-description command,
  but only if `--help` itself listed one first) rather than assuming any
  interface. `orbweaver integrations [--json]`.

  Verified against the real estate: 13/28 candidate binaries found (13
  tool names × `{name, name-cli}`). `deckhand` exposes a genuine
  machine-readable `capabilities` JSON manifest when probed from its own
  repo directory (the path already known from a snapshot) — the
  json-manifest discovery tier picked this up automatically, no
  guessing involved. Everything else fell back to help-text parsing
  across two different real formats in the wild (plain clap-default,
  and a hand-styled ANSI-colored template used by padagonia).

  Caught and fixed one real bug from running against real binaries:
  `cambrian`, `goglz`, `hellhound`, and `deliver` all reject `--version`
  with a nonzero exit and print clap's "unexpected argument" error —
  the version-fallback wasn't checking exit status, so that error text
  was being displayed as the tool's "version". Fixed by extracting the
  accept/reject decision into a pure, directly-tested function
  (`accept_version_output`) gated on process success.

  Tool list: the 13 named in directive section 4.3 (Ontism, Padagonia,
  Cambrian, Deckhand, Kaptaind, Skillastic, Switchboard, Mimic, Goglz,
  Hellhound, Isopod, Schem, Dreamseq) plus `deliver`, added after
  explicit confirmation that it's core ELCI infrastructure rather than
  just another scanned repository (ordinary repos found by `orbweaver
  scan` are not auto-promoted into the connector list — that would blur
  "every repo we found" with "the infrastructure we treat as a
  connector").

## Remaining Phase III work

Connector *capability mapping* is still shallow — a discovered command
list (e.g. padagonia's `bfs`/`vector-search`/`to-json`) isn't yet turned
into `Capability`/`Interface` records the way Rust repos get from
`orbweaver-ingest`, and connector reports aren't persisted into
snapshots (no `Evidence` trail, no `orbweaver integrations --repo`
history over time) — both natural next steps once there's a concrete
use for them, e.g. an opportunity that needs to cite "padagonia exposes
vector search" as evidence.
