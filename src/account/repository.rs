use std::collections::HashMap;
use std::future::Future;

use thiserror::Error;

use super::model::{Account, AccountEvent, AccountId, AccountState};
use crate::types::Versioned;

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("storage failure: {0}")]
    Storage(String),

    #[error("concurrent modification: expected stream version {expected}, found {actual}")]
    ConcurrencyConflict { expected: u64, actual: u64 },
}

pub trait AccountRepository {
    fn load(
        &self,
        id: &AccountId,
    ) -> impl Future<Output = Result<Option<Versioned<Account>>, RepositoryError>> + Send;

    fn save(
        &mut self,
        account: &mut Versioned<Account>,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
}

#[derive(Debug, Default)]
pub struct InMemoryAccountRepository {
    streams: HashMap<AccountId, Vec<AccountEvent>>,
}

impl AccountRepository for InMemoryAccountRepository {
    async fn load(&self, id: &AccountId) -> Result<Option<Versioned<Account>>, RepositoryError> {
        let Some(events) = self.streams.get(id) else {
            return Ok(None);
        };
        let account = Account::reconstitute(AccountState::empty(*id), Some(events.clone()));
        Ok(Some(Versioned::at(events.len() as u64, account)))
    }

    async fn save(&mut self, account: &mut Versioned<Account>) -> Result<(), RepositoryError> {
        if account.value.events().is_empty() {
            return Ok(());
        }
        let actual = self
            .streams
            .get(account.value.id())
            .map_or(0, |events| events.len() as u64);
        if account.version != actual {
            return Err(RepositoryError::ConcurrencyConflict {
                expected: account.version,
                actual,
            });
        }
        let stream = self.streams.entry(*account.value.id()).or_default();
        stream.extend_from_slice(account.value.events());
        account.version = stream.len() as u64;
        account.value.mark_committed();
        Ok(())
    }
}

#[cfg(test)]
mod repository_tests {
    use super::*;
    use crate::types::Decimal;

    fn seeded_account() -> Versioned<Account> {
        let mut account = Versioned::new(Account::default());
        account.value.add_funds(Decimal::from(10000u64)).unwrap();
        let a = account.value.add_category(100).unwrap();
        account.value.add_category(50).unwrap();
        account.value.spend(&a, Decimal::from(1000u64)).unwrap();
        account.value.rollover(Decimal::from(5000u64)).unwrap();
        account.value.spend(&a, Decimal::from(6000u64)).unwrap();
        account
    }

    #[tokio::test]
    async fn load_empty_returns_none() {
        let repo = InMemoryAccountRepository::default();
        assert!(repo.load(&AccountId::new_v7()).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn save_then_load_roundtrips() {
        let mut account = seeded_account();
        let mut repo = InMemoryAccountRepository::default();
        repo.save(&mut account).await.unwrap();
        let loaded = repo.load(account.value.id()).await.unwrap().unwrap();
        assert_eq!(loaded.value.snapshot(), account.value.snapshot());
        assert_eq!(loaded.version, account.version);
    }

    #[tokio::test]
    async fn save_appends_only_new_events() {
        let mut account = seeded_account();
        let mut repo = InMemoryAccountRepository::default();
        repo.save(&mut account).await.unwrap();
        let version = account.version;
        account.value.add_funds(Decimal::from(500u64)).unwrap();
        repo.save(&mut account).await.unwrap();
        assert!(account.version > version);
        let loaded = repo.load(account.value.id()).await.unwrap().unwrap();
        assert_eq!(loaded.value.snapshot(), account.value.snapshot());
        assert_eq!(loaded.version, account.version);
    }

    #[tokio::test]
    async fn save_without_events_is_noop() {
        let mut account = seeded_account();
        let mut repo = InMemoryAccountRepository::default();
        repo.save(&mut account).await.unwrap();
        let version = account.version;
        repo.save(&mut account).await.unwrap();
        assert_eq!(account.version, version);
        assert!(repo.load(account.value.id()).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn stale_save_conflicts_and_keeps_events() {
        let mut account = seeded_account();
        let mut repo = InMemoryAccountRepository::default();
        repo.save(&mut account).await.unwrap();

        let mut first = repo.load(account.value.id()).await.unwrap().unwrap();
        let mut second = repo.load(account.value.id()).await.unwrap().unwrap();

        first.value.add_funds(Decimal::from(100u64)).unwrap();
        repo.save(&mut first).await.unwrap();

        second.value.add_funds(Decimal::from(200u64)).unwrap();
        let result = repo.save(&mut second).await;
        assert!(matches!(
            result,
            Err(RepositoryError::ConcurrencyConflict { .. })
        ));
        assert!(!second.value.events().is_empty());
    }

    #[tokio::test]
    async fn failed_save_does_not_create_stream() {
        let mut account = Versioned::at(1, Account::default());
        account.value.add_funds(Decimal::from(100u64)).unwrap();
        let mut repo = InMemoryAccountRepository::default();
        let result = repo.save(&mut account).await;
        assert!(matches!(
            result,
            Err(RepositoryError::ConcurrencyConflict {
                expected: 1,
                actual: 0
            })
        ));
        assert!(repo.load(account.value.id()).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn accounts_are_isolated() {
        let mut a = Versioned::new(Account::default());
        a.value.add_funds(Decimal::from(1000u64)).unwrap();
        let mut b = Versioned::new(Account::default());
        b.value.add_funds(Decimal::from(2000u64)).unwrap();

        let mut repo = InMemoryAccountRepository::default();
        repo.save(&mut a).await.unwrap();
        repo.save(&mut b).await.unwrap();

        let loaded_a = repo.load(a.value.id()).await.unwrap().unwrap();
        let loaded_b = repo.load(b.value.id()).await.unwrap().unwrap();
        assert_ne!(loaded_a.value.id(), loaded_b.value.id());
        assert_eq!(loaded_a.value.snapshot().balance, Decimal::from(1000u64));
        assert_eq!(loaded_b.value.snapshot().balance, Decimal::from(2000u64));
    }
}
