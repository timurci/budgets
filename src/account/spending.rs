use std::collections::HashMap;

use crate::types::{Decimal, DecimalError};

use super::error::AccountInvariantError;
use super::invariant;
use super::model::CategoryId;

pub(super) const MAX_ALLOCATION: u8 = 200;

pub(super) fn budget(balance: Decimal, allocation: u8) -> Decimal {
    balance * allocation / MAX_ALLOCATION
}

pub(super) fn available_surplus(
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

pub(super) fn total_budget(
    balance: Decimal,
    categories: &HashMap<CategoryId, Spending>,
) -> Decimal {
    categories
        .values()
        .map(|spending| budget(balance, spending.allocation))
        .sum()
}

pub(super) fn removal_balance(balance: Decimal, surplus: Decimal, spent: Decimal) -> Decimal {
    (balance + surplus) - spent
}

#[derive(Clone, Debug)]
pub(super) struct Spending {
    pub(super) allocation: u8,
    pub(super) spent: Decimal,
    pub(super) surplus: Decimal,
}

impl Spending {
    pub(super) fn new(
        allocation: u8,
        spent: Decimal,
        surplus: Decimal,
    ) -> Result<Self, AccountInvariantError> {
        invariant::check_allocation_limit(allocation)?;
        Ok(Self {
            allocation,
            spent,
            surplus,
        })
    }
}

#[cfg(test)]
mod tests {
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
