# Persistence conventions

These conventions apply to a slice's persistence port (`repository.rs`) and
its implementations (`repository/inmem.rs`, `repository/sqlx.rs`). The
account slice is the reference implementation.

- Persistence is event-sourced behind an aggregate-shaped port: `load`
  replays `reconstitute` from the stored stream; `save` checks the
  aggregate's `Versioned::version` and buffers its not-yet-buffered events;
  `commit` appends them behind a compare-and-swap and only then advances
  versions and drops the committed events.
- Repositories are transaction-scoped: the SQLx adapter acts as the unit of
  work, owning its transaction from `begin` and exposing
  `commit(accounts)`/`rollback`. `save` buffers appends in memory without
  mutating the aggregate, and `load` overlays those pending appends, so a
  unit of work sees its own writes. `commit` flushes every buffered append
  behind a compare-and-swap update and commits the transaction; only after
  the commit succeeds does it advance each aggregate's version and drop the
  committed prefix of its event log. Any error leaves the aggregates
  untouched (events and version), so the caller can retry; `rollback`
  discards pending appends without touching the aggregates. The in-memory
  adapter has no later failure point, so it commits and clears eagerly in
  `save`.
- Events are persisted as one row per event next to a per-stream version
  row; `load` trusts the stream row's version and rejects a mismatch with
  the event count.
- Serde derives on the event types define the payload format: the event
  variants and `Decimal`'s raw scaled integer are the durable format, so
  changing either or `Decimal::scale()` requires an event-store migration.
