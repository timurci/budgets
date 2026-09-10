# budgets

This is a personal budget management project. The project follows
domain-driven design (DDD) conventions.

## Project structure

- `src/lib.rs` — crate entry point
- `src/account/` — the account aggregate: `model.rs` holds the aggregate
  root and the `AccountId`/`CategoryId`/`DebtId` newtypes, and
  `repository.rs` the persistence trait — see
  [src/account/AGENTS.md](src/account/AGENTS.md) for domain conventions
- `src/types/` — shared value objects: `Decimal` (fixed-point money with 8
  decimal places backed by `u64`) and `Versioned<T>` (a value plus its
  event-stream version); `id.rs` provides the crate-internal `id_type!`
  macro for aggregate-owned UUIDv7 id newtypes
- `docs/design.md` — project requirements

## Commands

All three must pass before a change is done:

- `cargo test`
- `cargo clippy --all-targets`
- `cargo fmt`
