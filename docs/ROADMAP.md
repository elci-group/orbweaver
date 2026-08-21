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
  directive's discovery doctrine (sections 26–27) — for each connector
  candidate, checks whether a binary actually exists on PATH, and only
  ever runs `--help`/`--version` (plus a self-description command, but
  only if `--help` itself listed one first) rather than assuming any
  interface. `orbweaver integrations [--org NAME] [--json]`.

  The tool list itself is no longer hardcoded in Orbweaver at all — it's
  discovered two ways, both real:

  1. **GitHub org discovery** (`orbweaver-connectors::github`): queries
     `gh repo list <org>` (defaults to `elci-group`) for the actual
     current repository inventory — 118 repos on the real org, not the
     13 the directive happened to name. Any git operation this crate
     performs uses the `sshUrl` GitHub returns, and only that — never an
     `https://` URL is constructed.
  2. **`.orb` self-declaration** (`orbweaver-connectors::orb`): most of
     those 118 repos are not ELCI tooling (client apps, render services,
     experiments), and Orbweaver has no principled way to guess which
     are without hardcoding a filter — so it doesn't guess. A repository
     opts itself in by carrying a `.orb` TOML file declaring binary name
     hints and whether Orbweaver may acquire it when missing. Checked
     locally only (in a repository already known from a snapshot) — a
     repo that's never been cloned can't be checked without cloning it
     speculatively, which this deliberately doesn't do for every one of
     118 repos. `.orb` files were added to the 11 local repos previously
     on the hardcoded list that still exist in the org (not `deliver` or
     `switchboard` — a real finding: neither exists in `elci-group` on
     GitHub at all, so GitHub-driven discovery correctly can't see them
     any more than it can see any other purely-local tool).

  **Robust local binary matching**: candidates are `.orb`'s explicit
  hints (if any) plus `{repo}`/`{repo}-cli`, all tried — not just the
  first match, so a repo like kaptaind that legitimately installs two
  separate binaries gets both reported, and a repo with zero matches
  gets one consolidated "not found" line instead of one miss per name
  variant tried.

  **JIT install** (`orbweaver-connectors::install`): when every
  candidate is missing and `.orb` marks the repo `installable`, Orbweaver
  acquires it right then, inline, before reporting — a matching GitHub
  release binary if one exists (checked by Rust target-triple substring
  in the asset name, e.g. `x86_64-unknown-linux-gnu`; downloaded via `gh
  release download`, `.tar.gz`/`.zip` extracted via `tar`/`unzip`, raw
  binary assets used directly), falling back to a source build via
  `baby --user` otherwise (building in the already-known local checkout
  in place — no redundant clone; SSH clone into an Orbweaver-managed
  cache dir is the fallback for a repo not yet known locally, currently
  unreachable in practice since `.orb` can only be checked on repos
  already local, but real, tested, and ready for when that changes).
  `orbweaver doctor` explicitly never triggers this (`allow_install =
  false`) — a routine health check must not have the side effect of
  cloning and building software.

  Verified end-to-end against the real estate, including the failure
  path: of the 11 `.orb`-flagged tools, 10 were already installed and
  probed correctly (`deckhand` via its genuine machine-readable
  `capabilities` JSON manifest, the rest via help-text parsing across
  two real formats — plain clap-default and padagonia's hand-styled
  ANSI-colored template). `ontism` was genuinely missing; release lookup
  correctly reported no matching asset and fell back to `baby`, which
  ran a real `cargo build` in `/home/sal/ontism` that failed on *actual*
  compile errors in that repo's own source (unrelated to Orbweaver) —
  reported cleanly as `Unavailable` with a tail of the real compiler
  output, not a crash or a silently-wrong success.

  Also caught and fixed a real bug from running against real binaries
  during the earlier hardcoded-list version: `cambrian`/`goglz`/
  `hellhound` all reject `--version` with a nonzero exit and print
  clap's "unexpected argument" error — the version-fallback wasn't
  checking exit status, so that error text was being displayed as the
  tool's "version". Fixed by extracting the accept/reject decision into
  a pure, directly-tested function (`accept_version_output`) gated on
  process success; still correct after the GitHub/`.orb` rewrite.

## Remaining Phase III work

Connector *capability mapping* is still shallow — a discovered command
list (e.g. padagonia's `bfs`/`vector-search`/`to-json`) isn't yet turned
into `Capability`/`Interface` records the way Rust repos get from
`orbweaver-ingest`, and connector reports aren't persisted into
snapshots (no `Evidence` trail, no `orbweaver integrations` history over
time) — both natural next steps once there's a concrete use for them,
e.g. an opportunity that needs to cite "padagonia exposes vector search"
as evidence. `.orb`'s `group` field is also accepted but not yet read by
anything — reserved for grouping/hierarchy features, not implemented
speculatively ahead of a real consumer.
