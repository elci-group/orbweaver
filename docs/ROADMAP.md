# Roadmap status

Phases as defined in the original directive (`orbweaver` project brief).
Each gate is the directive's own acceptance criterion for the phase.

| Phase | Gate | Status |
|---|---|---|
| I — Foundation | Orbweaver can produce a reproducible ecosystem snapshot | **Done** |
| II — Capability intelligence | Orbweaver can explain what each repository *does* | Not started |
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

## Immediate next step (start of Phase II)

Capability extraction needs more than a manifest line to name a
capability (directive section 12: "function ≠ capability" — aggregate
evidence from function/CLI surface/README/tests/consumers/schema). The
natural first slice: parse each repo's public CLI surface (`clap`
`Command` definitions, or `--help` output for non-Rust tools) as one
evidence source, combine with README section headers as a second, and
only name a capability when at least two independent sources agree.
Prerequisite for the leverage engine having anything real to rank.

## Phase III connector order

Not yet decided — the directive is explicit that connector order should
follow "actual discovered interfaces and leverage," not a fixed list.
First step when Phase III starts: run `orbweaver doctor`-style discovery
against each of Ontism, Padagonia, Kaptaind, Deckhand, Skillastic,
Switchboard, Cambrian, Mimic, Goglz, Hellhound, Isopod, Schem to find out
which actually expose a machine-readable CLI/API surface today, and
prioritise from that, not from the order they're listed in the directive.
