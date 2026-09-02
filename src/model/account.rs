use std::collections::HashMap;

use thiserror::Error;

const MAX_ALLOCATION: u8 = 200;

#[derive(Clone, Debug)]
pub struct Account {
    balance: f32,
    categories: HashMap<String, Spending>,
}

#[derive(Clone, Debug)]
pub struct Spending {
    allocation: u8,
    spent: f32,
}

impl Account {
    pub fn new(
        balance: f32,
        categories: HashMap<String, Spending>,
    ) -> Result<Self, AccountNewError> {
        if balance < 0.0 {
            return Err(AccountNewError::NegativeBalance);
        }
        Self::new_check_total_allocations(&categories)?;
        Self::new_check_spending_exceeds_allocation(balance, &categories)?;

        Ok(Self {
            balance,
            categories,
        })
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

    fn new_check_total_allocations(
        categories: &HashMap<String, Spending>,
    ) -> Result<(), AccountNewError> {
        let total_allocation: u8 = categories.values().map(|c| c.allocation).sum();
        if total_allocation > MAX_ALLOCATION {
            return Err(AccountNewError::TotalAllocations(total_allocation));
        }
        Ok(())
    }

    fn new_check_spending_exceeds_allocation(
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
}

impl Spending {
    pub fn new(allocation: u8, spent: f32) -> Result<Self, SpendingNewError> {
        if allocation > MAX_ALLOCATION {
            return Err(SpendingNewError::AllocationLimit(allocation));
        }

        if spent < 0.0 {
            return Err(SpendingNewError::NegativeSpending(spent));
        }

        Ok(Self { allocation, spent })
    }
}

#[derive(Error, Debug)]
pub enum AccountNewError {
    #[error("negative balance")]
    NegativeBalance,

    #[error("total allocation ({0}) exceeds the limit ({MAX_ALLOCATION})")]
    TotalAllocations(u8),

    #[error("category {0} spending ({1}) exceeds the allocation ({2})")]
    SpendingExceedsAllocation(String, f32, f32),

    #[error("duplicate spending category: {0}")]
    DuplicateCategory(String),
}

#[derive(Error, Debug)]
pub enum AccountSpendError {
    #[error("category {0} does not exist")]
    CategoryNotFound(String),

    #[error("change amount is negative")]
    NegativeAmount,

    #[error(transparent)]
    Initialization(#[from] AccountNewError),
}

#[derive(Error, Debug)]
pub enum AccountFundsError {
    #[error("change amount is negative")]
    NegativeAmount,

    #[error(transparent)]
    Initialization(#[from] AccountNewError),
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

    #[test]
    fn new_cant_have_negative_balance() {
        assert!(matches!(
            Account::new(-100.0, HashMap::new()),
            Err(AccountNewError::NegativeBalance)
        ));
    }

    #[test]
    fn new_allocations_cant_exceed_max_allocation() {
        let cat1 = Spending::new(MAX_ALLOCATION, 0.0).unwrap();
        let cat2 = Spending::new(5, 0.0).unwrap();
        let categories = HashMap::from([("test".to_string(), cat1), ("test2".to_string(), cat2)]);
        let account = Account::new(100.0, categories);
        assert!(matches!(account, Err(AccountNewError::TotalAllocations(_))));
    }

    #[test]
    fn new_spending_cant_exceed_allocation() {
        let cat1 = Spending::new(MAX_ALLOCATION / 2, 60.0).unwrap();
        let categories = HashMap::from([("test".to_string(), cat1)]);
        let account = Account::new(100.0, categories);
        assert!(matches!(
            account,
            Err(AccountNewError::SpendingExceedsAllocation(_, _, _))
        ));
    }

    #[test]
    fn spend_category_not_found() {
        let cat1 = Spending::new(10, 0.0).unwrap();
        let cat2 = Spending::new(10, 0.0).unwrap();
        let categories = HashMap::from([("test1".to_string(), cat1), ("test2".to_string(), cat2)]);
        let account = Account::new(10000.0, categories).unwrap();
        assert!(matches!(
            account.spend("savings", 50.0),
            Err(AccountSpendError::CategoryNotFound(_))
        ));
        // original still usable after error - true immutability
        assert!(account.spend("test1", 10.0).is_ok());
    }

    #[test]
    fn spend_category_spending_exceeds_allocation() {
        let cat1 = Spending::new(MAX_ALLOCATION / 2, 0.0).unwrap();
        let categories = HashMap::from([("test1".to_string(), cat1)]);
        let account = Account::new(10000.0, categories).unwrap();

        let account = account.spend("test1", 5000.0).unwrap();
        assert!(matches!(
            account.spend("test1", 500.0),
            Err(AccountSpendError::Initialization(
                AccountNewError::SpendingExceedsAllocation(_, _, _)
            ))
        ));
    }

    #[test]
    fn spend_is_immutable() {
        let cat1 = Spending::new(10, 0.0).unwrap();
        let categories = HashMap::from([("test1".to_string(), cat1)]);
        let account = Account::new(1000.0, categories).unwrap();
        let new_account = account.spend("test1", 10.0).unwrap();
        // original unchanged - spending again on original still succeeds with full allocation
        assert!(account.spend("test1", 10.0).is_ok());
        assert!(new_account.spend("test1", 10.0).is_ok());
    }

    #[test]
    fn add_funds_immutable() {
        let cat1 = Spending::new(10, 0.0).unwrap();
        let categories = HashMap::from([("test1".to_string(), cat1)]);
        let account = Account::new(1000.0, categories).unwrap();
        let new_account = account.add_funds(500.0).unwrap();
        // original unchanged, both can remove funds
        assert!(account.remove_funds(200.0).is_ok());
        assert!(new_account.remove_funds(200.0).is_ok());
        // error does not consume original
        assert!(account.remove_funds(200.0).is_ok());
    }

    #[test]
    fn remove_funds_fails_when_spending_exceeds_allocation() {
        let cat1 = Spending::new(MAX_ALLOCATION / 2, 4000.0).unwrap();
        let categories = HashMap::from([("test1".to_string(), cat1)]);
        let account = Account::new(10000.0, categories).unwrap();
        // removing 5000 -> new_balance 5000, allocation 100 -> allowed 2500, spent 4000 exceeds
        assert!(matches!(
            account.remove_funds(5000.0),
            Err(AccountFundsError::Initialization(
                AccountNewError::SpendingExceedsAllocation(_, _, _)
            ))
        ));
        // original still usable after error
        assert!(account.remove_funds(100.0).is_ok());
    }

    #[test]
    fn remove_funds_immutable_on_error() {
        let cat1 = Spending::new(MAX_ALLOCATION / 2, 0.0).unwrap();
        let categories = HashMap::from([("test1".to_string(), cat1)]);
        let account = Account::new(10000.0, categories).unwrap();
        let result = account.remove_funds(9000.0);
        // original still available without clone
        assert!(account.add_funds(100.0).is_ok());
        assert!(result.is_ok());
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
