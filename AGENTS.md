# budgets

This is a personal budget management project. The project follows
domain-driven design (DDD) conventions.

## Project structure

- `src/lib.rs` — crate entry point
- `src/<slice>/` — one feature slice per aggregate; each slice carries its
  own `AGENTS.md` mapping its files and linking the shared conventions —
  read the slice's `AGENTS.md` before changing it (see
  [src/account/AGENTS.md](src/account/AGENTS.md) for the account slice)
- `src/types/` — shared value objects: `Decimal` (fixed-point money with 8
  decimal places backed by `u64`) and `Versioned<T>` (a value plus its
  event-stream version); `id.rs` provides the crate-internal `id_type!`
  macro for aggregate-owned UUIDv7 id newtypes
- `migrations/sqlite/`, `migrations/postgres/` — event-store schema,
  embedded by `sqlx::migrate!` and applied automatically by `#[sqlx::test]`
  for SQLite
- `docs/` — shared conventions:
  [domain-modeling.md](docs/domain-modeling.md) (model.rs anatomy, events,
  reconstitution, testing), [validation.md](docs/validation.md)
  (precondition and invariant check structure),
  [persistence.md](docs/persistence.md) (event-store port and unit of work).
  `docs/design.md` holds local project requirements and is not committed.

The conventions in `docs/` are defaults. When a case fits no rule, or
following a rule would degrade the design, bring it to the user instead of
force-fitting.

## Commands

All three must pass before a change is done:

- `cargo test`
- `cargo clippy --all-targets --all-features`
- `cargo fmt`

The `sqlite` (default) and `postgres` Cargo features select the SQLx
drivers; `cargo test --no-default-features` builds the domain without
either. SQLite repository tests run against throwaway databases managed by
`#[sqlx::test]` and need no `DATABASE_URL`. The PostgreSQL smoke test is
compiled with `--features postgres` and skips itself unless `DATABASE_URL`
holds a PostgreSQL URL.
