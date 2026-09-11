mod inmem;

#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub mod sqlx;

use std::future::Future;

use thiserror::Error;

use super::model::{Account, AccountId};
use crate::types::Versioned;

pub use inmem::InMemoryAccountRepository;

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("storage failure: {0}")]
    Storage(String),

    #[error("concurrent modification: expected stream version {expected}, found {actual}")]
    ConcurrencyConflict { expected: u64, actual: u64 },

    #[error("commit is missing the aggregate for account {0}")]
    MissingAggregate(AccountId),
}

pub trait AccountRepository {
    fn load(
        &mut self,
        id: &AccountId,
    ) -> impl Future<Output = Result<Option<Versioned<Account>>, RepositoryError>> + Send;

    fn save(
        &mut self,
        account: &mut Versioned<Account>,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
}
