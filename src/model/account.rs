use std::collections::HashSet;

use thiserror::Error;

const MAX_ALLOCATION: u8 = 200;

pub struct Account {
    balance: f32,
    categories: Vec<SpendingCategory>,
}

pub struct SpendingCategory {
    name: String,
    allocation: u8,
    spending: f32,
}

impl Account {
    pub fn new(balance: f32, categories: Vec<SpendingCategory>) -> Result<Self, AccountNewError> {
        Self::new_check_total_allocations(&categories)?;
        Self::new_check_spending_exceeds_allocation(balance, &categories)?;
        Self::new_check_duplicate_categories(&categories)?;

        Ok(Self {
            balance,
            categories,
        })
    }

    fn new_check_total_allocations(categories: &[SpendingCategory]) -> Result<(), AccountNewError> {
        let total_allocation: u8 = categories.iter().map(|c| c.allocation).sum();
        if total_allocation > MAX_ALLOCATION {
            return Err(AccountNewError::TotalAllocations(total_allocation));
        }
        Ok(())
    }

    fn new_check_spending_exceeds_allocation(
        balance: f32,
        categories: &[SpendingCategory],
    ) -> Result<(), AccountNewError> {
        let balance_per_allocation = balance / MAX_ALLOCATION as f32;
        for category in categories {
            if category.spending > balance_per_allocation * category.allocation as f32 {
                return Err(AccountNewError::SpendingExceedsAllocation(
                    category.name.clone(),
                    category.spending,
                    category.allocation as f32,
                ));
            }
        }
        Ok(())
    }

    fn new_check_duplicate_categories(
        categories: &[SpendingCategory],
    ) -> Result<(), AccountNewError> {
        let mut category_names = HashSet::new();
        for category in categories.iter() {
            if !category_names.insert(category.name.clone()) {
                return Err(AccountNewError::DuplicateCategory(category.name.clone()));
            }
        }
        Ok(())
    }
}

impl SpendingCategory {
    pub fn new(
        name: String,
        allocation: u8,
        spending: f32,
    ) -> Result<Self, SpendingCategoryNewError> {
        if allocation > MAX_ALLOCATION {
            return Err(SpendingCategoryNewError::AllocationLimit(allocation));
        }

        if spending < 0.0 {
            return Err(SpendingCategoryNewError::NegativeSpending(spending));
        }

        Ok(Self {
            name,
            allocation,
            spending,
        })
    }
}

#[derive(Error, Debug)]
pub enum AccountNewError {
    #[error("total allocation ({0}) exceeds the limit ({MAX_ALLOCATION})")]
    TotalAllocations(u8),

    #[error("spending category {0} balance ({1}) exceeds the allocation ({2})")]
    SpendingExceedsAllocation(String, f32, f32),

    #[error("duplicate spending category: {0}")]
    DuplicateCategory(String),
}

#[derive(Error, Debug)]
pub enum SpendingCategoryNewError {
    #[error("allocation ({0}) exceeds the limit ({MAX_ALLOCATION})")]
    AllocationLimit(u8),

    #[error("spending ({0}) is negative")]
    NegativeSpending(f32),
}

#[cfg(test)]
mod account_tests {
    use super::*;

    #[test]
    fn new_allocations_cant_exceed_max_allocation() {
        let cat1 = SpendingCategory::new("test".to_string(), MAX_ALLOCATION, 0.0).unwrap();
        let cat2 = SpendingCategory::new("test2".to_string(), 5, 0.0).unwrap();
        let account = Account::new(100.0, vec![cat1, cat2]);
        assert!(matches!(account, Err(AccountNewError::TotalAllocations(_))));
    }

    #[test]
    fn new_spending_cant_exceed_allocation() {
        let cat1 = SpendingCategory::new("test".to_string(), MAX_ALLOCATION / 2, 60.0).unwrap();
        let account = Account::new(100.0, vec![cat1]);
        assert!(matches!(
            account,
            Err(AccountNewError::SpendingExceedsAllocation(_, _, _))
        ));
    }

    #[test]
    fn new_cant_have_duplicate_category() {
        let cat1 = SpendingCategory::new("test".to_string(), 100, 0.0).unwrap();
        let cat2 = SpendingCategory::new("test".to_string(), 50, 20.0).unwrap();
        let account = Account::new(10000.0, vec![cat1, cat2]);
        assert!(matches!(
            account,
            Err(AccountNewError::DuplicateCategory(_))
        ));
    }
}

#[cfg(test)]
mod spending_category_tests {
    use super::*;

    #[test]
    fn new_cant_exceed_max_allocation() {
        let result = SpendingCategory::new("test".to_string(), MAX_ALLOCATION + 1, 0.0);
        assert!(matches!(
            result,
            Err(SpendingCategoryNewError::AllocationLimit(_))
        ));
    }

    #[test]
    fn new_cant_have_negative_spending() {
        let result = SpendingCategory::new("test".to_string(), MAX_ALLOCATION, -1.0);
        assert!(matches!(
            result,
            Err(SpendingCategoryNewError::NegativeSpending(_))
        ));
    }
}
