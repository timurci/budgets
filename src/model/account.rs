use std::collections::HashMap;

use thiserror::Error;

const MAX_ALLOCATION: u8 = 200;

#[derive(Clone, Debug)]
pub struct Account {
    balance: f32,
    categories: HashMap<String, Spending>,
}

#[derive(Clone, Debug)]
struct Spending {
    allocation: u8,
    spent: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AccountState {
    pub balance: f32,
    pub categories: HashMap<String, CategoryState>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CategoryState {
    pub allocation: u8,
    pub spent: f32,
}

impl Account {
    pub fn add_category(&self, name: &str, allocation: u8) -> Result<Self, AddCategoryError> {
        if self.categories.contains_key(name) {
            return Err(AddCategoryError::DuplicateCategory(name.to_string()));
        }
        let spending =
            Spending::new(allocation, 0.0).map_err(|e| AddCategoryError::Aggregate(e.into()))?;
        let mut categories = self.categories.clone();
        categories.insert(name.to_string(), spending);
        Self::new(self.balance, categories).map_err(|e| AddCategoryError::Aggregate(e.into()))
    }

    pub fn add_categories(
        &self,
        allocations: HashMap<String, u8>,
    ) -> Result<Self, AddCategoriesError> {
        let mut categories = self.categories.clone();
        for (name, allocation) in allocations {
            if categories.contains_key(&name) {
                return Err(AddCategoriesError::DuplicateCategory(name));
            }
            let spending = Spending::new(allocation, 0.0)
                .map_err(|e| AddCategoriesError::Aggregate(e.into()))?;
            categories.insert(name, spending);
        }
        Self::new(self.balance, categories).map_err(|e| AddCategoriesError::Aggregate(e.into()))
    }

    pub fn spend(&self, category: &str, amount: f32) -> Result<Self, AccountSpendError> {
        if amount < 0.0 {
            return Err(AccountSpendError::NegativeAmount);
        }
        let mut categories = self.categories.clone();
        let spending = categories
            .get_mut(category)
            .ok_or_else(|| AccountSpendError::CategoryNotFound(category.to_string()))?;
        spending.spent += amount;
        Self::new(self.balance, categories).map_err(|e| e.into())
    }

    pub fn add_funds(&self, amount: f32) -> Result<Self, AccountFundsError> {
        if amount < 0.0 {
            return Err(AccountFundsError::NegativeAmount);
        }
        Self::new(self.balance + amount, self.categories.clone()).map_err(|e| e.into())
    }

    pub fn remove_funds(&self, amount: f32) -> Result<Self, AccountFundsError> {
        if amount < 0.0 {
            return Err(AccountFundsError::NegativeAmount);
        }
        Self::new(self.balance - amount, self.categories.clone()).map_err(|e| e.into())
    }

    pub fn reallocate_categories(
        &self,
        allocations: HashMap<String, u8>,
    ) -> Result<Self, AccountReallocationError> {
        if self.categories.len() != allocations.len()
            || !self.categories.keys().all(|k| allocations.contains_key(k))
        {
            return Err(AccountReallocationError::UnmatchedCategories);
        }
        let categories = allocations
            .into_iter()
            .map(|(name, allocation)| {
                let spent = self.categories.get(&name).expect("keys matched").spent;
                Spending::new(allocation, spent).map(|s| (name, s))
            })
            .collect::<Result<HashMap<_, _>, _>>()
            .map_err(|e| AccountReallocationError::Aggregate(e.into()))?;
        Self::new(self.balance, categories)
            .map_err(|e| AccountReallocationError::Aggregate(e.into()))
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
                        },
                    )
                })
                .collect(),
        }
    }

    pub fn reconstitute(state: AccountState) -> Result<Self, AccountAggregateError> {
        let categories = state
            .categories
            .into_iter()
            .map(|(name, category)| {
                Spending::new(category.allocation, category.spent).map(|s| (name, s))
            })
            .collect::<Result<HashMap<_, _>, _>>()?;
        Self::new(state.balance, categories).map_err(|e| e.into())
    }

    fn new(balance: f32, categories: HashMap<String, Spending>) -> Result<Self, AccountNewError> {
        invariants::check_negative_balance(balance)?;
        invariants::check_total_allocations(&categories)?;
        invariants::check_spending_exceeds_allocation(balance, &categories)?;
        Ok(Self {
            balance,
            categories,
        })
    }
}

impl Default for Account {
    fn default() -> Self {
        Self {
            balance: 0.0,
            categories: HashMap::new(),
        }
    }
}

impl Spending {
    pub fn new(allocation: u8, spent: f32) -> Result<Self, SpendingNewError> {
        invariants::check_allocation_limit(allocation)?;
        invariants::check_negative_spending(spent)?;
        Ok(Self { allocation, spent })
    }
}

mod invariants {
    use std::collections::HashMap;

    use super::{AccountNewError, MAX_ALLOCATION, Spending, SpendingNewError};

    pub(super) fn check_negative_balance(balance: f32) -> Result<(), AccountNewError> {
        if balance < 0.0 {
            return Err(AccountNewError::NegativeBalance);
        }
        Ok(())
    }

    pub(super) fn check_total_allocations(
        categories: &HashMap<String, Spending>,
    ) -> Result<(), AccountNewError> {
        let total_allocation: u16 = categories.values().map(|c| c.allocation as u16).sum();
        if total_allocation > MAX_ALLOCATION as u16 {
            return Err(AccountNewError::TotalAllocations(total_allocation));
        }
        Ok(())
    }

    pub(super) fn check_spending_exceeds_allocation(
        balance: f32,
        categories: &HashMap<String, Spending>,
    ) -> Result<(), AccountNewError> {
        let balance_per_allocation = balance / MAX_ALLOCATION as f32;
        for (name, category) in categories {
            if category.spent > balance_per_allocation * category.allocation as f32 {
                return Err(AccountNewError::SpendingExceedsAllocation(
                    name.clone(),
                    category.spent,
                    category.allocation as f32,
                ));
            }
        }
        Ok(())
    }

    pub(super) fn check_allocation_limit(allocation: u8) -> Result<(), SpendingNewError> {
        if allocation > MAX_ALLOCATION {
            return Err(SpendingNewError::AllocationLimit(allocation));
        }
        Ok(())
    }

    pub(super) fn check_negative_spending(spent: f32) -> Result<(), SpendingNewError> {
        if spent < 0.0 {
            return Err(SpendingNewError::NegativeSpending(spent));
        }
        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum AccountNewError {
    #[error("negative balance")]
    NegativeBalance,

    #[error("total allocation ({0}) exceeds the limit ({MAX_ALLOCATION})")]
    TotalAllocations(u16),

    #[error("category {0} spending ({1}) exceeds the allocation ({2})")]
    SpendingExceedsAllocation(String, f32, f32),
}

#[derive(Error, Debug)]
pub enum AccountSpendError {
    #[error("category {0} does not exist")]
    CategoryNotFound(String),

    #[error("change amount is negative")]
    NegativeAmount,

    #[error(transparent)]
    AccountInitialization(#[from] AccountNewError),
}

#[derive(Error, Debug)]
pub enum AccountFundsError {
    #[error("change amount is negative")]
    NegativeAmount,

    #[error(transparent)]
    AccountInitialization(#[from] AccountNewError),
}

#[derive(Error, Debug)]
pub enum AccountReallocationError {
    #[error("requested list of categories do not match existing ones")]
    UnmatchedCategories,

    #[error(transparent)]
    Aggregate(#[from] AccountAggregateError),
}

#[derive(Error, Debug)]
pub enum AccountAggregateError {
    #[error(transparent)]
    Account(#[from] AccountNewError),

    #[error(transparent)]
    Spending(#[from] SpendingNewError),
}

#[derive(Error, Debug)]
pub enum AddCategoryError {
    #[error("duplicate category: {0}")]
    DuplicateCategory(String),

    #[error(transparent)]
    Aggregate(#[from] AccountAggregateError),
}

#[derive(Error, Debug)]
pub enum AddCategoriesError {
    #[error("duplicate category: {0}")]
    DuplicateCategory(String),

    #[error(transparent)]
    Aggregate(#[from] AccountAggregateError),
}

#[derive(Error, Debug)]
pub enum SpendingNewError {
    #[error("allocation ({0}) exceeds the limit ({MAX_ALLOCATION})")]
    AllocationLimit(u8),

    #[error("spending ({0}) is negative")]
    NegativeSpending(f32),
}

#[cfg(test)]
mod account_tests {
    use super::*;
    use std::collections::HashMap;

    fn account_with(balance: f32, allocations: &[(&str, u8)]) -> Account {
        let allocations = allocations
            .iter()
            .map(|(name, allocation)| (name.to_string(), *allocation))
            .collect();
        Account::default()
            .add_funds(balance)
            .unwrap()
            .add_categories(allocations)
            .unwrap()
    }

    #[test]
    fn default_is_empty() {
        let state = Account::default().snapshot();
        assert_eq!(state.balance, 0.0);
        assert!(state.categories.is_empty());
    }

    #[test]
    fn add_category_succeeds() {
        let account = account_with(1000.0, &[]);
        let account = account.add_category("test", 10).unwrap();
        let state = account.snapshot();
        assert_eq!(state.balance, 1000.0);
        assert_eq!(
            state.categories.get("test").unwrap(),
            &CategoryState {
                allocation: 10,
                spent: 0.0,
            }
        );
    }

    #[test]
    fn add_category_cant_duplicate() {
        let account = account_with(1000.0, &[("test", 10)]);
        assert!(matches!(
            account.add_category("test", 10),
            Err(AddCategoryError::DuplicateCategory(_))
        ));
    }

    #[test]
    fn add_categories_cant_exceed_max_allocation() {
        let account = account_with(100.0, &[]);
        assert!(matches!(
            account.add_categories(HashMap::from([
                ("test".to_string(), MAX_ALLOCATION),
                ("test2".to_string(), 5),
            ])),
            Err(AddCategoriesError::Aggregate(
                AccountAggregateError::Account(AccountNewError::TotalAllocations(_))
            ))
        ));
    }

    #[test]
    fn add_categories_total_does_not_overflow() {
        let account = account_with(100.0, &[]);
        assert!(matches!(
            account.add_categories(HashMap::from([
                ("a".to_string(), MAX_ALLOCATION - MAX_ALLOCATION / 4),
                ("b".to_string(), MAX_ALLOCATION - MAX_ALLOCATION / 4),
            ])),
            Err(AddCategoriesError::Aggregate(
                AccountAggregateError::Account(AccountNewError::TotalAllocations(300))
            ))
        ));
    }

    #[test]
    fn reconstitute_cant_have_negative_balance() {
        let state = AccountState {
            balance: -100.0,
            categories: HashMap::new(),
        };
        assert!(matches!(
            Account::reconstitute(state),
            Err(AccountAggregateError::Account(
                AccountNewError::NegativeBalance
            ))
        ));
    }

    #[test]
    fn reconstitute_spending_cant_exceed_allocation() {
        let state = AccountState {
            balance: 100.0,
            categories: HashMap::from([(
                "test".to_string(),
                CategoryState {
                    allocation: MAX_ALLOCATION / 2,
                    spent: 60.0,
                },
            )]),
        };
        assert!(matches!(
            Account::reconstitute(state),
            Err(AccountAggregateError::Account(
                AccountNewError::SpendingExceedsAllocation(_, _, _)
            ))
        ));
    }

    #[test]
    fn reconstitute_spending_cant_exceed_allocation_limit() {
        let state = AccountState {
            balance: 100.0,
            categories: HashMap::from([(
                "test".to_string(),
                CategoryState {
                    allocation: MAX_ALLOCATION + 1,
                    spent: 0.0,
                },
            )]),
        };
        assert!(matches!(
            Account::reconstitute(state),
            Err(AccountAggregateError::Spending(
                SpendingNewError::AllocationLimit(_)
            ))
        ));
    }

    #[test]
    fn snapshot_reconstitute_roundtrip() {
        let account = account_with(
            10000.0,
            &[("a", MAX_ALLOCATION / 2), ("b", MAX_ALLOCATION / 4)],
        );
        let account = account.spend("a", 1000.0).unwrap();
        let state = account.snapshot();
        let reconstituted = Account::reconstitute(state.clone()).unwrap();
        assert_eq!(reconstituted.snapshot(), state);
    }

    #[test]
    fn spend_category_not_found() {
        let account = account_with(10000.0, &[("test1", 10), ("test2", 10)]);
        assert!(matches!(
            account.spend("savings", 50.0),
            Err(AccountSpendError::CategoryNotFound(_))
        ));
        assert!(account.spend("test1", 10.0).is_ok());
    }

    #[test]
    fn spend_category_spending_exceeds_allocation() {
        let account = account_with(10000.0, &[("test1", MAX_ALLOCATION / 2)]);
        let account = account.spend("test1", 5000.0).unwrap();
        assert!(matches!(
            account.spend("test1", 500.0),
            Err(AccountSpendError::AccountInitialization(
                AccountNewError::SpendingExceedsAllocation(_, _, _)
            ))
        ));
    }

    #[test]
    fn spend_is_immutable() {
        let account = account_with(1000.0, &[("test1", 10)]);
        let new_account = account.spend("test1", 10.0).unwrap();
        assert!(account.spend("test1", 10.0).is_ok());
        assert!(new_account.spend("test1", 10.0).is_ok());
    }

    #[test]
    fn add_funds_immutable() {
        let account = account_with(1000.0, &[("test1", 10)]);
        let new_account = account.add_funds(500.0).unwrap();
        assert!(account.remove_funds(200.0).is_ok());
        assert!(new_account.remove_funds(200.0).is_ok());
        assert!(account.remove_funds(200.0).is_ok());
    }

    #[test]
    fn remove_funds_fails_when_spending_exceeds_allocation() {
        let account = account_with(10000.0, &[("test1", MAX_ALLOCATION / 2)]);
        let account = account.spend("test1", 4000.0).unwrap();
        assert!(matches!(
            account.remove_funds(5000.0),
            Err(AccountFundsError::AccountInitialization(
                AccountNewError::SpendingExceedsAllocation(_, _, _)
            ))
        ));
        assert!(account.remove_funds(100.0).is_ok());
    }

    #[test]
    fn remove_funds_immutable_on_error() {
        let account = account_with(10000.0, &[("test1", MAX_ALLOCATION / 2)]);
        let result = account.remove_funds(9000.0);
        assert!(account.add_funds(100.0).is_ok());
        assert!(result.is_ok());
    }

    #[test]
    fn reallocate_successful() {
        let account = account_with(
            10000.0,
            &[("a", MAX_ALLOCATION / 2), ("b", MAX_ALLOCATION / 2)],
        );
        let result = account.reallocate_categories(HashMap::from([
            ("a".to_string(), MAX_ALLOCATION / 4),
            ("b".to_string(), MAX_ALLOCATION - MAX_ALLOCATION / 4),
        ]));
        assert!(result.is_ok());
    }

    #[test]
    fn reallocate_fails_when_spending_exceeds_new_allocation() {
        let account = account_with(
            10000.0,
            &[("a", MAX_ALLOCATION / 2), ("b", MAX_ALLOCATION / 2)],
        );
        let account = account.spend("a", 4000.0).unwrap();
        assert!(matches!(
            account.reallocate_categories(HashMap::from([
                ("a".to_string(), MAX_ALLOCATION / 4),
                ("b".to_string(), MAX_ALLOCATION - MAX_ALLOCATION / 4),
            ])),
            Err(AccountReallocationError::Aggregate(
                AccountAggregateError::Account(AccountNewError::SpendingExceedsAllocation(
                    name, _, _
                ))
            )) if name == "a"
        ));
    }
}

#[cfg(test)]
mod spending_tests {
    use super::*;

    #[test]
    fn new_cant_exceed_max_allocation() {
        let result = Spending::new(MAX_ALLOCATION + 1, 0.0);
        assert!(matches!(result, Err(SpendingNewError::AllocationLimit(_))));
    }

    #[test]
    fn new_cant_have_negative_spending() {
        let result = Spending::new(MAX_ALLOCATION, -1.0);
        assert!(matches!(result, Err(SpendingNewError::NegativeSpending(_))));
    }
}
