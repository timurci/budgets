use std::collections::HashMap;

use crate::types::Decimal;

use super::error::AccountPreconditionError;
use super::model::{CategoryId, DebtId};
use super::spending::Spending;

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

#[cfg(test)]
mod tests {
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
            check_category_exists(&category_id(1), &categories),
            Err(AccountPreconditionError::CategoryNotFound(id)) if id == category_id(1)
        ));
    }

    #[test]
    fn existing_category_passes() {
        let categories = categories(&["a"]);
        assert!(check_category_exists(&category_id(0), &categories).is_ok());
    }

    #[test]
    fn same_source_and_target_fails() {
        assert!(matches!(
            check_distinct_source_target(&category_id(0), &category_id(0)),
            Err(AccountPreconditionError::SameCategory(id)) if id == category_id(0)
        ));
    }

    #[test]
    fn distinct_source_and_target_passes() {
        assert!(check_distinct_source_target(&category_id(0), &category_id(1)).is_ok());
    }

    #[test]
    fn missing_transfer_source_fails() {
        let categories = categories(&["b"]);
        assert!(matches!(
            check_source_exists(&category_id(1), &categories),
            Err(AccountPreconditionError::SourceCategoryNotFound(id)) if id == category_id(1)
        ));
    }

    #[test]
    fn missing_transfer_target_fails() {
        let categories = categories(&["a"]);
        assert!(matches!(
            check_target_exists(&category_id(1), &categories),
            Err(AccountPreconditionError::TargetCategoryNotFound(id)) if id == category_id(1)
        ));
    }

    #[test]
    fn mismatched_allocation_count_fails() {
        let categories = categories(&["a"]);
        let allocations = HashMap::from([(category_id(0), 10), (category_id(1), 10)]);
        assert!(matches!(
            check_categories_match(&categories, &allocations),
            Err(AccountPreconditionError::UnmatchedCategories)
        ));
    }

    #[test]
    fn mismatched_allocation_names_fails() {
        let categories = categories(&["a"]);
        let allocations = HashMap::from([(category_id(1), 10)]);
        assert!(matches!(
            check_categories_match(&categories, &allocations),
            Err(AccountPreconditionError::UnmatchedCategories)
        ));
    }

    #[test]
    fn matching_allocations_pass() {
        let categories = categories(&["a", "b"]);
        let allocations = HashMap::from([(category_id(0), 10), (category_id(1), 20)]);
        assert!(check_categories_match(&categories, &allocations).is_ok());
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
            check_debt_exists(&debt_id(1), &debts),
            Err(AccountPreconditionError::DebtNotFound(id)) if id == debt_id(1)
        ));
    }

    #[test]
    fn existing_debt_passes() {
        let debts = debts(&["loan"]);
        assert!(check_debt_exists(&debt_id(0), &debts).is_ok());
    }
}
