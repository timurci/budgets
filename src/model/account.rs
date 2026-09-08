use std::collections::HashMap;

use thiserror::Error;

use crate::types::Decimal;

const MAX_ALLOCATION: u8 = 200;

fn budget(balance: Decimal, allocation: u8) -> Decimal {
    balance * allocation / MAX_ALLOCATION
}

fn total_budget(balance: Decimal, categories: &HashMap<String, Spending>) -> Decimal {
    categories
        .values()
        .map(|spending| budget(balance, spending.allocation))
        .sum()
}

fn removal_balance(balance: Decimal, surplus: Decimal, spent: Decimal) -> Decimal {
    (balance + surplus) - spent
}

#[derive(Clone, Debug)]
pub struct Account {
    balance: Decimal,
    categories: HashMap<String, Spending>,
    debts: HashMap<String, Decimal>,
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
    pub debts: HashMap<String, Decimal>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CategoryState {
    pub allocation: u8,
    pub spent: Decimal,
    pub surplus: Decimal,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AccountEvent {
    CategoryAdded {
        name: String,
        allocation: u8,
    },
    CategoryRemoved {
        name: String,
    },
    RolledOver {
        income: Decimal,
    },
    Spent {
        category: String,
        amount: Decimal,
    },
    SurplusTransferred {
        from: String,
        to: String,
    },
    FundsAdded {
        amount: Decimal,
    },
    FundsRemoved {
        amount: Decimal,
    },
    CategoriesReallocated {
        allocations: HashMap<String, u8>,
    },
    DebtAdded {
        name: String,
        amount: Decimal,
    },
    DebtRepaid {
        category: String,
        debt: String,
        amount: Decimal,
    },
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
        let funded = self.balance + spending.surplus;
        invariants::check_negative_balance(funded, spending.spent)?;
        let balance = removal_balance(self.balance, spending.surplus, spending.spent);
        invariants::check_all_spending_within_budget(
            balance,
            self.categories
                .iter()
                .filter(|(existing, _)| existing.as_str() != name),
        )?;
        self.emit(AccountEvent::CategoryRemoved {
            name: name.to_string(),
        });
        Ok(())
    }

    pub fn rollover(&mut self, income: Decimal) -> Result<(), AccountError> {
        for spending in self.categories.values() {
            invariants::check_negative_surplus(
                self.balance,
                spending.allocation,
                spending.surplus,
                spending.spent,
            )?;
        }
        invariants::check_negative_balance(
            income + self.balance,
            total_budget(self.balance, &self.categories),
        )?;
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

    pub fn add_debt(&mut self, name: &str, amount: Decimal) -> Result<(), AccountError> {
        preconditions::check_debt_absent(name, &self.debts)?;
        self.emit(AccountEvent::DebtAdded {
            name: name.to_string(),
            amount,
        });
        Ok(())
    }

    pub fn repay_debt(
        &mut self,
        category: &str,
        debt: &str,
        amount: Decimal,
    ) -> Result<(), AccountError> {
        preconditions::check_category_exists(category, &self.categories)?;
        preconditions::check_debt_exists(debt, &self.debts)?;
        let owed = self.debts.get(debt).expect("debt existence checked");
        invariants::check_debt_within_owed(debt, *owed, amount)?;
        let spending = self.categories.get(category).expect("existence checked");
        invariants::check_spending_within_budget(
            category,
            self.balance,
            spending.allocation,
            spending.surplus,
            spending.spent + amount,
        )?;
        self.emit(AccountEvent::DebtRepaid {
            category: category.to_string(),
            debt: debt.to_string(),
            amount,
        });
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
            debts: self.debts.clone(),
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
                let spending = self.categories.remove(name).expect("validated event");
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
            AccountEvent::DebtAdded { name, amount } => {
                self.debts.insert(name.clone(), *amount);
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
                    self.debts.insert(debt.clone(), remaining);
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
            debts: HashMap::new(),
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

    pub(super) fn check_debt_within_owed(
        name: &str,
        owed: Decimal,
        amount: Decimal,
    ) -> Result<(), AccountInvariantError> {
        if owed.safe_sub(amount).is_err() {
            return Err(AccountInvariantError::DebtOverpayment(
                name.to_string(),
                owed,
                amount,
            ));
        }
        Ok(())
    }
}

mod preconditions {
    use std::collections::HashMap;

    use super::{AccountPreconditionError, Decimal, Spending};

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

    pub(super) fn check_debt_absent(
        name: &str,
        debts: &HashMap<String, Decimal>,
    ) -> Result<(), AccountPreconditionError> {
        if debts.contains_key(name) {
            return Err(AccountPreconditionError::DuplicateDebt(name.to_string()));
        }
        Ok(())
    }

    pub(super) fn check_debt_exists(
        name: &str,
        debts: &HashMap<String, Decimal>,
    ) -> Result<(), AccountPreconditionError> {
        if !debts.contains_key(name) {
            return Err(AccountPreconditionError::DebtNotFound(name.to_string()));
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

    #[error("transfer source and target are the same: {0}")]
    SameCategory(String),

    #[error("transfer source {0} does not exist")]
    SourceCategoryNotFound(String),

    #[error("transfer target {0} does not exist")]
    TargetCategoryNotFound(String),

    #[error("requested list of categories do not match existing ones")]
    UnmatchedCategories,

    #[error("duplicate debt: {0}")]
    DuplicateDebt(String),

    #[error("debt {0} does not exist")]
    DebtNotFound(String),
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

    #[error("debt {0} repayment ({2}) exceeds owed ({1})")]
    DebtOverpayment(String, Decimal, Decimal),
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
        assert!(state.debts.is_empty());
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
            Decimal::from(5000u64) + (Decimal::from(10000u64) - (budget_a + budget_b))
        );
    }

    #[test]
    fn rollover_with_full_allocation_leaves_only_income() {
        let mut account = account_with(
            Decimal::from(10000u64),
            &[("a", MAX_ALLOCATION / 2), ("b", MAX_ALLOCATION / 2)],
        );
        account.rollover(Decimal::from(2000u64)).unwrap();
        let state = account.snapshot();
        assert_eq!(state.balance, Decimal::from(2000u64));
    }

    #[test]
    fn rollover_expenses_spent_from_total() {
        let mut account = account_with(Decimal::from(10000u64), &[("a", MAX_ALLOCATION / 2)]);
        account.spend("a", Decimal::from(1000u64)).unwrap();
        account.rollover(Decimal::from(5000u64)).unwrap();
        let state = account.snapshot();
        let budget_a = Decimal::from(10000u64) * (MAX_ALLOCATION / 2) / MAX_ALLOCATION;
        assert_eq!(
            state.balance,
            Decimal::from(5000u64) + (Decimal::from(10000u64) - budget_a)
        );
        assert_eq!(
            state.categories.get("a").unwrap().surplus,
            budget_a - Decimal::from(1000u64)
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
            (Decimal::from(10000u64) - first_budget) - second_budget
        );
    }

    #[test]
    fn remove_category_settles_spent_into_balance() {
        let mut account = account_with(Decimal::from(10000u64), &[("a", MAX_ALLOCATION / 4)]);
        account.spend("a", Decimal::from(1000u64)).unwrap();
        account.remove_category("a").unwrap();
        let state = account.snapshot();
        assert_eq!(state.balance, Decimal::from(9000u64));
        assert!(!state.categories.contains_key("a"));
    }

    #[test]
    fn remove_category_fails_when_sibling_exceeds_shrunken_budget() {
        let mut account = account_with(
            Decimal::from(10000u64),
            &[("a", MAX_ALLOCATION / 2), ("b", MAX_ALLOCATION / 2)],
        );
        let full_budget = Decimal::from(10000u64) * (MAX_ALLOCATION / 2) / MAX_ALLOCATION;
        account.spend("a", full_budget).unwrap();
        account.spend("b", full_budget).unwrap();
        let result = account.remove_category("a");
        assert!(matches!(
            result,
            Err(AccountError::Invariant(
                AccountInvariantError::SpendingExceedsAllocation(name, _, _, _)
            )) if name == "b"
        ));
    }

    #[test]
    fn remove_category_returns_surplus_to_balance() {
        let mut account = account_with(Decimal::from(10000u64), &[("a", MAX_ALLOCATION / 2)]);
        account.rollover(Decimal::ZERO).unwrap();
        account.remove_category("a").unwrap();
        let state = account.snapshot();
        assert_eq!(state.balance, Decimal::from(10000u64));
        assert!(!state.categories.contains_key("a"));
    }

    #[test]
    fn remove_category_settles_surplus_minus_spent() {
        let mut account = account_with(
            Decimal::from(10000u64),
            &[("a", MAX_ALLOCATION / 2), ("b", MAX_ALLOCATION / 2)],
        );
        account.rollover(Decimal::ZERO).unwrap();
        account.spend("a", Decimal::from(1000u64)).unwrap();
        account.remove_category("a").unwrap();
        let state = account.snapshot();
        let surplus_a = Decimal::from(10000u64) * (MAX_ALLOCATION / 2) / MAX_ALLOCATION;
        assert_eq!(state.balance, surplus_a - Decimal::from(1000u64));
        assert!(!state.categories.contains_key("a"));
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

    #[test]
    fn add_debt_tracks_owed() {
        let mut account = account_with(Decimal::from(1000u64), &[]);
        account.add_debt("loan", Decimal::from(400u64)).unwrap();
        let state = account.snapshot();
        assert_eq!(state.debts.get("loan"), Some(&Decimal::from(400u64)));
    }

    #[test]
    fn repay_debt_adds_on_top_of_spent() {
        let mut account = account_with(Decimal::from(10000u64), &[("a", MAX_ALLOCATION / 2)]);
        account.add_debt("loan", Decimal::from(1000u64)).unwrap();
        account.spend("a", Decimal::from(1000u64)).unwrap();
        account
            .repay_debt("a", "loan", Decimal::from(400u64))
            .unwrap();
        let state = account.snapshot();
        assert_eq!(
            state.categories.get("a").unwrap().spent,
            Decimal::from(1400u64)
        );
        assert_eq!(state.debts.get("loan"), Some(&Decimal::from(600u64)));
        assert_eq!(state.balance, Decimal::from(10000u64));
    }

    #[test]
    fn repay_debt_in_full_removes_entry() {
        let mut account = account_with(Decimal::from(10000u64), &[("a", MAX_ALLOCATION / 2)]);
        account.add_debt("loan", Decimal::from(400u64)).unwrap();
        account
            .repay_debt("a", "loan", Decimal::from(400u64))
            .unwrap();
        let state = account.snapshot();
        assert!(!state.debts.contains_key("loan"));
        assert_eq!(
            state.categories.get("a").unwrap().spent,
            Decimal::from(400u64)
        );
    }

    #[test]
    fn debt_snapshot_reconstitute_roundtrip() {
        let mut account = account_with(Decimal::from(10000u64), &[("a", MAX_ALLOCATION / 2)]);
        account.add_debt("loan", Decimal::from(1000u64)).unwrap();
        account
            .repay_debt("a", "loan", Decimal::from(400u64))
            .unwrap();
        let state = account.snapshot();
        let reconstituted = Account::reconstitute(state.clone(), None);
        assert_eq!(reconstituted.snapshot(), state);
    }

    #[test]
    fn reconstitute_replays_debt_events() {
        let mut account = account_with(Decimal::from(10000u64), &[("a", MAX_ALLOCATION / 2)]);
        let base = account.snapshot();
        account.add_debt("loan", Decimal::from(1000u64)).unwrap();
        account
            .repay_debt("a", "loan", Decimal::from(400u64))
            .unwrap();
        let final_state = account.snapshot();

        let replayed = Account::reconstitute(
            base,
            Some(vec![
                AccountEvent::DebtAdded {
                    name: "loan".to_string(),
                    amount: Decimal::from(1000u64),
                },
                AccountEvent::DebtRepaid {
                    category: "a".to_string(),
                    debt: "loan".to_string(),
                    amount: Decimal::from(400u64),
                },
            ]),
        );
        assert_eq!(replayed.snapshot(), final_state);
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

    #[test]
    fn debt_repayment_within_owed_passes() {
        assert!(
            invariants::check_debt_within_owed(
                "loan",
                Decimal::from(1000u64),
                Decimal::from(400u64)
            )
            .is_ok()
        );
    }

    #[test]
    fn debt_repayment_exactly_owed_passes() {
        assert!(
            invariants::check_debt_within_owed(
                "loan",
                Decimal::from(1000u64),
                Decimal::from(1000u64)
            )
            .is_ok()
        );
    }

    #[test]
    fn debt_repayment_exceeding_owed_fails() {
        assert!(matches!(
            invariants::check_debt_within_owed(
                "loan",
                Decimal::from(1000u64),
                Decimal::from(1000u64) + Decimal::new(1),
            ),
            Err(AccountInvariantError::DebtOverpayment(name, _, _)) if name == "loan"
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

    fn debts(names: &[(&str, u64)]) -> HashMap<String, Decimal> {
        names
            .iter()
            .map(|(name, amount)| (name.to_string(), Decimal::from(*amount)))
            .collect()
    }

    #[test]
    fn duplicate_debt_fails() {
        let debts = debts(&[("loan", 100)]);
        assert!(matches!(
            preconditions::check_debt_absent("loan", &debts),
            Err(AccountPreconditionError::DuplicateDebt(name)) if name == "loan"
        ));
    }

    #[test]
    fn absent_debt_passes() {
        let debts = debts(&[("loan", 100)]);
        assert!(preconditions::check_debt_absent("other", &debts).is_ok());
    }

    #[test]
    fn missing_debt_fails() {
        let debts = debts(&[("loan", 100)]);
        assert!(matches!(
            preconditions::check_debt_exists("other", &debts),
            Err(AccountPreconditionError::DebtNotFound(name)) if name == "other"
        ));
    }

    #[test]
    fn existing_debt_passes() {
        let debts = debts(&[("loan", 100)]);
        assert!(preconditions::check_debt_exists("loan", &debts).is_ok());
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
