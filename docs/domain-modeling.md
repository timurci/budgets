# Domain modeling conventions

These conventions apply to every slice's domain model. The account slice is
the reference implementation, so its types appear in the examples.

- Each aggregate lives in its own slice folder; `model.rs` defines the
  aggregate root, `event.rs` its domain events, and `error.rs` the umbrella
  error. Only the aggregate root, its events, and its errors are public;
  child entity types and the check catalogues stay private to the slice and
  never appear in public signatures.
- Commands use `(&mut self)` and return `Result<T, E>` where `E` is the
  aggregate's umbrella error (e.g. `AccountError`). `T` is situational and
  `()` unless something must be returned. Queries that don't mutate use
  `(&self)`.
- Each aggregate defines a single umbrella error (e.g. `AccountError`) that
  wraps two collected errors via `#[error(transparent)]`:
  `...PreconditionError` and `...InvariantError`. Per-method error enums are
  not defined.
- The `precondition` module holds argument and readiness checks; the
  `invariant` module holds target-state checks. Both catalogues are the
  single source of truth, never duplicated in method bodies. Child-entity
  constructors validate through the `invariant` module. See
  [validation.md](validation.md) for how checks are structured.
- Validation is method-specific, not initialization-based: each command calls
  the precondition checks relevant to its arguments and the invariant checks
  relevant to its target state (e.g. `spend` checks the amount against the
  category's remaining budget plus surplus).
- Domain events (e.g. `AccountEvent`) carry command inputs.
  `apply(&mut self, event: &Event)` is private, contains the transition logic,
  has no checks and does not record. `emit(&mut self, event)` is private: it
  applies the event and records it. Only validated events are emitted;
  commands validate first and then emit, so a failed command leaves the
  aggregate untouched.
- `Default` returns an empty aggregate with a freshly minted `AccountId`;
  commands mutate it toward a desired state.
- `reconstitute(snapshot, Option<Vec<Event>>) -> Self` is infallible: it
  trusts persisted state (including the aggregate id carried by the
  `...State` DTO), assigns fields directly and replays trailing events
  through `apply` without recording them. To address private field concerns,
  we define public-field DTOs with a `...State` suffix; they carry primitive
  payloads only, keeping the aggregate persistence-ignorant (repositories may
  live outside the crate).
- Identity is always a type, never a name. The aggregate-owned id newtypes
  (`AccountId`, `CategoryId`, `DebtId`) are defined in `model.rs` from the
  `id_type!` macro in `src/types/id.rs`. Child entities are keyed by
  aggregate-generated `CategoryId`/`DebtId`; `add_category`/`add_debt`
  generate them (`new_v7`) and return them. Display names are not domain
  state; they belong to a read model keyed by id (deferred).
- Events and snapshots carry ids only. Errors report ids; mapping them to
  display names is the application's concern.

## Test-driven design

When adding a feature within the domain models, define the signature of the
command or event, identify the failure points, and place each check in
`mod preconditions` or `mod invariants` with its error variant. Then write the
minimum amount of tests that reproduce the failure condition by calling the
check functions directly, and proceed with the minimal implementation that
satisfies the tests. Preferably, mention what types of failure modes have
been identified and what the new tests are going to validate.

## Test conventions

- Failure modes are tested directly against the `precondition::` and
  `invariant::` check functions — one minimal test per check branch,
  asserting the collected error variant.
- Command-level tests cover happy-path wiring (validate → emit → apply), the
  recorded event log, and the snapshot → `reconstitute` roundtrip with and
  without trailing events. Add command-level failure tests only when
  investigating an actual bug in a command.
- Derive numeric expectations from named constants (e.g. `MAX_ALLOCATION`),
  not literals.
- Seed aggregate state through commands, or `reconstitute` when testing the
  repository path.
