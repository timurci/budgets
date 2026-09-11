use crate::types::Decimal;

use super::error::AccountInvariantError;
use super::model::{CategoryId, DebtId};
use super::spending::{MAX_ALLOCATION, Spending, available_surplus};

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

#[cfg(test)]
mod tests {
    use super::super::spending::budget;
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
            check_negative_balance(Decimal::from(5u64), Decimal::from(10u64)),
            Err(AccountInvariantError::NegativeBalance)
        ));
    }

    #[test]
    fn exact_deduction_passes() {
        assert!(check_negative_balance(Decimal::from(5u64), Decimal::from(5u64)).is_ok());
    }

    #[test]
    fn total_allocations_at_limit_passes() {
        assert!(check_total_allocations([MAX_ALLOCATION].into_iter()).is_ok());
    }

    #[test]
    fn total_allocations_over_limit_fails() {
        let total = u32::from(MAX_ALLOCATION) + 1;
        assert!(matches!(
            check_total_allocations([MAX_ALLOCATION, 1].into_iter()),
            Err(AccountInvariantError::TotalAllocations(t)) if t == total
        ));
    }

    #[test]
    fn spending_dipping_into_surplus_passes() {
        let balance = Decimal::from(1000u64);
        let allocation = MAX_ALLOCATION / 2;
        let spent = budget(balance, allocation) + Decimal::from(500u64);
        assert!(
            check_spending_within_budget(
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
            check_spending_within_budget(
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
        assert!(check_all_spending_within_budget(Decimal::from(1000u64), &categories).is_ok());
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
            check_all_spending_within_budget(Decimal::from(1000u64), &categories),
            Err(AccountInvariantError::SpendingExceedsAllocation(id, _, _, _))
                if id == category_id(1)
        ));
    }

    #[test]
    fn allocation_at_limit_passes() {
        assert!(check_allocation_limit(MAX_ALLOCATION).is_ok());
    }

    #[test]
    fn allocation_over_limit_fails() {
        assert!(matches!(
            check_allocation_limit(MAX_ALLOCATION + 1),
            Err(AccountInvariantError::AllocationLimit(_))
        ));
    }

    #[test]
    fn surplus_plus_budget_covering_spending_passes() {
        assert!(
            check_negative_surplus(
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
            check_negative_surplus(
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
            check_negative_surplus(
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
            check_transfer_within_surplus(
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
            check_transfer_within_surplus(
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
            check_transfer_within_surplus(
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
            check_transfer_within_surplus(
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
            check_transfer_within_surplus(
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
            check_transfer_within_surplus(
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
            check_transfer_within_surplus(
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
            check_debt_within_owed(&debt_id(0), Decimal::from(1000u64), Decimal::from(400u64))
                .is_ok()
        );
    }

    #[test]
    fn debt_repayment_exactly_owed_passes() {
        assert!(
            check_debt_within_owed(&debt_id(0), Decimal::from(1000u64), Decimal::from(1000u64))
                .is_ok()
        );
    }

    #[test]
    fn debt_repayment_exceeding_owed_fails() {
        assert!(matches!(
            check_debt_within_owed(
                &debt_id(0),
                Decimal::from(1000u64),
                Decimal::from(1000u64) + Decimal::new(1),
            ),
            Err(AccountInvariantError::DebtOverpayment(id, _, _)) if id == debt_id(0)
        ));
    }
}
