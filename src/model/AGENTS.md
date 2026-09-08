## Domain Models

- Each file exposes a single aggregate root matching its name. Only the
  aggregate root and its domain events are public; child entity types stay
  private and never appear in public signatures.
- Commands use `(&mut self)` and return `Result<T, E>` where `E` is the
  aggregate's umbrella error (e.g. `AccountError`). `T` is situational and
  `()` unless something must be returned. Queries that don't mutate use
  `(&self)`.
- Each aggregate defines a single umbrella error (e.g. `AccountError`) that
  wraps two collected errors via `#[error(transparent)]`:
  `...PreconditionError` and `...InvariantError`. Per-method error enums are
  not defined.
- `mod preconditions` holds argument and readiness checks; `mod invariants`
  holds target-state checks. Both catalogues are the single source of truth,
  never duplicated in method bodies. Child-entity constructors validate
  through `mod invariants`.
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
- `Default` returns an empty aggregate; commands mutate it toward a desired
  state.
- `reconstitute(snapshot, Option<Vec<Event>>) -> Self` is infallible: it
  trusts persisted state, assigns fields directly and replays trailing events
  through `apply` without recording them. To address private field concerns,
  we define public-field DTOs with a `...State` suffix; they carry primitive
  payloads only, keeping the aggregate persistence-ignorant (repositories may
  live outside the crate).
- Consumption (`spent`) only enters an aggregate through spending commands or
  `reconstitute`.

### Check function structure

Invariants are the business rules that define the valid states of the
aggregate. Commands adapt to feed them — never the other way around: a
signature may be shaped so commands can call it conveniently, but the check
itself stays a statement about state, not about commands.

Both catalogues contain free functions returning
`Result<(), ...PreconditionError>` or `Result<(), ...InvariantError>`;
commands convert these into the umbrella error with `?`. They never borrow
the aggregate: a command judges its target state by passing the relevant
fragments, so no candidate state ever needs to be constructed.

- **Naming.** Name a check after the state rule it enforces
  (`check_spending_within_budget`: a category's spending may not exceed its
  budget plus surplus), not after a command that happens to trigger it
  (`check_spend_amount`). One rule serves many commands: `spend`,
  `transfer_surplus`, `reallocate_categories` and `repay_debt` all
  call `check_spending_within_budget` with different arguments.
- **Judged values vs. reported values.** Some parameters decide pass/fail;
  others exist only so the error can name the offender (`name: &str` in
  `check_spending_within_budget` builds the `SpendingExceedsAllocation`
  payload). Reported values, when present, come first, judged values after,
  so every signature reads "check that for *X*, the rule holds".
- **Judged values are always prospective.** A check judges the state as it
  will be *after* the command, so commands pass post-command values.

  Right — `spend`:
  `check_spending_within_budget(name, balance, allocation, surplus, spending.spent + amount)`
  Wrong:
  `check_spending_within_budget(name, balance, allocation, surplus, spending.spent)`
  The wrong call judges the current state, which is already valid — the
  check would always pass and `spend` could push a category over its
  budget. Likewise `remove_funds` passes `(self.balance, amount)`:
  subtraction panics on underflow (and silently wraps in release), so the
  check performs `safe_sub` internally and judges the result — passing
  `self.balance` would validate the past, not the future.
- **Collection rules own their reduction.** When a rule is about a whole
  collection (total allocations ≤ MAX), the check receives the items and
  folds them itself; the command only selects *which* items belong to the
  prospective state.

  Right — `add_category`:
  `check_total_allocations(self.categories.values().map(|s| s.allocation).chain(once(allocation)))`
  Wrong:
  `let total = self.categories.values().map(|s| s.allocation as u16).sum::<u16>() + allocation as u16;`
  `check_total_allocations(total)`
  The wrong form puts derivation (summing, upcasting) in the command, where
  it is covered by no check-function test and can silently drift from the
  rule.
- **A rule exists once.** `check_all_spending_within_budget` is a loop that
  delegates to `check_spending_within_budget` — the comparison lives in one
  place. A command passes only the entities whose prospective values need
  judging: `remove_category` omits the removed category because it no longer
  exists in the prospective state — its spent is settled into the balance by
  the removal math, so its *current* spent, judged against the shrunken
  post-command budget, would fail spuriously (spent 900, surplus 400, the
  balance shrunk by the 900 already paid out).
- **Non-trivial derivation is shared with `apply` — or it is a single
  expression.** Validation must never compute a value that `apply` also
  computes independently; the two would drift and the command would
  validate a fiction. `rollover` and `apply` therefore share
  `rollover_increment` and `flushed_total` for the flush math. A one-op
  mirror like `spend`'s `spent + amount` may stay inline: it is a single
  expression that visibly corresponds to `apply`'s `spent += amount`.
- **Overflow safety belongs to the check, not the caller.** A check never
  forces a command to widen a value. Either the operation is infallible in
  practice (addition of `Decimal` amounts cannot underflow and cannot
  realistically overflow, so `spend` passes `spending.spent + amount`
  as-is), or the check performs the fallible arithmetic itself with
  checked operations (`check_negative_balance` receives balance and
  deduction and judges the prospective balance via `safe_sub` internally;
  `check_total_allocations` receives u8 allocations and accumulates in
  u32). A command upcasting to satisfy a signature is glue — see the
  collection rule. Reductions accumulate into types whose overflow is out
  of the question for this application (u32 for counts); leaf values keep
  their natural narrow types.
- **Preconditions are the mirror image.** They ask whether the command
  makes sense *now*: their state parameters are current values
  (`check_category_exists(name, categories)` looks at the map as it is),
  and they may receive command arguments directly
  (`check_non_negative_amount(amount)` involves no state at all).

### Test-driven design

When adding a feature within the domain models, define the signature of the
command or event, identify the failure points, and place each check in
`mod preconditions` or `mod invariants` with its error variant. Then write the
minimum amount of tests that reproduce the failure condition by calling the
check functions directly, and proceed with the minimal implementation that
satisfies the tests. Preferably, mention what types of failure modes have
been identified and what the new tests are going to validate.

### Test conventions

- Failure modes are tested directly against the `preconditions::` and
  `invariants::` check functions — one minimal test per check branch,
  asserting the collected error variant.
- Command-level tests cover happy-path wiring (validate → emit → apply), the
  recorded event log, and the snapshot → `reconstitute` roundtrip with and
  without trailing events. Add command-level failure tests only when
  investigating an actual bug in a command.
- Derive numeric expectations from named constants (e.g. `MAX_ALLOCATION`),
  not literals.
- Seed consumption through spending commands, or `reconstitute` when testing
  the repository path.

These rules are defaults. When a case fits no rule here, or following one
would degrade the design, bring it to the user instead of force-fitting.
