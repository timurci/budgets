## Domain Models

- Currently each file is intended to expose a single aggregate root reflecting
  its name. Only the aggregate root is public; child entity types stay private
  and never appear in public signatures.
- Entity methods use `(&self)` without consuming the object and return
  `<Self, CustomError>`. Every method recreates the aggregate through
  initialization, so aggregate invariants are re-checked on each step.
- There are two types of failure points in object methods: command
  preconditions or failing aggregate invariants. Each method defines its
  errors and checks its command preconditions itself, while deferring
  aggregate invariants to initialization.
- Aggregate invariants live in a per-module invariant catalogue — the single
  source of truth, never duplicated in method bodies. Initialization runs the
  full catalogue for every method: drift-safety over per-method check lists.
- `Default` returns an empty aggregate; chained methods act as the builder for
  reaching a desired state. The private `new` is the initialization door that
  runs the invariant catalogue over given parts.
- We expose `pub fn reconstitute` at the aggregate root to load a persisted
  state without going through the builder-like pattern. To address private
  field concerns, we define public-field DTOs with a `...State` suffix; they
  carry primitive payloads only, keeping the aggregate persistence-ignorant
  (repositories may live outside the crate).
- Consumption (`spent`) only enters an aggregate through spending commands or
  `reconstitute`.

### Test-driven design

When adding a feature within the domain models, define the signature of the
method or function, then identify the failure points and write the error enum
variants. Then write the minimum amount of tests that reproduce the failure
condition and see the test fail without complete implementation, then proceed
with the minimal implementation that satisfies the tests. Preferably, mention
what types of failure modes have been identified and what would the new tests
going to validate.

### Test conventions

- Derive numeric expectations from named constants (e.g. `MAX_ALLOCATION`),
  not literals.
- Seed consumption through spending commands, or `reconstitute` when testing
  the repository path; keep a snapshot → reconstitute roundtrip test.
- No inline comments.
