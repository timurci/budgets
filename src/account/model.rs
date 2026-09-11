use std::collections::HashMap;

use super::error::AccountError;
use super::event::AccountEvent;
use super::spending::{Spending, budget, removal_balance, total_budget};
use super::{invariant, precondition};
use crate::types::Decimal;
use crate::types::id::id_type;

id_type!(AccountId, CategoryId, DebtId);

#[derive(Clone, Debug)]
pub struct Account {
    id: AccountId,
    balance: Decimal,
    categories: HashMap<CategoryId, Spending>,
    debts: HashMap<DebtId, Decimal>,
    events: Vec<AccountEvent>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AccountState {
    pub id: AccountId,
    pub balance: Decimal,
    pub categories: HashMap<CategoryId, CategoryState>,
    pub debts: HashMap<DebtId, Decimal>,
}

impl AccountState {
    pub fn empty(id: AccountId) -> Self {
        Self {
            id,
            balance: Decimal::ZERO,
            categories: HashMap::new(),
            debts: HashMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CategoryState {
    pub allocation: u8,
    pub spent: Decimal,
    pub surplus: Decimal,
}

impl Account {
    pub fn id(&self) -> &AccountId {
        &self.id
    }

    pub fn events(&self) -> &[AccountEvent] {
        &self.events
    }

    /// Drops the first `committed` recorded events, keeping any events emitted
    /// after the last `save`.
    pub fn mark_committed(&mut self, committed: usize) {
        let drop_count = committed.min(self.events.len());
        drop(self.events.drain(..drop_count));
    }

    pub fn add_category(&mut self, allocation: u8) -> Result<CategoryId, AccountError> {
        Spending::new(allocation, Decimal::ZERO, Decimal::ZERO)?;
        invariant::check_total_allocations(
            self.categories
                .values()
                .map(|spending| spending.allocation)
                .chain(std::iter::once(allocation)),
        )?;
        let id = CategoryId::new_v7();
        self.emit(AccountEvent::CategoryAdded { id, allocation });
        Ok(id)
    }

    pub fn remove_category(&mut self, id: &CategoryId) -> Result<(), AccountError> {
        precondition::check_category_exists(id, &self.categories)?;
        let spending = self.categories.get(id).expect("existence checked");
        let funded = self.balance + spending.surplus;
        invariant::check_negative_balance(funded, spending.spent)?;
        let balance = removal_balance(self.balance, spending.surplus, spending.spent);
        invariant::check_all_spending_within_budget(
            balance,
            self.categories
                .iter()
                .filter(|(existing, _)| *existing != id),
        )?;
        self.emit(AccountEvent::CategoryRemoved { id: *id });
        Ok(())
    }

    pub fn rollover(&mut self, income: Decimal) -> Result<(), AccountError> {
        for spending in self.categories.values() {
            invariant::check_negative_surplus(
                self.balance,
                spending.allocation,
                spending.surplus,
                spending.spent,
            )?;
        }
        invariant::check_negative_balance(
            income + self.balance,
            total_budget(self.balance, &self.categories),
        )?;
        self.emit(AccountEvent::RolledOver { income });
        Ok(())
    }

    pub fn spend(&mut self, category: &CategoryId, amount: Decimal) -> Result<(), AccountError> {
        precondition::check_category_exists(category, &self.categories)?;
        let spending = self.categories.get(category).expect("existence checked");
        invariant::check_spending_within_budget(
            category,
            self.balance,
            spending.allocation,
            spending.surplus,
            spending.spent + amount,
        )?;
        self.emit(AccountEvent::Spent {
            category: *category,
            amount,
        });
        Ok(())
    }

    pub fn transfer_surplus(
        &mut self,
        from: &CategoryId,
        to: &CategoryId,
        amount: Decimal,
    ) -> Result<(), AccountError> {
        precondition::check_distinct_source_target(from, to)?;
        precondition::check_source_exists(from, &self.categories)?;
        precondition::check_target_exists(to, &self.categories)?;
        let source = self.categories.get(from).expect("source checked");
        invariant::check_transfer_within_surplus(
            from,
            self.balance,
            source.allocation,
            source.surplus,
            source.spent,
            amount,
        )?;
        self.emit(AccountEvent::SurplusTransferred {
            from: *from,
            to: *to,
            amount,
        });
        Ok(())
    }

    pub fn add_funds(&mut self, amount: Decimal) -> Result<(), AccountError> {
        self.emit(AccountEvent::FundsAdded { amount });
        Ok(())
    }

    pub fn remove_funds(&mut self, amount: Decimal) -> Result<(), AccountError> {
        invariant::check_negative_balance(self.balance, amount)?;
        let balance = self.balance - amount;
        invariant::check_all_spending_within_budget(balance, &self.categories)?;
        self.emit(AccountEvent::FundsRemoved { amount });
        Ok(())
    }

    pub fn reallocate_categories(
        &mut self,
        allocations: HashMap<CategoryId, u8>,
    ) -> Result<(), AccountError> {
        precondition::check_categories_match(&self.categories, &allocations)?;
        invariant::check_total_allocations(allocations.values().copied())?;
        for (id, allocation) in &allocations {
            let existing = self.categories.get(id).expect("keys matched");
            Spending::new(*allocation, existing.spent, existing.surplus)?;
            invariant::check_spending_within_budget(
                id,
                self.balance,
                *allocation,
                existing.surplus,
                existing.spent,
            )?;
        }
        self.emit(AccountEvent::CategoriesReallocated { allocations });
        Ok(())
    }

    pub fn add_debt(&mut self, amount: Decimal) -> Result<DebtId, AccountError> {
        let id = DebtId::new_v7();
        self.emit(AccountEvent::DebtAdded { id, amount });
        Ok(id)
    }

    pub fn repay_debt(
        &mut self,
        category: &CategoryId,
        debt: &DebtId,
        amount: Decimal,
    ) -> Result<(), AccountError> {
        precondition::check_category_exists(category, &self.categories)?;
        precondition::check_debt_exists(debt, &self.debts)?;
        let owed = self.debts.get(debt).expect("debt existence checked");
        invariant::check_debt_within_owed(debt, *owed, amount)?;
        let spending = self.categories.get(category).expect("existence checked");
        invariant::check_spending_within_budget(
            category,
            self.balance,
            spending.allocation,
            spending.surplus,
            spending.spent + amount,
        )?;
        self.emit(AccountEvent::DebtRepaid {
            category: *category,
            debt: *debt,
            amount,
        });
        Ok(())
    }

    pub fn snapshot(&self) -> AccountState {
        AccountState {
            id: self.id,
            balance: self.balance,
            categories: self
                .categories
                .iter()
                .map(|(id, spending)| {
                    (
                        *id,
                        CategoryState {
                            allocation: spending.allocation,
                            spent: spending.spent,
                            surplus: spending.surplus,
                        },
                    )
                })
                .collect(),
            debts: self.debts.clone(),
        }
    }

    /// Rebuilds an account from persisted state and trailing events.
    ///
    /// Infallible by design: only pass snapshots and events produced by this
    /// aggregate, as loaded from the event store. Replaying arbitrary or
    /// corrupted events can panic in `apply`.
    pub fn reconstitute(snapshot: AccountState, events: Option<Vec<AccountEvent>>) -> Account {
        let mut account = Account {
            id: snapshot.id,
            balance: snapshot.balance,
            categories: snapshot
                .categories
                .into_iter()
                .map(|(id, category)| {
                    (
                        id,
                        Spending {
                            allocation: category.allocation,
                            spent: category.spent,
                            surplus: category.surplus,
                        },
                    )
                })
                .collect(),
            debts: snapshot.debts,
            events: Vec::new(),
        };
        if let Some(events) = events {
            for event in events {
                account.apply(&event);
            }
        }
        account
    }

    fn apply(&mut self, event: &AccountEvent) {
        match event {
            AccountEvent::CategoryAdded { id, allocation } => {
                self.categories.insert(
                    *id,
                    Spending {
                        allocation: *allocation,
                        spent: Decimal::ZERO,
                        surplus: Decimal::ZERO,
                    },
                );
            }
            AccountEvent::CategoryRemoved { id } => {
                let spending = self.categories.remove(id).expect("validated event");
                self.balance = removal_balance(self.balance, spending.surplus, spending.spent);
            }
            AccountEvent::RolledOver { income } => {
                let balance = self.balance;
                let flushed = total_budget(balance, &self.categories);
                for spending in self.categories.values_mut() {
                    let current = budget(balance, spending.allocation);
                    spending.surplus = spending.surplus + current - spending.spent;
                    spending.spent = Decimal::ZERO;
                }
                self.balance = *income + (balance - flushed);
            }
            AccountEvent::Spent { category, amount } => {
                self.categories
                    .get_mut(category)
                    .expect("validated event")
                    .spent += *amount;
            }
            AccountEvent::SurplusTransferred { from, to, amount } => {
                self.categories
                    .get_mut(from)
                    .expect("validated event")
                    .surplus -= *amount;
                self.categories
                    .get_mut(to)
                    .expect("validated event")
                    .surplus += *amount;
            }
            AccountEvent::FundsAdded { amount } => {
                self.balance += *amount;
            }
            AccountEvent::FundsRemoved { amount } => {
                self.balance -= *amount;
            }
            AccountEvent::CategoriesReallocated { allocations } => {
                for (id, allocation) in allocations {
                    self.categories
                        .get_mut(id)
                        .expect("validated event")
                        .allocation = *allocation;
                }
            }
            AccountEvent::DebtAdded { id, amount } => {
                self.debts.insert(*id, *amount);
            }
            AccountEvent::DebtRepaid {
                category,
                debt,
                amount,
            } => {
                self.categories
                    .get_mut(category)
                    .expect("validated event")
                    .spent += *amount;
                let remaining = self
                    .debts
                    .get(debt)
                    .expect("validated event")
                    .safe_sub(*amount)
                    .expect("validated event");
                if remaining.is_zero() {
                    self.debts.remove(debt);
                } else {
                    self.debts.insert(*debt, remaining);
                }
            }
        }
    }

    fn emit(&mut self, event: AccountEvent) {
        self.apply(&event);
        self.events.push(event);
    }
}

impl Default for Account {
    fn default() -> Self {
        Self {
            id: AccountId::new_v7(),
            balance: Decimal::ZERO,
            categories: HashMap::new(),
            debts: HashMap::new(),
            events: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::error::AccountInvariantError;
    use super::super::spending::MAX_ALLOCATION;
    use super::*;
    use std::collections::HashMap;

    fn account_with(
        balance: Decimal,
        allocations: &[(&str, u8)],
    ) -> (Account, HashMap<String, CategoryId>) {
        let mut account = Account::default();
        account.add_funds(balance).unwrap();
        let ids = allocations
            .iter()
            .map(|(name, allocation)| {
                let id = account.add_category(*allocation).unwrap();
                ((*name).to_string(), id)
            })
            .collect();
        (account, ids)
    }

    #[test]
    fn default_is_empty() {
        let account = Account::default();
        let state = account.snapshot();
        assert_eq!(state.id, *account.id());
        assert_eq!(state.balance, Decimal::ZERO);
        assert!(state.categories.is_empty());
        assert!(state.debts.is_empty());
    }

    #[test]
    fn add_category_succeeds() {
        let (mut account, _) = account_with(Decimal::from(1000u64), &[]);
        let id = account.add_category(10).unwrap();
        let state = account.snapshot();
        assert_eq!(state.balance, Decimal::from(1000u64));
        assert_eq!(
            state.categories.get(&id).unwrap(),
            &CategoryState {
                allocation: 10,
                spent: Decimal::ZERO,
                surplus: Decimal::ZERO,
            }
        );
    }

    #[test]
    fn add_debt_returns_id() {
        let (mut account, _) = account_with(Decimal::from(1000u64), &[]);
        let id = account.add_debt(Decimal::from(400u64)).unwrap();
        assert_eq!(
            account.snapshot().debts.get(&id),
            Some(&Decimal::from(400u64))
        );
    }

    #[test]
    fn remove_category_succeeds() {
        let (mut account, cats) = account_with(Decimal::from(1000u64), &[("test", 10)]);
        account.remove_category(&cats["test"]).unwrap();
        let state = account.snapshot();
        assert_eq!(state.balance, Decimal::from(1000u64));
        assert!(state.categories.is_empty());
    }

    #[test]
    fn remove_after_transferring_surplus() {
        let (mut account, cats) = account_with(
            Decimal::from(10000u64),
            &[("a", MAX_ALLOCATION / 2), ("b", MAX_ALLOCATION / 4)],
        );
        account.rollover(Decimal::ZERO).unwrap();
        let surplus_a = Decimal::from(10000u64) * (MAX_ALLOCATION / 2) / MAX_ALLOCATION;
        account
            .transfer_surplus(&cats["a"], &cats["b"], surplus_a)
            .unwrap();
        account.remove_category(&cats["a"]).unwrap();
        let state = account.snapshot();
        assert!(!state.categories.contains_key(&cats["a"]));
        let surplus_b = Decimal::from(10000u64) * (MAX_ALLOCATION / 4) / MAX_ALLOCATION
            + Decimal::from(10000u64) * (MAX_ALLOCATION / 2) / MAX_ALLOCATION;
        assert_eq!(state.categories.get(&cats["b"]).unwrap().surplus, surplus_b);
    }

    #[test]
    fn rollover_flushes_balance_into_surplus_and_resets_spent() {
        let (mut account, cats) = account_with(
            Decimal::from(10000u64),
            &[("a", MAX_ALLOCATION / 2), ("b", MAX_ALLOCATION / 4)],
        );
        account.spend(&cats["a"], Decimal::from(1000u64)).unwrap();
        account.rollover(Decimal::from(5000u64)).unwrap();
        let state = account.snapshot();
        let budget_a = Decimal::from(10000u64) * (MAX_ALLOCATION / 2) / MAX_ALLOCATION;
        let budget_b = Decimal::from(10000u64) * (MAX_ALLOCATION / 4) / MAX_ALLOCATION;
        assert_eq!(
            state.categories.get(&cats["a"]).unwrap().surplus,
            budget_a - Decimal::from(1000u64)
        );
        assert_eq!(state.categories.get(&cats["b"]).unwrap().surplus, budget_b);
        assert_eq!(
            state.categories.get(&cats["a"]).unwrap().spent,
            Decimal::ZERO
        );
        assert_eq!(
            state.balance,
            Decimal::from(5000u64) + (Decimal::from(10000u64) - (budget_a + budget_b))
        );
    }

    #[test]
    fn rollover_with_full_allocation_leaves_only_income() {
        let (mut account, _) = account_with(
            Decimal::from(10000u64),
            &[("a", MAX_ALLOCATION / 2), ("b", MAX_ALLOCATION / 2)],
        );
        account.rollover(Decimal::from(2000u64)).unwrap();
        let state = account.snapshot();
        assert_eq!(state.balance, Decimal::from(2000u64));
    }

    #[test]
    fn rollover_expenses_spent_from_total() {
        let (mut account, cats) =
            account_with(Decimal::from(10000u64), &[("a", MAX_ALLOCATION / 2)]);
        account.spend(&cats["a"], Decimal::from(1000u64)).unwrap();
        account.rollover(Decimal::from(5000u64)).unwrap();
        let state = account.snapshot();
        let budget_a = Decimal::from(10000u64) * (MAX_ALLOCATION / 2) / MAX_ALLOCATION;
        assert_eq!(
            state.balance,
            Decimal::from(5000u64) + (Decimal::from(10000u64) - budget_a)
        );
        assert_eq!(
            state.categories.get(&cats["a"]).unwrap().surplus,
            budget_a - Decimal::from(1000u64)
        );
    }

    #[test]
    fn rollover_accumulates_existing_surplus() {
        let (mut account, cats) =
            account_with(Decimal::from(10000u64), &[("a", MAX_ALLOCATION / 2)]);
        account.rollover(Decimal::ZERO).unwrap();
        account.spend(&cats["a"], Decimal::from(2000u64)).unwrap();
        account.rollover(Decimal::ZERO).unwrap();
        let state = account.snapshot();
        let first_budget = Decimal::from(10000u64) * (MAX_ALLOCATION / 2) / MAX_ALLOCATION;
        let second_budget =
            (Decimal::from(10000u64) - first_budget) * (MAX_ALLOCATION / 2) / MAX_ALLOCATION;
        let increment = second_budget - Decimal::from(2000u64);
        assert_eq!(
            state.categories.get(&cats["a"]).unwrap().surplus,
            first_budget + increment
        );
        assert_eq!(
            state.categories.get(&cats["a"]).unwrap().spent,
            Decimal::ZERO
        );
        assert_eq!(
            state.balance,
            (Decimal::from(10000u64) - first_budget) - second_budget
        );
    }

    #[test]
    fn remove_category_settles_spent_into_balance() {
        let (mut account, cats) =
            account_with(Decimal::from(10000u64), &[("a", MAX_ALLOCATION / 4)]);
        account.spend(&cats["a"], Decimal::from(1000u64)).unwrap();
        account.remove_category(&cats["a"]).unwrap();
        let state = account.snapshot();
        assert_eq!(state.balance, Decimal::from(9000u64));
        assert!(!state.categories.contains_key(&cats["a"]));
    }

    #[test]
    fn remove_category_fails_when_sibling_exceeds_shrunken_budget() {
        let (mut account, cats) = account_with(
            Decimal::from(10000u64),
            &[("a", MAX_ALLOCATION / 2), ("b", MAX_ALLOCATION / 2)],
        );
        let full_budget = Decimal::from(10000u64) * (MAX_ALLOCATION / 2) / MAX_ALLOCATION;
        account.spend(&cats["a"], full_budget).unwrap();
        account.spend(&cats["b"], full_budget).unwrap();
        let result = account.remove_category(&cats["a"]);
        assert!(matches!(
            result,
            Err(AccountError::Invariant(
                AccountInvariantError::SpendingExceedsAllocation(id, _, _, _)
            )) if id == cats["b"]
        ));
    }

    #[test]
    fn remove_category_returns_surplus_to_balance() {
        let (mut account, cats) =
            account_with(Decimal::from(10000u64), &[("a", MAX_ALLOCATION / 2)]);
        account.rollover(Decimal::ZERO).unwrap();
        account.remove_category(&cats["a"]).unwrap();
        let state = account.snapshot();
        assert_eq!(state.balance, Decimal::from(10000u64));
        assert!(!state.categories.contains_key(&cats["a"]));
    }

    #[test]
    fn remove_category_settles_surplus_minus_spent() {
        let (mut account, cats) = account_with(
            Decimal::from(10000u64),
            &[("a", MAX_ALLOCATION / 2), ("b", MAX_ALLOCATION / 2)],
        );
        account.rollover(Decimal::ZERO).unwrap();
        account.spend(&cats["a"], Decimal::from(1000u64)).unwrap();
        account.remove_category(&cats["a"]).unwrap();
        let state = account.snapshot();
        let surplus_a = Decimal::from(10000u64) * (MAX_ALLOCATION / 2) / MAX_ALLOCATION;
        assert_eq!(state.balance, surplus_a - Decimal::from(1000u64));
        assert!(!state.categories.contains_key(&cats["a"]));
    }

    #[test]
    fn transfer_surplus_moves_given_amount() {
        let (mut account, cats) = account_with(
            Decimal::from(10000u64),
            &[("a", MAX_ALLOCATION / 2), ("b", MAX_ALLOCATION / 4)],
        );
        account.rollover(Decimal::ZERO).unwrap();
        let surplus_a = Decimal::from(10000u64) * (MAX_ALLOCATION / 2) / MAX_ALLOCATION;
        let surplus_b = Decimal::from(10000u64) * (MAX_ALLOCATION / 4) / MAX_ALLOCATION;
        let amount = surplus_a / 2u8;
        account
            .transfer_surplus(&cats["a"], &cats["b"], amount)
            .unwrap();
        let state = account.snapshot();
        assert_eq!(
            state.categories.get(&cats["a"]).unwrap().surplus,
            surplus_a - amount
        );
        assert_eq!(
            state.categories.get(&cats["b"]).unwrap().surplus,
            surplus_b + amount
        );
        assert_eq!(
            state.balance,
            Decimal::from(10000u64) - (surplus_a + surplus_b)
        );
    }

    #[test]
    fn transfer_surplus_of_full_available_empties_source() {
        let (mut account, cats) = account_with(
            Decimal::from(10000u64),
            &[("a", MAX_ALLOCATION / 2), ("b", MAX_ALLOCATION / 4)],
        );
        account.rollover(Decimal::ZERO).unwrap();
        let surplus_a = Decimal::from(10000u64) * (MAX_ALLOCATION / 2) / MAX_ALLOCATION;
        let surplus_b = Decimal::from(10000u64) * (MAX_ALLOCATION / 4) / MAX_ALLOCATION;
        account
            .transfer_surplus(&cats["a"], &cats["b"], surplus_a)
            .unwrap();
        let state = account.snapshot();
        assert_eq!(
            state.categories.get(&cats["a"]).unwrap().surplus,
            Decimal::ZERO
        );
        assert_eq!(
            state.categories.get(&cats["b"]).unwrap().surplus,
            surplus_a + surplus_b
        );
    }

    #[test]
    fn reallocate_successful() {
        let (mut account, cats) = account_with(
            Decimal::from(10000u64),
            &[("a", MAX_ALLOCATION / 2), ("b", MAX_ALLOCATION / 2)],
        );
        let result = account.reallocate_categories(HashMap::from([
            (cats["a"], MAX_ALLOCATION / 4),
            (cats["b"], MAX_ALLOCATION - MAX_ALLOCATION / 4),
        ]));
        assert!(result.is_ok());
    }

    #[test]
    fn snapshot_reconstitute_roundtrip() {
        let (mut account, cats) = account_with(
            Decimal::from(10000u64),
            &[("a", MAX_ALLOCATION / 2), ("b", MAX_ALLOCATION / 4)],
        );
        account.spend(&cats["a"], Decimal::from(1000u64)).unwrap();
        account.rollover(Decimal::from(5000u64)).unwrap();
        account.spend(&cats["a"], Decimal::from(6000u64)).unwrap();
        let state = account.snapshot();
        let reconstituted = Account::reconstitute(state.clone(), None);
        assert_eq!(reconstituted.snapshot(), state);
    }

    #[test]
    fn reconstitute_applies_trailing_events() {
        let (mut account, cats) =
            account_with(Decimal::from(10000u64), &[("a", MAX_ALLOCATION / 4)]);
        let base = account.snapshot();
        account.spend(&cats["a"], Decimal::from(1000u64)).unwrap();
        account.rollover(Decimal::from(500u64)).unwrap();
        let final_state = account.snapshot();

        let replayed = Account::reconstitute(
            base,
            Some(vec![
                AccountEvent::Spent {
                    category: cats["a"],
                    amount: Decimal::from(1000u64),
                },
                AccountEvent::RolledOver {
                    income: Decimal::from(500u64),
                },
            ]),
        );
        assert_eq!(replayed.snapshot(), final_state);
    }

    #[test]
    fn reconstitute_replays_transfer_events() {
        let (mut account, cats) = account_with(
            Decimal::from(10000u64),
            &[("a", MAX_ALLOCATION / 2), ("b", MAX_ALLOCATION / 4)],
        );
        account.rollover(Decimal::ZERO).unwrap();
        let base = account.snapshot();
        let amount = Decimal::from(500u64);
        account
            .transfer_surplus(&cats["a"], &cats["b"], amount)
            .unwrap();
        let final_state = account.snapshot();

        let replayed = Account::reconstitute(
            base,
            Some(vec![AccountEvent::SurplusTransferred {
                from: cats["a"],
                to: cats["b"],
                amount,
            }]),
        );
        assert_eq!(replayed.snapshot(), final_state);
    }

    #[test]
    fn reconstitute_replays_events_without_recording() {
        let (account, _) = account_with(Decimal::from(1000u64), &[("a", 10)]);
        let snapshot = account.snapshot();
        let account = Account::reconstitute(
            snapshot,
            Some(vec![AccountEvent::FundsAdded {
                amount: Decimal::from(100u64),
            }]),
        );
        assert_eq!(account.snapshot().balance, Decimal::from(1100u64));
        assert!(account.events().is_empty());
    }

    #[test]
    fn spend_records_emitted_event() {
        let (mut account, cats) = account_with(Decimal::from(1000u64), &[("a", 10)]);
        account.spend(&cats["a"], Decimal::from(5u64)).unwrap();
        assert_eq!(
            account.events(),
            &[
                AccountEvent::FundsAdded {
                    amount: Decimal::from(1000u64),
                },
                AccountEvent::CategoryAdded {
                    id: cats["a"],
                    allocation: 10,
                },
                AccountEvent::Spent {
                    category: cats["a"],
                    amount: Decimal::from(5u64),
                },
            ]
        );
    }

    #[test]
    fn mark_committed_clears_recorded_events() {
        let (mut account, cats) = account_with(Decimal::from(1000u64), &[("a", 10)]);
        account.spend(&cats["a"], Decimal::from(5u64)).unwrap();
        let recorded = account.events().len();
        account.mark_committed(recorded);
        assert!(account.events().is_empty());
    }

    #[test]
    fn mark_committed_keeps_events_emitted_after_the_count() {
        let (mut account, cats) = account_with(Decimal::from(1000u64), &[("a", 10)]);
        account.spend(&cats["a"], Decimal::from(5u64)).unwrap();
        account.mark_committed(2);
        assert_eq!(
            account.events(),
            &[AccountEvent::Spent {
                category: cats["a"],
                amount: Decimal::from(5u64),
            }]
        );
    }

    #[test]
    fn repay_debt_adds_on_top_of_spent() {
        let (mut account, cats) =
            account_with(Decimal::from(10000u64), &[("a", MAX_ALLOCATION / 2)]);
        let loan = account.add_debt(Decimal::from(1000u64)).unwrap();
        account.spend(&cats["a"], Decimal::from(1000u64)).unwrap();
        account
            .repay_debt(&cats["a"], &loan, Decimal::from(400u64))
            .unwrap();
        let state = account.snapshot();
        assert_eq!(
            state.categories.get(&cats["a"]).unwrap().spent,
            Decimal::from(1400u64)
        );
        assert_eq!(state.debts.get(&loan), Some(&Decimal::from(600u64)));
        assert_eq!(state.balance, Decimal::from(10000u64));
    }

    #[test]
    fn repay_debt_in_full_removes_entry() {
        let (mut account, cats) =
            account_with(Decimal::from(10000u64), &[("a", MAX_ALLOCATION / 2)]);
        let loan = account.add_debt(Decimal::from(400u64)).unwrap();
        account
            .repay_debt(&cats["a"], &loan, Decimal::from(400u64))
            .unwrap();
        let state = account.snapshot();
        assert!(!state.debts.contains_key(&loan));
        assert_eq!(
            state.categories.get(&cats["a"]).unwrap().spent,
            Decimal::from(400u64)
        );
    }

    #[test]
    fn debt_snapshot_reconstitute_roundtrip() {
        let (mut account, cats) =
            account_with(Decimal::from(10000u64), &[("a", MAX_ALLOCATION / 2)]);
        let loan = account.add_debt(Decimal::from(1000u64)).unwrap();
        account
            .repay_debt(&cats["a"], &loan, Decimal::from(400u64))
            .unwrap();
        let state = account.snapshot();
        let reconstituted = Account::reconstitute(state.clone(), None);
        assert_eq!(reconstituted.snapshot(), state);
    }

    #[test]
    fn reconstitute_replays_debt_events() {
        let (mut account, cats) =
            account_with(Decimal::from(10000u64), &[("a", MAX_ALLOCATION / 2)]);
        let base = account.snapshot();
        let loan = account.add_debt(Decimal::from(1000u64)).unwrap();
        account
            .repay_debt(&cats["a"], &loan, Decimal::from(400u64))
            .unwrap();
        let final_state = account.snapshot();

        let replayed = Account::reconstitute(
            base,
            Some(vec![
                AccountEvent::DebtAdded {
                    id: loan,
                    amount: Decimal::from(1000u64),
                },
                AccountEvent::DebtRepaid {
                    category: cats["a"],
                    debt: loan,
                    amount: Decimal::from(400u64),
                },
            ]),
        );
        assert_eq!(replayed.snapshot(), final_state);
    }
}
