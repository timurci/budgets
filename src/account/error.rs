use thiserror::Error;

use crate::types::Decimal;

use super::model::{CategoryId, DebtId};
use super::spending::MAX_ALLOCATION;

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
