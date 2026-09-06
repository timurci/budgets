# budgets

This is a personal budget management project. The project follows
domain-driven design (DDD) conventions.

## Project structure

- `src/lib.rs` — crate entry point
- `src/model/` — domain models; each file exposes a single aggregate root —
  see [src/model/AGENTS.md](src/model/AGENTS.md) for domain conventions
- `src/types/` — shared value objects (`Decimal`: fixed-point money with 8
  decimal places backed by `u64`)
- `docs/design.md` — project requirements

## Commands

All three must pass before a change is done:

- `cargo test`
- `cargo clippy --all-targets`
- `cargo fmt`
