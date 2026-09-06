use std::collections::HashMap;

use thiserror::Error;

use crate::types::Decimal;

const MAX_ALLOCATION: u8 = 200;

fn budget(balance: Decimal, allocation: u8) -> Decimal {
    balance * allocation / MAX_ALLOCATION
}

#[derive(Clone, Debug)]
pub struct Account {
    balance: Decimal,
    categories: HashMap<String, Spending>,
    events: Vec<AccountEvent>,
}

#[derive(Clone, Debug)]
struct Spending {
    allocation: u8,
    spent: Decimal,
    surplus: Decimal,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AccountState {
    pub balance: Decimal,
    pub categories: HashMap<String, CategoryState>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CategoryState {
    pub allocation: u8,
    pub spent: Decimal,
    pub surplus: Decimal,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AccountEvent {
    CategoryAdded { name: String, allocation: u8 },
    CategoryRemoved { name: String },
    RolledOver { income: Decimal },
    Spent { category: String, amount: Decimal },
    SpentConsumed { category: String },
    SurplusConsumed { category: String },
    SurplusTransferred { from: String, to: String },
    FundsAdded { amount: Decimal },
    FundsRemoved { amount: Decimal },
    CategoriesReallocated { allocations: HashMap<String, u8> },
}

impl Account {
    pub fn add_category(&mut self, name: &str, allocation: u8) -> Result<(), AccountError> {
        preconditions::check_category_absent(name, &self.categories)?;
        Spending::new(allocation, Decimal::ZERO, Decimal::ZERO)?;
        invariants::check_total_allocations(
            self.categories
                .values()
                .map(|spending| spending.allocation)
                .chain(std::iter::once(allocation)),
        )?;
        self.emit(AccountEvent::CategoryAdded {
            name: name.to_string(),
            allocation,
        });
        Ok(())
    }

    pub fn remove_category(&mut self, name: &str) -> Result<(), AccountError> {
        preconditions::check_category_exists(name, &self.categories)?;
        let spending = self.categories.get(name).expect("existence checked");
        preconditions::check_category_idle(name, spending)?;
        self.emit(AccountEvent::CategoryRemoved {
            name: name.to_string(),
        });
        Ok(())
    }

    pub fn rollover(&mut self, income: Decimal) -> Result<(), AccountError> {
        let mut total_budget = Decimal::ZERO;
        let mut total_spent = Decimal::ZERO;
        for spending in self.categories.values() {
            let current = budget(self.balance, spending.allocation);
            total_budget += current;
            total_spent += spending.spent;
            invariants::check_negative_surplus(
                self.balance,
                spending.allocation,
                spending.surplus,
                spending.spent,
            )?;
        }
        invariants::check_negative_balance(income + self.balance + total_spent, total_budget)?;
        self.emit(AccountEvent::RolledOver { income });
        Ok(())
    }

    pub fn spend(&mut self, category: &str, amount: Decimal) -> Result<(), AccountError> {
        preconditions::check_category_exists(category, &self.categories)?;
        let spending = self.categories.get(category).expect("existence checked");
        invariants::check_spending_within_budget(
            category,
            self.balance,
            spending.allocation,
            spending.surplus,
            spending.spent + amount,
        )?;
        self.emit(AccountEvent::Spent {
            category: category.to_string(),
            amount,
        });
        Ok(())
    }

    pub fn consume_spent(&mut self, category: &str) -> Result<(), AccountError> {
        preconditions::check_category_exists(category, &self.categories)?;
        let spent = self
            .categories
            .get(category)
            .expect("existence checked")
            .spent;
        invariants::check_negative_balance(self.balance, spent)?;
        let balance = self.balance - spent;
        invariants::check_all_spending_within_budget(
            balance,
            self.categories
                .iter()
                .filter(|(name, _)| name.as_str() != category),
        )?;
        self.emit(AccountEvent::SpentConsumed {
            category: category.to_string(),
        });
        Ok(())
    }

    pub fn consume_surplus(&mut self, category: &str) -> Result<(), AccountError> {
        preconditions::check_category_exists(category, &self.categories)?;
        let spending = self.categories.get(category).expect("existence checked");
        invariants::check_spending_within_budget(
            category,
            self.balance + spending.surplus,
            spending.allocation,
            Decimal::ZERO,
            spending.spent,
        )?;
        self.emit(AccountEvent::SurplusConsumed {
            category: category.to_string(),
        });
        Ok(())
    }

    pub fn transfer_surplus(&mut self, from: &str, to: &str) -> Result<(), AccountError> {
        preconditions::check_distinct_source_target(from, to)?;
        preconditions::check_source_exists(from, &self.categories)?;
        preconditions::check_target_exists(to, &self.categories)?;
        let source = self.categories.get(from).expect("source checked");
        invariants::check_spending_within_budget(
            from,
            self.balance,
            source.allocation,
            Decimal::ZERO,
            source.spent,
        )?;
        self.emit(AccountEvent::SurplusTransferred {
            from: from.to_string(),
            to: to.to_string(),
        });
        Ok(())
    }

    pub fn add_funds(&mut self, amount: Decimal) -> Result<(), AccountError> {
        self.emit(AccountEvent::FundsAdded { amount });
        Ok(())
    }

    pub fn remove_funds(&mut self, amount: Decimal) -> Result<(), AccountError> {
        invariants::check_negative_balance(self.balance, amount)?;
        let balance = self.balance - amount;
        invariants::check_all_spending_within_budget(balance, &self.categories)?;
        self.emit(AccountEvent::FundsRemoved { amount });
        Ok(())
    }

    pub fn reallocate_categories(
        &mut self,
        allocations: HashMap<String, u8>,
    ) -> Result<(), AccountError> {
        preconditions::check_categories_match(&self.categories, &allocations)?;
        invariants::check_total_allocations(allocations.values().copied())?;
        for (name, allocation) in &allocations {
            let existing = self.categories.get(name).expect("keys matched");
            Spending::new(*allocation, existing.spent, existing.surplus)?;
            invariants::check_spending_within_budget(
                name,
                self.balance,
                *allocation,
                existing.surplus,
                existing.spent,
            )?;
        }
        self.emit(AccountEvent::CategoriesReallocated { allocations });
        Ok(())
    }

    pub fn snapshot(&self) -> AccountState {
        AccountState {
            balance: self.balance,
            categories: self
                .categories
                .iter()
                .map(|(name, spending)| {
                    (
                        name.clone(),
                        CategoryState {
                            allocation: spending.allocation,
                            spent: spending.spent,
                            surplus: spending.surplus,
                        },
                    )
                })
                .collect(),
        }
    }

    pub fn reconstitute(snapshot: AccountState, events: Option<Vec<AccountEvent>>) -> Account {
        let mut account = Account {
            balance: snapshot.balance,
            categories: snapshot
                .categories
                .into_iter()
                .map(|(name, category)| {
                    (
                        name,
                        Spending {
                            allocation: category.allocation,
                            spent: category.spent,
                            surplus: category.surplus,
                        },
                    )
                })
                .collect(),
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
            AccountEvent::CategoryAdded { name, allocation } => {
                self.categories.insert(
                    name.clone(),
                    Spending {
                        allocation: *allocation,
                        spent: Decimal::ZERO,
                        surplus: Decimal::ZERO,
                    },
                );
            }
            AccountEvent::CategoryRemoved { name } => {
                self.categories.remove(name);
            }
            AccountEvent::RolledOver { income } => {
                let balance = self.balance;
                let mut total_budget = Decimal::ZERO;
                let mut total_spent = Decimal::ZERO;
                for spending in self.categories.values_mut() {
                    let current = budget(balance, spending.allocation);
                    total_budget += current;
                    total_spent += spending.spent;
                    spending.surplus = spending.surplus + current - spending.spent;
                    spending.spent = Decimal::ZERO;
                }
                self.balance = *income + ((balance + total_spent) - total_budget);
            }
            AccountEvent::Spent { category, amount } => {
                self.categories
                    .get_mut(category)
                    .expect("validated event")
                    .spent += *amount;
            }
            AccountEvent::SpentConsumed { category } => {
                let spending = self.categories.get_mut(category).expect("validated event");
                let spent = spending.spent;
                spending.spent = Decimal::ZERO;
                self.balance -= spent;
            }
            AccountEvent::SurplusConsumed { category } => {
                let spending = self.categories.get_mut(category).expect("validated event");
                let surplus = spending.surplus;
                spending.surplus = Decimal::ZERO;
                self.balance += surplus;
            }
            AccountEvent::SurplusTransferred { from, to } => {
                let surplus = self.categories.get(from).expect("validated event").surplus;
                self.categories
                    .get_mut(from)
                    .expect("validated event")
                    .surplus = Decimal::ZERO;
                self.categories
                    .get_mut(to)
                    .expect("validated event")
                    .surplus += surplus;
            }
            AccountEvent::FundsAdded { amount } => {
                self.balance += *amount;
            }
            AccountEvent::FundsRemoved { amount } => {
                self.balance -= *amount;
            }
            AccountEvent::CategoriesReallocated { allocations } => {
                for (name, allocation) in allocations {
                    self.categories
                        .get_mut(name)
                        .expect("validated event")
                        .allocation = *allocation;
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
            balance: Decimal::ZERO,
            categories: HashMap::new(),
            events: Vec::new(),
        }
    }
}

impl Spending {
    fn new(
        allocation: u8,
        spent: Decimal,
        surplus: Decimal,
    ) -> Result<Self, AccountInvariantError> {
        invariants::check_allocation_limit(allocation)?;
        Ok(Self {
            allocation,
            spent,
            surplus,
        })
    }
}

mod invariants {
    use super::{AccountInvariantError, Decimal, MAX_ALLOCATION, Spending, budget};

    pub(super) fn check_negative_balance(
        balance: Decimal,
        deduction: Decimal,
    ) -> Result<(), AccountInvariantError> {
        if balance.safe_sub(deduction).is_err() {
            return Err(AccountInvariantError::NegativeBalance);
        }
        Ok(())
    }

    pub(super) fn check_total_allocations(
        allocations: impl Iterator<Item = u8>,
    ) -> Result<(), AccountInvariantError> {
        let total = allocations.map(u32::from).sum::<u32>();
        if total > u32::from(MAX_ALLOCATION) {
            return Err(AccountInvariantError::TotalAllocations(total));
        }
        Ok(())
    }

    pub(super) fn check_spending_within_budget(
        name: &str,
        balance: Decimal,
        allocation: u8,
        surplus: Decimal,
        spent: Decimal,
    ) -> Result<(), AccountInvariantError> {
        if spent > budget(balance, allocation) + surplus {
            return Err(AccountInvariantError::SpendingExceedsAllocation(
                name.to_string(),
                spent,
                allocation,
                surplus,
            ));
        }
        Ok(())
    }

    pub(super) fn check_all_spending_within_budget<'a>(
        balance: Decimal,
        categories: impl IntoIterator<Item = (&'a String, &'a Spending)>,
    ) -> Result<(), AccountInvariantError> {
        for (name, spending) in categories {
            check_spending_within_budget(
                name,
                balance,
                spending.allocation,
                spending.surplus,
                spending.spent,
            )?;
        }
        Ok(())
    }

    pub(super) fn check_allocation_limit(allocation: u8) -> Result<(), AccountInvariantError> {
        if allocation > MAX_ALLOCATION {
            return Err(AccountInvariantError::AllocationLimit(allocation));
        }
        Ok(())
    }

    pub(super) fn check_negative_surplus(
        balance: Decimal,
        allocation: u8,
        surplus: Decimal,
        spent: Decimal,
    ) -> Result<(), AccountInvariantError> {
        let covered = surplus
            .safe_add(budget(balance, allocation))
            .and_then(|total| total.safe_sub(spent));
        if covered.is_err() {
            return Err(AccountInvariantError::NegativeSurplus);
        }
        Ok(())
    }
}

mod preconditions {
    use std::collections::HashMap;

    use super::{AccountPreconditionError, Spending};

    pub(super) fn check_category_exists(
        name: &str,
        categories: &HashMap<String, Spending>,
    ) -> Result<(), AccountPreconditionError> {
        if !categories.contains_key(name) {
            return Err(AccountPreconditionError::CategoryNotFound(name.to_string()));
        }
        Ok(())
    }

    pub(super) fn check_category_absent(
        name: &str,
        categories: &HashMap<String, Spending>,
    ) -> Result<(), AccountPreconditionError> {
        if categories.contains_key(name) {
            return Err(AccountPreconditionError::DuplicateCategory(
                name.to_string(),
            ));
        }
        Ok(())
    }

    pub(super) fn check_category_idle(
        name: &str,
        spending: &Spending,
    ) -> Result<(), AccountPreconditionError> {
        if !spending.spent.is_zero() {
            return Err(AccountPreconditionError::CategoryHasSpending(
                name.to_string(),
            ));
        }
        if !spending.surplus.is_zero() {
            return Err(AccountPreconditionError::CategoryHasSurplus(
                name.to_string(),
            ));
        }
        Ok(())
    }

    pub(super) fn check_distinct_source_target(
        from: &str,
        to: &str,
    ) -> Result<(), AccountPreconditionError> {
        if from == to {
            return Err(AccountPreconditionError::SameCategory(from.to_string()));
        }
        Ok(())
    }

    pub(super) fn check_source_exists(
        from: &str,
        categories: &HashMap<String, Spending>,
    ) -> Result<(), AccountPreconditionError> {
        if !categories.contains_key(from) {
            return Err(AccountPreconditionError::SourceCategoryNotFound(
                from.to_string(),
            ));
        }
        Ok(())
    }

    pub(super) fn check_target_exists(
        to: &str,
        categories: &HashMap<String, Spending>,
    ) -> Result<(), AccountPreconditionError> {
        if !categories.contains_key(to) {
            return Err(AccountPreconditionError::TargetCategoryNotFound(
                to.to_string(),
            ));
        }
        Ok(())
    }

    pub(super) fn check_categories_match(
        categories: &HashMap<String, Spending>,
        allocations: &HashMap<String, u8>,
    ) -> Result<(), AccountPreconditionError> {
        if categories.len() != allocations.len()
            || !categories.keys().all(|name| allocations.contains_key(name))
        {
            return Err(AccountPreconditionError::UnmatchedCategories);
        }
        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum AccountError {
    #[error(transparent)]
    Precondition(#[from] AccountPreconditionError),

    #[error(transparent)]
    Invariant(#[from] AccountInvariantError),
}

#[derive(Error, Debug)]
pub enum AccountPreconditionError {
    #[error("category {0} does not exist")]
    CategoryNotFound(String),

    #[error("duplicate category: {0}")]
    DuplicateCategory(String),

    #[error("category {0} still has spending")]
    CategoryHasSpending(String),

    #[error("category {0} still has surplus")]
    CategoryHasSurplus(String),

    #[error("transfer source and target are the same: {0}")]
    SameCategory(String),

    #[error("transfer source {0} does not exist")]
    SourceCategoryNotFound(String),

    #[error("transfer target {0} does not exist")]
    TargetCategoryNotFound(String),

    #[error("requested list of categories do not match existing ones")]
    UnmatchedCategories,
}

#[derive(Error, Debug)]
pub enum AccountInvariantError {
    #[error("negative balance")]
    NegativeBalance,

    #[error("total allocation ({0}) exceeds the limit ({MAX_ALLOCATION})")]
    TotalAllocations(u32),

    #[error("category {0} spending ({1}) exceeds the allocation ({2}) plus surplus ({3})")]
    SpendingExceedsAllocation(String, Decimal, u8, Decimal),

    #[error("allocation ({0}) exceeds the limit ({MAX_ALLOCATION})")]
    AllocationLimit(u8),

    #[error("surplus would be negative")]
    NegativeSurplus,
}

#[cfg(test)]
mod account_tests {
    use super::*;
    use std::collections::HashMap;

    fn account_with(balance: Decimal, allocations: &[(&str, u8)]) -> Account {
        let mut account = Account::default();
        account.add_funds(balance).unwrap();
        for (name, allocation) in allocations {
            account.add_category(name, *allocation).unwrap();
        }
        account
    }

    #[test]
    fn default_is_empty() {
        let state = Account::default().snapshot();
        assert_eq!(state.balance, Decimal::ZERO);
        assert!(state.categories.is_empty());
    }

    #[test]
    fn add_category_succeeds() {
        let mut account = account_with(Decimal::from(1000u64), &[]);
        account.add_category("test", 10).unwrap();
        let state = account.snapshot();
        assert_eq!(state.balance, Decimal::from(1000u64));
        assert_eq!(
            state.categories.get("test").unwrap(),
            &CategoryState {
                allocation: 10,
                spent: Decimal::ZERO,
                surplus: Decimal::ZERO,
            }
        );
    }

    #[test]
    fn remove_category_succeeds() {
        let mut account = account_with(Decimal::from(1000u64), &[("test", 10)]);
        account.remove_category("test").unwrap();
        let state = account.snapshot();
        assert_eq!(state.balance, Decimal::from(1000u64));
        assert!(state.categories.is_empty());
    }

    #[test]
    fn remove_after_transferring_surplus() {
        let mut account = account_with(
            Decimal::from(10000u64),
            &[("a", MAX_ALLOCATION / 2), ("b", MAX_ALLOCATION / 4)],
        );
        account.rollover(Decimal::ZERO).unwrap();
        account.transfer_surplus("a", "b").unwrap();
        account.remove_category("a").unwrap();
        let state = account.snapshot();
        assert!(!state.categories.contains_key("a"));
        let surplus_b = Decimal::from(10000u64) * (MAX_ALLOCATION / 4) / MAX_ALLOCATION
            + Decimal::from(10000u64) * (MAX_ALLOCATION / 2) / MAX_ALLOCATION;
        assert_eq!(state.categories.get("b").unwrap().surplus, surplus_b);
    }

    #[test]
    fn rollover_flushes_balance_into_surplus_and_resets_spent() {
        let mut account = account_with(
            Decimal::from(10000u64),
            &[("a", MAX_ALLOCATION / 2), ("b", MAX_ALLOCATION / 4)],
        );
        account.spend("a", Decimal::from(1000u64)).unwrap();
        account.rollover(Decimal::from(5000u64)).unwrap();
        let state = account.snapshot();
        let budget_a = Decimal::from(10000u64) * (MAX_ALLOCATION / 2) / MAX_ALLOCATION;
        let budget_b = Decimal::from(10000u64) * (MAX_ALLOCATION / 4) / MAX_ALLOCATION;
        assert_eq!(
            state.categories.get("a").unwrap().surplus,
            budget_a - Decimal::from(1000u64)
        );
        assert_eq!(state.categories.get("b").unwrap().surplus, budget_b);
        assert_eq!(state.categories.get("a").unwrap().spent, Decimal::ZERO);
        assert_eq!(
            state.balance,
            Decimal::from(5000u64)
                + (Decimal::from(10000u64) - (budget_a - Decimal::from(1000u64) + budget_b))
        );
    }

    #[test]
    fn rollover_accumulates_existing_surplus() {
        let mut account = account_with(Decimal::from(10000u64), &[("a", MAX_ALLOCATION / 2)]);
        account.rollover(Decimal::ZERO).unwrap();
        account.spend("a", Decimal::from(2000u64)).unwrap();
        account.rollover(Decimal::ZERO).unwrap();
        let state = account.snapshot();
        let first_budget = Decimal::from(10000u64) * (MAX_ALLOCATION / 2) / MAX_ALLOCATION;
        let second_budget =
            (Decimal::from(10000u64) - first_budget) * (MAX_ALLOCATION / 2) / MAX_ALLOCATION;
        let increment = second_budget - Decimal::from(2000u64);
        assert_eq!(
            state.categories.get("a").unwrap().surplus,
            first_budget + increment
        );
        assert_eq!(state.categories.get("a").unwrap().spent, Decimal::ZERO);
        assert_eq!(
            state.balance,
            Decimal::from(10000u64) - first_budget - increment
        );
    }

    #[test]
    fn consume_spent_settles_into_balance() {
        let mut account = account_with(Decimal::from(10000u64), &[("a", MAX_ALLOCATION / 4)]);
        account.spend("a", Decimal::from(1000u64)).unwrap();
        account.consume_spent("a").unwrap();
        let state = account.snapshot();
        assert_eq!(state.balance, Decimal::from(9000u64));
        assert_eq!(state.categories.get("a").unwrap().spent, Decimal::ZERO);
    }

    #[test]
    fn consume_surplus_returns_to_balance() {
        let mut account = account_with(Decimal::from(10000u64), &[("a", MAX_ALLOCATION / 2)]);
        account.rollover(Decimal::ZERO).unwrap();
        account.consume_surplus("a").unwrap();
        let state = account.snapshot();
        let surplus = Decimal::from(10000u64) * (MAX_ALLOCATION / 2) / MAX_ALLOCATION;
        assert_eq!(state.balance, Decimal::from(10000u64) - surplus + surplus);
        assert_eq!(state.categories.get("a").unwrap().surplus, Decimal::ZERO);
    }

    #[test]
    fn transfer_surplus_moves_entire_surplus() {
        let mut account = account_with(
            Decimal::from(10000u64),
            &[("a", MAX_ALLOCATION / 2), ("b", MAX_ALLOCATION / 4)],
        );
        account.rollover(Decimal::ZERO).unwrap();
        account.transfer_surplus("a", "b").unwrap();
        let state = account.snapshot();
        let surplus_a = Decimal::from(10000u64) * (MAX_ALLOCATION / 2) / MAX_ALLOCATION;
        let surplus_b = Decimal::from(10000u64) * (MAX_ALLOCATION / 4) / MAX_ALLOCATION;
        assert_eq!(state.categories.get("a").unwrap().surplus, Decimal::ZERO);
        assert_eq!(
            state.categories.get("b").unwrap().surplus,
            surplus_a + surplus_b
        );
        assert_eq!(
            state.balance,
            Decimal::from(10000u64) - (surplus_a + surplus_b)
        );
    }

    #[test]
    fn reallocate_successful() {
        let mut account = account_with(
            Decimal::from(10000u64),
            &[("a", MAX_ALLOCATION / 2), ("b", MAX_ALLOCATION / 2)],
        );
        let result = account.reallocate_categories(HashMap::from([
            ("a".to_string(), MAX_ALLOCATION / 4),
            ("b".to_string(), MAX_ALLOCATION - MAX_ALLOCATION / 4),
        ]));
        assert!(result.is_ok());
    }

    #[test]
    fn snapshot_reconstitute_roundtrip() {
        let mut account = account_with(
            Decimal::from(10000u64),
            &[("a", MAX_ALLOCATION / 2), ("b", MAX_ALLOCATION / 4)],
        );
        account.spend("a", Decimal::from(1000u64)).unwrap();
        account.rollover(Decimal::from(5000u64)).unwrap();
        account.spend("a", Decimal::from(6000u64)).unwrap();
        let state = account.snapshot();
        let reconstituted = Account::reconstitute(state.clone(), None);
        assert_eq!(reconstituted.snapshot(), state);
    }

    #[test]
    fn reconstitute_applies_trailing_events() {
        let mut account = account_with(Decimal::from(10000u64), &[("a", MAX_ALLOCATION / 4)]);
        let base = account.snapshot();
        account.spend("a", Decimal::from(1000u64)).unwrap();
        account.rollover(Decimal::from(500u64)).unwrap();
        let final_state = account.snapshot();

        let replayed = Account::reconstitute(
            base,
            Some(vec![
                AccountEvent::Spent {
                    category: "a".to_string(),
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
    fn reconstitute_replays_events_without_recording() {
        let snapshot = account_with(Decimal::from(1000u64), &[("a", 10)]).snapshot();
        let account = Account::reconstitute(
            snapshot,
            Some(vec![AccountEvent::FundsAdded {
                amount: Decimal::from(100u64),
            }]),
        );
        assert_eq!(account.snapshot().balance, Decimal::from(1100u64));
        assert!(account.events.is_empty());
    }

    #[test]
    fn spend_records_emitted_event() {
        let mut account = account_with(Decimal::from(1000u64), &[("a", 10)]);
        account.spend("a", Decimal::from(5u64)).unwrap();
        assert_eq!(
            account.events,
            vec![
                AccountEvent::FundsAdded {
                    amount: Decimal::from(1000u64),
                },
                AccountEvent::CategoryAdded {
                    name: "a".to_string(),
                    allocation: 10,
                },
                AccountEvent::Spent {
                    category: "a".to_string(),
                    amount: Decimal::from(5u64),
                },
            ]
        );
    }
}

#[cfg(test)]
mod invariants_tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn deduction_exceeding_balance_fails() {
        assert!(matches!(
            invariants::check_negative_balance(Decimal::from(5u64), Decimal::from(10u64)),
            Err(AccountInvariantError::NegativeBalance)
        ));
    }

    #[test]
    fn exact_deduction_passes() {
        assert!(
            invariants::check_negative_balance(Decimal::from(5u64), Decimal::from(5u64)).is_ok()
        );
    }

    #[test]
    fn total_allocations_at_limit_passes() {
        assert!(invariants::check_total_allocations([MAX_ALLOCATION].into_iter()).is_ok());
    }

    #[test]
    fn total_allocations_over_limit_fails() {
        let total = u32::from(MAX_ALLOCATION) + 1;
        assert!(matches!(
            invariants::check_total_allocations([MAX_ALLOCATION, 1].into_iter()),
            Err(AccountInvariantError::TotalAllocations(t)) if t == total
        ));
    }

    #[test]
    fn spending_dipping_into_surplus_passes() {
        let balance = Decimal::from(1000u64);
        let allocation = MAX_ALLOCATION / 2;
        let spent = budget(balance, allocation) + Decimal::from(500u64);
        assert!(
            invariants::check_spending_within_budget(
                "a",
                balance,
                allocation,
                Decimal::from(500u64),
                spent
            )
            .is_ok()
        );
    }

    #[test]
    fn spending_exceeding_budget_and_surplus_fails() {
        let balance = Decimal::from(1000u64);
        let allocation = MAX_ALLOCATION / 2;
        let spent = budget(balance, allocation) + Decimal::from(500u64) + Decimal::new(1_000_000);
        assert!(matches!(
            invariants::check_spending_within_budget(
                "a",
                balance,
                allocation,
                Decimal::from(500u64),
                spent
            ),
            Err(AccountInvariantError::SpendingExceedsAllocation(name, _, _, _)) if name == "a"
        ));
    }

    #[test]
    fn all_spending_within_budget_passes() {
        let categories = HashMap::from([
            (
                "a".to_string(),
                Spending {
                    allocation: MAX_ALLOCATION / 2,
                    spent: Decimal::ZERO,
                    surplus: Decimal::ZERO,
                },
            ),
            (
                "b".to_string(),
                Spending {
                    allocation: MAX_ALLOCATION / 2,
                    spent: Decimal::ZERO,
                    surplus: Decimal::ZERO,
                },
            ),
        ]);
        assert!(
            invariants::check_all_spending_within_budget(Decimal::from(1000u64), &categories)
                .is_ok()
        );
    }

    #[test]
    fn all_spending_checks_each_category() {
        let categories = HashMap::from([
            (
                "a".to_string(),
                Spending {
                    allocation: MAX_ALLOCATION / 2,
                    spent: Decimal::ZERO,
                    surplus: Decimal::ZERO,
                },
            ),
            (
                "b".to_string(),
                Spending {
                    allocation: MAX_ALLOCATION / 2,
                    spent: budget(Decimal::from(1000u64), MAX_ALLOCATION / 2) + Decimal::from(1u64),
                    surplus: Decimal::ZERO,
                },
            ),
        ]);
        assert!(matches!(
            invariants::check_all_spending_within_budget(Decimal::from(1000u64), &categories),
            Err(AccountInvariantError::SpendingExceedsAllocation(name, _, _, _)) if name == "b"
        ));
    }

    #[test]
    fn allocation_at_limit_passes() {
        assert!(invariants::check_allocation_limit(MAX_ALLOCATION).is_ok());
    }

    #[test]
    fn allocation_over_limit_fails() {
        assert!(matches!(
            invariants::check_allocation_limit(MAX_ALLOCATION + 1),
            Err(AccountInvariantError::AllocationLimit(_))
        ));
    }

    #[test]
    fn surplus_plus_budget_covering_spending_passes() {
        assert!(
            invariants::check_negative_surplus(
                Decimal::from(10000u64),
                MAX_ALLOCATION / 2,
                Decimal::from(500u64),
                Decimal::from(4000u64),
            )
            .is_ok()
        );
    }

    #[test]
    fn spending_exactly_depleting_surplus_passes() {
        assert!(
            invariants::check_negative_surplus(
                Decimal::from(10000u64),
                MAX_ALLOCATION / 2,
                Decimal::from(500u64),
                Decimal::from(5500u64),
            )
            .is_ok()
        );
    }

    #[test]
    fn spending_exceeding_surplus_plus_budget_fails() {
        assert!(matches!(
            invariants::check_negative_surplus(
                Decimal::from(10000u64),
                MAX_ALLOCATION / 2,
                Decimal::from(500u64),
                Decimal::from(5500u64) + Decimal::new(1),
            ),
            Err(AccountInvariantError::NegativeSurplus)
        ));
    }
}

#[cfg(test)]
mod preconditions_tests {
    use super::*;
    use std::collections::HashMap;

    fn categories(names: &[&str]) -> HashMap<String, Spending> {
        names
            .iter()
            .map(|name| {
                (
                    name.to_string(),
                    Spending {
                        allocation: 10,
                        spent: Decimal::ZERO,
                        surplus: Decimal::ZERO,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn missing_category_fails() {
        let categories = categories(&["a"]);
        assert!(matches!(
            preconditions::check_category_exists("b", &categories),
            Err(AccountPreconditionError::CategoryNotFound(name)) if name == "b"
        ));
    }

    #[test]
    fn existing_category_passes() {
        let categories = categories(&["a"]);
        assert!(preconditions::check_category_exists("a", &categories).is_ok());
    }

    #[test]
    fn duplicate_category_fails() {
        let categories = categories(&["a"]);
        assert!(matches!(
            preconditions::check_category_absent("a", &categories),
            Err(AccountPreconditionError::DuplicateCategory(name)) if name == "a"
        ));
    }

    #[test]
    fn absent_category_passes() {
        let categories = categories(&["a"]);
        assert!(preconditions::check_category_absent("b", &categories).is_ok());
    }

    #[test]
    fn category_with_spending_fails() {
        let spending = Spending {
            allocation: 10,
            spent: Decimal::from(1u64),
            surplus: Decimal::ZERO,
        };
        assert!(matches!(
            preconditions::check_category_idle("a", &spending),
            Err(AccountPreconditionError::CategoryHasSpending(name)) if name == "a"
        ));
    }

    #[test]
    fn category_with_surplus_fails() {
        let spending = Spending {
            allocation: 10,
            spent: Decimal::ZERO,
            surplus: Decimal::from(1u64),
        };
        assert!(matches!(
            preconditions::check_category_idle("a", &spending),
            Err(AccountPreconditionError::CategoryHasSurplus(name)) if name == "a"
        ));
    }

    #[test]
    fn idle_category_passes() {
        let spending = Spending {
            allocation: 10,
            spent: Decimal::ZERO,
            surplus: Decimal::ZERO,
        };
        assert!(preconditions::check_category_idle("a", &spending).is_ok());
    }

    #[test]
    fn same_source_and_target_fails() {
        assert!(matches!(
            preconditions::check_distinct_source_target("a", "a"),
            Err(AccountPreconditionError::SameCategory(name)) if name == "a"
        ));
    }

    #[test]
    fn distinct_source_and_target_passes() {
        assert!(preconditions::check_distinct_source_target("a", "b").is_ok());
    }

    #[test]
    fn missing_transfer_source_fails() {
        let categories = categories(&["b"]);
        assert!(matches!(
            preconditions::check_source_exists("a", &categories),
            Err(AccountPreconditionError::SourceCategoryNotFound(name)) if name == "a"
        ));
    }

    #[test]
    fn missing_transfer_target_fails() {
        let categories = categories(&["a"]);
        assert!(matches!(
            preconditions::check_target_exists("b", &categories),
            Err(AccountPreconditionError::TargetCategoryNotFound(name)) if name == "b"
        ));
    }

    #[test]
    fn mismatched_allocation_count_fails() {
        let categories = categories(&["a"]);
        let allocations = HashMap::from([("a".to_string(), 10), ("b".to_string(), 10)]);
        assert!(matches!(
            preconditions::check_categories_match(&categories, &allocations),
            Err(AccountPreconditionError::UnmatchedCategories)
        ));
    }

    #[test]
    fn mismatched_allocation_names_fails() {
        let categories = categories(&["a"]);
        let allocations = HashMap::from([("b".to_string(), 10)]);
        assert!(matches!(
            preconditions::check_categories_match(&categories, &allocations),
            Err(AccountPreconditionError::UnmatchedCategories)
        ));
    }

    #[test]
    fn matching_allocations_pass() {
        let categories = categories(&["a", "b"]);
        let allocations = HashMap::from([("a".to_string(), 10), ("b".to_string(), 20)]);
        assert!(preconditions::check_categories_match(&categories, &allocations).is_ok());
    }
}

#[cfg(test)]
mod spending_tests {
    use super::*;

    #[test]
    fn new_cant_exceed_max_allocation() {
        let result = Spending::new(MAX_ALLOCATION + 1, Decimal::ZERO, Decimal::ZERO);
        assert!(matches!(
            result,
            Err(AccountInvariantError::AllocationLimit(_))
        ));
    }
}
