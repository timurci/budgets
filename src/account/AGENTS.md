# account slice

## Layout

- `model.rs` — the `Account` aggregate root, the `AccountState`/`CategoryState`
  DTOs, and the `AccountId`/`CategoryId`/`DebtId` newtypes.
- `event.rs` — `AccountEvent`, the durable payload.
- `error.rs` — the `AccountError` umbrella and the precondition/invariant
  error types.
- `spending.rs` — the private `Spending` child entity and the budget math
  shared by `apply` and the checks.
- `precondition.rs` / `invariant.rs` — the check catalogues.
- `repository.rs` — the `AccountRepository` port and `RepositoryError`.
- `repository/inmem.rs` — the in-memory implementation used by domain
  tests.
- `repository/sqlx.rs` — the SQLx unit of work over `account_streams` and
  `account_events`, generic over SQLite and PostgreSQL.

## Conventions

- [Domain modeling](../../docs/domain-modeling.md)
- [Validation](../../docs/validation.md)
- [Persistence](../../docs/persistence.md)

## Slice facts

- Streams are versioned in `account_streams`; events live one row per event
  in `account_events`, keyed by `(account_id, stream_version)`.
- `AccountEvent` is the durable payload; see
  [persistence](../../docs/persistence.md) for the evolution policy.
- Consumption (`spent`) enters the aggregate only through spending commands
  or `reconstitute`.
- `save` never mutates an aggregate; `commit(accounts)` advances versions and
  drops committed events only after the transaction commits.
