use std::collections::HashMap;

use thiserror::Error;

use crate::types::id::id_type;
use crate::types::{Decimal, DecimalError};

id_type!(AccountId, CategoryId, DebtId);

const MAX_ALLOCATION: u8 = 200;

fn budget(balance: Decimal, allocation: u8) -> Decimal {
    balance * allocation / MAX_ALLOCATION
}

fn available_surplus(
    balance: Decimal,
    allocation: u8,
    surplus: Decimal,
    spent: Decimal,
) -> Result<Decimal, DecimalError> {
    let overspent = spent
        .safe_sub(budget(balance, allocation))
        .unwrap_or(Decimal::ZERO);
    surplus.safe_sub(overspent)
}

fn total_budget(balance: Decimal, categories: &HashMap<CategoryId, Spending>) -> Decimal {
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
    id: AccountId,
    balance: Decimal,
    categories: HashMap<CategoryId, Spending>,
    debts: HashMap<DebtId, Decimal>,
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

#[derive(Clone, Debug, PartialEq)]
pub enum AccountEvent {
    CategoryAdded {
        id: CategoryId,
        allocation: u8,
    },
    CategoryRemoved {
        id: CategoryId,
    },
    RolledOver {
        income: Decimal,
    },
    Spent {
        category: CategoryId,
        amount: Decimal,
    },
    SurplusTransferred {
        from: CategoryId,
        to: CategoryId,
        amount: Decimal,
    },
    FundsAdded {
        amount: Decimal,
    },
    FundsRemoved {
        amount: Decimal,
    },
    CategoriesReallocated {
        allocations: HashMap<CategoryId, u8>,
    },
    DebtAdded {
        id: DebtId,
        amount: Decimal,
    },
    DebtRepaid {
        category: CategoryId,
        debt: DebtId,
        amount: Decimal,
    },
}

impl Account {
    pub fn id(&self) -> &AccountId {
        &self.id
    }

    pub fn events(&self) -> &[AccountEvent] {
        &self.events
    }

    pub fn mark_committed(&mut self) {
        self.events.clear();
    }

    pub fn add_category(&mut self, allocation: u8) -> Result<CategoryId, AccountError> {
        Spending::new(allocation, Decimal::ZERO, Decimal::ZERO)?;
        invariants::check_total_allocations(
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
        preconditions::check_category_exists(id, &self.categories)?;
        let spending = self.categories.get(id).expect("existence checked");
        let funded = self.balance + spending.surplus;
        invariants::check_negative_balance(funded, spending.spent)?;
        let balance = removal_balance(self.balance, spending.surplus, spending.spent);
        invariants::check_all_spending_within_budget(
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

    pub fn spend(&mut self, category: &CategoryId, amount: Decimal) -> Result<(), AccountError> {
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
        preconditions::check_distinct_source_target(from, to)?;
        preconditions::check_source_exists(from, &self.categories)?;
        preconditions::check_target_exists(to, &self.categories)?;
        let source = self.categories.get(from).expect("source checked");
        invariants::check_transfer_within_surplus(
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
        invariants::check_negative_balance(self.balance, amount)?;
        let balance = self.balance - amount;
        invariants::check_all_spending_within_budget(balance, &self.categories)?;
        self.emit(AccountEvent::FundsRemoved { amount });
        Ok(())
    }

    pub fn reallocate_categories(
        &mut self,
        allocations: HashMap<CategoryId, u8>,
    ) -> Result<(), AccountError> {
        preconditions::check_categories_match(&self.categories, &allocations)?;
        invariants::check_total_allocations(allocations.values().copied())?;
        for (id, allocation) in &allocations {
            let existing = self.categories.get(id).expect("keys matched");
            Spending::new(*allocation, existing.spent, existing.surplus)?;
            invariants::check_spending_within_budget(
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
    use super::{
        AccountInvariantError, CategoryId, DebtId, Decimal, MAX_ALLOCATION, Spending,
        available_surplus,
    };

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
        id: &CategoryId,
        balance: Decimal,
        allocation: u8,
        surplus: Decimal,
        spent: Decimal,
    ) -> Result<(), AccountInvariantError> {
        if available_surplus(balance, allocation, surplus, spent).is_err() {
            return Err(AccountInvariantError::SpendingExceedsAllocation(
                *id, spent, allocation, surplus,
            ));
        }
        Ok(())
    }

    pub(super) fn check_all_spending_within_budget<'a>(
        balance: Decimal,
        categories: impl IntoIterator<Item = (&'a CategoryId, &'a Spending)>,
    ) -> Result<(), AccountInvariantError> {
        for (id, spending) in categories {
            check_spending_within_budget(
                id,
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
        available_surplus(balance, allocation, surplus, spent)
            .map(|_| ())
            .map_err(|_| AccountInvariantError::NegativeSurplus)
    }

    pub(super) fn check_transfer_within_surplus(
        id: &CategoryId,
        balance: Decimal,
        allocation: u8,
        surplus: Decimal,
        spent: Decimal,
        amount: Decimal,
    ) -> Result<(), AccountInvariantError> {
        let available = available_surplus(balance, allocation, surplus, spent)
            .map_err(|_| AccountInvariantError::NegativeSurplus)?;
        if amount > available {
            return Err(AccountInvariantError::InsufficientSurplus(
                *id, available, amount,
            ));
        }
        Ok(())
    }

    pub(super) fn check_debt_within_owed(
        id: &DebtId,
        owed: Decimal,
        amount: Decimal,
    ) -> Result<(), AccountInvariantError> {
        if owed.safe_sub(amount).is_err() {
            return Err(AccountInvariantError::DebtOverpayment(*id, owed, amount));
        }
        Ok(())
    }
}

mod preconditions {
    use std::collections::HashMap;

    use super::{AccountPreconditionError, CategoryId, DebtId, Decimal, Spending};

    pub(super) fn check_category_exists(
        id: &CategoryId,
        categories: &HashMap<CategoryId, Spending>,
    ) -> Result<(), AccountPreconditionError> {
        if !categories.contains_key(id) {
            return Err(AccountPreconditionError::CategoryNotFound(*id));
        }
        Ok(())
    }

    pub(super) fn check_distinct_source_target(
        from: &CategoryId,
        to: &CategoryId,
    ) -> Result<(), AccountPreconditionError> {
        if from == to {
            return Err(AccountPreconditionError::SameCategory(*from));
        }
        Ok(())
    }

    pub(super) fn check_source_exists(
        from: &CategoryId,
        categories: &HashMap<CategoryId, Spending>,
    ) -> Result<(), AccountPreconditionError> {
        if !categories.contains_key(from) {
            return Err(AccountPreconditionError::SourceCategoryNotFound(*from));
        }
        Ok(())
    }

    pub(super) fn check_target_exists(
        to: &CategoryId,
        categories: &HashMap<CategoryId, Spending>,
    ) -> Result<(), AccountPreconditionError> {
        if !categories.contains_key(to) {
            return Err(AccountPreconditionError::TargetCategoryNotFound(*to));
        }
        Ok(())
    }

    pub(super) fn check_categories_match(
        categories: &HashMap<CategoryId, Spending>,
        allocations: &HashMap<CategoryId, u8>,
    ) -> Result<(), AccountPreconditionError> {
        if categories.len() != allocations.len()
            || !categories.keys().all(|id| allocations.contains_key(id))
        {
            return Err(AccountPreconditionError::UnmatchedCategories);
        }
        Ok(())
    }

    pub(super) fn check_debt_exists(
        id: &DebtId,
        debts: &HashMap<DebtId, Decimal>,
    ) -> Result<(), AccountPreconditionError> {
        if !debts.contains_key(id) {
            return Err(AccountPreconditionError::DebtNotFound(*id));
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
    CategoryNotFound(CategoryId),

    #[error("transfer source and target are the same: {0}")]
    SameCategory(CategoryId),

    #[error("transfer source {0} does not exist")]
    SourceCategoryNotFound(CategoryId),

    #[error("transfer target {0} does not exist")]
    TargetCategoryNotFound(CategoryId),

    #[error("requested list of categories do not match existing ones")]
    UnmatchedCategories,

    #[error("debt {0} does not exist")]
    DebtNotFound(DebtId),
}

#[derive(Error, Debug)]
pub enum AccountInvariantError {
    #[error("negative balance")]
    NegativeBalance,

    #[error("total allocation ({0}) exceeds the limit ({MAX_ALLOCATION})")]
    TotalAllocations(u32),

    #[error("category {0} spending ({1}) exceeds the allocation ({2}) plus surplus ({3})")]
    SpendingExceedsAllocation(CategoryId, Decimal, u8, Decimal),

    #[error("allocation ({0}) exceeds the limit ({MAX_ALLOCATION})")]
    AllocationLimit(u8),

    #[error("surplus would be negative")]
    NegativeSurplus,

    #[error("category {0} has insufficient available surplus ({1}) to transfer {2}")]
    InsufficientSurplus(CategoryId, Decimal, Decimal),

    #[error("debt {0} repayment ({2}) exceeds owed ({1})")]
    DebtOverpayment(DebtId, Decimal, Decimal),
}

#[cfg(test)]
mod account_tests {
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
        assert!(!account.events().is_empty());
        account.mark_committed();
        assert!(account.events().is_empty());
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

#[cfg(test)]
mod invariants_tests {
    use super::*;
    use std::collections::HashMap;

    fn category_id(seed: u128) -> CategoryId {
        CategoryId::from(uuid::Uuid::from_u128(seed))
    }

    fn debt_id(seed: u128) -> DebtId {
        DebtId::from(uuid::Uuid::from_u128(seed))
    }

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
                &category_id(0),
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
                &category_id(0),
                balance,
                allocation,
                Decimal::from(500u64),
                spent
            ),
            Err(AccountInvariantError::SpendingExceedsAllocation(id, _, _, _))
                if id == category_id(0)
        ));
    }

    #[test]
    fn all_spending_within_budget_passes() {
        let categories = HashMap::from([
            (
                category_id(0),
                Spending {
                    allocation: MAX_ALLOCATION / 2,
                    spent: Decimal::ZERO,
                    surplus: Decimal::ZERO,
                },
            ),
            (
                category_id(1),
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
                category_id(0),
                Spending {
                    allocation: MAX_ALLOCATION / 2,
                    spent: Decimal::ZERO,
                    surplus: Decimal::ZERO,
                },
            ),
            (
                category_id(1),
                Spending {
                    allocation: MAX_ALLOCATION / 2,
                    spent: budget(Decimal::from(1000u64), MAX_ALLOCATION / 2) + Decimal::from(1u64),
                    surplus: Decimal::ZERO,
                },
            ),
        ]);
        assert!(matches!(
            invariants::check_all_spending_within_budget(Decimal::from(1000u64), &categories),
            Err(AccountInvariantError::SpendingExceedsAllocation(id, _, _, _))
                if id == category_id(1)
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
    fn transfer_within_available_surplus_passes() {
        assert!(
            invariants::check_transfer_within_surplus(
                &category_id(0),
                Decimal::from(10000u64),
                MAX_ALLOCATION / 2,
                Decimal::from(500u64),
                Decimal::from(4000u64),
                Decimal::from(400u64),
            )
            .is_ok()
        );
    }

    #[test]
    fn transfer_exactly_available_surplus_passes() {
        assert!(
            invariants::check_transfer_within_surplus(
                &category_id(0),
                Decimal::from(10000u64),
                MAX_ALLOCATION / 2,
                Decimal::from(500u64),
                Decimal::from(4000u64),
                Decimal::from(500u64),
            )
            .is_ok()
        );
    }

    #[test]
    fn transfer_of_zero_passes() {
        assert!(
            invariants::check_transfer_within_surplus(
                &category_id(0),
                Decimal::from(10000u64),
                MAX_ALLOCATION / 2,
                Decimal::ZERO,
                Decimal::ZERO,
                Decimal::ZERO,
            )
            .is_ok()
        );
    }

    #[test]
    fn transfer_exceeding_available_surplus_fails() {
        assert!(matches!(
            invariants::check_transfer_within_surplus(
                &category_id(0),
                Decimal::from(10000u64),
                MAX_ALLOCATION / 2,
                Decimal::from(500u64),
                Decimal::from(4000u64),
                Decimal::from(501u64),
            ),
            Err(AccountInvariantError::InsufficientSurplus(id, available, amount))
                if id == category_id(0)
                    && available == Decimal::from(500u64)
                    && amount == Decimal::from(501u64)
        ));
    }

    #[test]
    fn transfer_limited_by_overspending_passes() {
        let balance = Decimal::from(10000u64);
        let allocation = MAX_ALLOCATION / 2;
        let overspent = Decimal::from(200u64);
        let surplus = Decimal::from(500u64);
        let spent = budget(balance, allocation) + overspent;
        assert!(
            invariants::check_transfer_within_surplus(
                &category_id(0),
                balance,
                allocation,
                surplus,
                spent,
                surplus - overspent,
            )
            .is_ok()
        );
    }

    #[test]
    fn transfer_exceeding_overspent_surplus_fails() {
        let balance = Decimal::from(10000u64);
        let allocation = MAX_ALLOCATION / 2;
        let overspent = Decimal::from(200u64);
        let surplus = Decimal::from(500u64);
        let spent = budget(balance, allocation) + overspent;
        let available = surplus - overspent;
        assert!(matches!(
            invariants::check_transfer_within_surplus(
                &category_id(0),
                balance,
                allocation,
                surplus,
                spent,
                available + Decimal::new(1),
            ),
            Err(AccountInvariantError::InsufficientSurplus(id, reported, amount))
                if id == category_id(0)
                    && reported == available
                    && amount == available + Decimal::new(1)
        ));
    }

    #[test]
    fn transfer_from_underfunded_category_fails() {
        let balance = Decimal::from(10000u64);
        let allocation = MAX_ALLOCATION / 2;
        let spent = budget(balance, allocation) + Decimal::from(200u64);
        assert!(matches!(
            invariants::check_transfer_within_surplus(
                &category_id(0),
                balance,
                allocation,
                Decimal::from(100u64),
                spent,
                Decimal::from(1u64),
            ),
            Err(AccountInvariantError::NegativeSurplus)
        ));
    }

    #[test]
    fn debt_repayment_within_owed_passes() {
        assert!(
            invariants::check_debt_within_owed(
                &debt_id(0),
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
                &debt_id(0),
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
                &debt_id(0),
                Decimal::from(1000u64),
                Decimal::from(1000u64) + Decimal::new(1),
            ),
            Err(AccountInvariantError::DebtOverpayment(id, _, _)) if id == debt_id(0)
        ));
    }
}

#[cfg(test)]
mod preconditions_tests {
    use super::*;
    use std::collections::HashMap;

    fn category_id(seed: u128) -> CategoryId {
        CategoryId::from(uuid::Uuid::from_u128(seed))
    }

    fn debt_id(seed: u128) -> DebtId {
        DebtId::from(uuid::Uuid::from_u128(seed))
    }

    fn categories(names: &[&str]) -> HashMap<CategoryId, Spending> {
        names
            .iter()
            .enumerate()
            .map(|(index, _name)| {
                (
                    category_id(index as u128),
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
            preconditions::check_category_exists(&category_id(1), &categories),
            Err(AccountPreconditionError::CategoryNotFound(id)) if id == category_id(1)
        ));
    }

    #[test]
    fn existing_category_passes() {
        let categories = categories(&["a"]);
        assert!(preconditions::check_category_exists(&category_id(0), &categories).is_ok());
    }

    #[test]
    fn same_source_and_target_fails() {
        assert!(matches!(
            preconditions::check_distinct_source_target(&category_id(0), &category_id(0)),
            Err(AccountPreconditionError::SameCategory(id)) if id == category_id(0)
        ));
    }

    #[test]
    fn distinct_source_and_target_passes() {
        assert!(
            preconditions::check_distinct_source_target(&category_id(0), &category_id(1)).is_ok()
        );
    }

    #[test]
    fn missing_transfer_source_fails() {
        let categories = categories(&["b"]);
        assert!(matches!(
            preconditions::check_source_exists(&category_id(1), &categories),
            Err(AccountPreconditionError::SourceCategoryNotFound(id)) if id == category_id(1)
        ));
    }

    #[test]
    fn missing_transfer_target_fails() {
        let categories = categories(&["a"]);
        assert!(matches!(
            preconditions::check_target_exists(&category_id(1), &categories),
            Err(AccountPreconditionError::TargetCategoryNotFound(id)) if id == category_id(1)
        ));
    }

    #[test]
    fn mismatched_allocation_count_fails() {
        let categories = categories(&["a"]);
        let allocations = HashMap::from([(category_id(0), 10), (category_id(1), 10)]);
        assert!(matches!(
            preconditions::check_categories_match(&categories, &allocations),
            Err(AccountPreconditionError::UnmatchedCategories)
        ));
    }

    #[test]
    fn mismatched_allocation_names_fails() {
        let categories = categories(&["a"]);
        let allocations = HashMap::from([(category_id(1), 10)]);
        assert!(matches!(
            preconditions::check_categories_match(&categories, &allocations),
            Err(AccountPreconditionError::UnmatchedCategories)
        ));
    }

    #[test]
    fn matching_allocations_pass() {
        let categories = categories(&["a", "b"]);
        let allocations = HashMap::from([(category_id(0), 10), (category_id(1), 20)]);
        assert!(preconditions::check_categories_match(&categories, &allocations).is_ok());
    }

    fn debts(names: &[&str]) -> HashMap<DebtId, Decimal> {
        names
            .iter()
            .enumerate()
            .map(|(index, _name)| (debt_id(index as u128), Decimal::from(100u64)))
            .collect()
    }

    #[test]
    fn missing_debt_fails() {
        let debts = debts(&["loan"]);
        assert!(matches!(
            preconditions::check_debt_exists(&debt_id(1), &debts),
            Err(AccountPreconditionError::DebtNotFound(id)) if id == debt_id(1)
        ));
    }

    #[test]
    fn existing_debt_passes() {
        let debts = debts(&["loan"]);
        assert!(preconditions::check_debt_exists(&debt_id(0), &debts).is_ok());
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
