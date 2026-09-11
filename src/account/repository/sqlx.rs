use std::collections::HashMap;

use sqlx::database::Database;
use sqlx::{Pool, QueryBuilder, Transaction};
use uuid::Uuid;

use super::{AccountRepository, RepositoryError};
use crate::account::event::AccountEvent;
use crate::account::model::{Account, AccountId, AccountState};
use crate::types::Versioned;

#[derive(Debug)]
struct PendingAppend {
    id: AccountId,
    expected: u64,
    events: Vec<AccountEvent>,
}

pub struct SqlxAccountRepository<'c, DB: Database> {
    transaction: Transaction<'c, DB>,
    pending: Vec<PendingAppend>,
    /// Events already buffered (or overlaid) for the live aggregate of each
    /// stream, so repeated saves buffer only the new suffix.
    buffered: HashMap<AccountId, usize>,
}

impl<DB: Database> SqlxAccountRepository<'static, DB> {
    pub async fn begin(pool: &Pool<DB>) -> Result<Self, RepositoryError> {
        let transaction = pool.begin().await.map_err(storage)?;
        Ok(Self {
            transaction,
            pending: Vec::new(),
            buffered: HashMap::new(),
        })
    }
}

impl<'c, DB> SqlxAccountRepository<'c, DB>
where
    DB: Database,
    Uuid: sqlx::Type<DB> + for<'q> sqlx::Encode<'q, DB> + for<'r> sqlx::Decode<'r, DB>,
    i64: sqlx::Type<DB> + for<'q> sqlx::Encode<'q, DB> + for<'r> sqlx::Decode<'r, DB>,
    String: sqlx::Type<DB> + for<'q> sqlx::Encode<'q, DB> + for<'r> sqlx::Decode<'r, DB>,
    usize: sqlx::ColumnIndex<<DB as Database>::Row>,
    <DB as Database>::Arguments: sqlx::IntoArguments<DB>,
    for<'e> &'e mut <DB as Database>::Connection: sqlx::Executor<'e, Database = DB>,
{
    /// Persists every buffered append in order and commits the transaction.
    ///
    /// The passed aggregates are advanced and cleaned only after the commit
    /// succeeds; any error leaves their events and versions untouched.
    pub async fn commit(
        self,
        accounts: impl IntoIterator<Item = &mut Versioned<Account>>,
    ) -> Result<(), RepositoryError> {
        let accounts: Vec<&mut Versioned<Account>> = accounts.into_iter().collect();
        for append in &self.pending {
            if !accounts
                .iter()
                .any(|account| *account.value.id() == append.id)
            {
                return Err(RepositoryError::MissingAggregate(append.id));
            }
        }
        let Self {
            mut transaction,
            pending,
            ..
        } = self;
        for append in &pending {
            persist(&mut transaction, append).await?;
        }
        transaction.commit().await.map_err(storage)?;
        for account in accounts {
            let id = *account.value.id();
            let Some(last) = pending.iter().rev().find(|append| append.id == id) else {
                continue;
            };
            let committed: usize = pending
                .iter()
                .filter(|append| append.id == id)
                .map(|append| append.events.len())
                .sum();
            account.version = last.expected + last.events.len() as u64;
            account.value.mark_committed(committed);
        }
        Ok(())
    }

    /// Discards every buffered append and rolls back the transaction.
    pub async fn rollback(self) -> Result<(), RepositoryError> {
        self.transaction.rollback().await.map_err(storage)
    }

    async fn read_stream(
        &mut self,
        id: &AccountId,
    ) -> Result<Option<(u64, Vec<AccountEvent>)>, RepositoryError> {
        let mut query =
            QueryBuilder::<DB>::new("SELECT version FROM account_streams WHERE account_id = ");
        query.push_bind(id.as_uuid());
        let stored: Option<i64> = query
            .build_query_scalar()
            .fetch_optional(&mut *self.transaction)
            .await
            .map_err(storage)?;
        let Some(stored) = stored else {
            return Ok(None);
        };
        let version = version_to_u64(stored)?;

        let mut query =
            QueryBuilder::<DB>::new("SELECT event FROM account_events WHERE account_id = ");
        query.push_bind(id.as_uuid());
        query.push(" ORDER BY stream_version");
        let payloads: Vec<String> = query
            .build_query_scalar()
            .fetch_all(&mut *self.transaction)
            .await
            .map_err(storage)?;

        if payloads.len() as u64 != version {
            return Err(RepositoryError::Storage(format!(
                "stream {id} has version {version} but {} events",
                payloads.len()
            )));
        }

        let events = payloads
            .iter()
            .map(|payload| serde_json::from_str(payload).map_err(corrupt))
            .collect::<Result<Vec<AccountEvent>, RepositoryError>>()?;
        Ok(Some((version, events)))
    }
}

impl<'c, DB> AccountRepository for SqlxAccountRepository<'c, DB>
where
    DB: Database,
    Uuid: sqlx::Type<DB> + for<'q> sqlx::Encode<'q, DB> + for<'r> sqlx::Decode<'r, DB>,
    i64: sqlx::Type<DB> + for<'q> sqlx::Encode<'q, DB> + for<'r> sqlx::Decode<'r, DB>,
    String: sqlx::Type<DB> + for<'q> sqlx::Encode<'q, DB> + for<'r> sqlx::Decode<'r, DB>,
    usize: sqlx::ColumnIndex<<DB as Database>::Row>,
    <DB as Database>::Arguments: sqlx::IntoArguments<DB>,
    for<'e> &'e mut <DB as Database>::Connection: sqlx::Executor<'e, Database = DB>,
{
    async fn load(
        &mut self,
        id: &AccountId,
    ) -> Result<Option<Versioned<Account>>, RepositoryError> {
        let stored = self.read_stream(id).await?;
        let mut version = stored.as_ref().map_or(0, |(version, _)| *version);
        let mut events = stored.map_or_else(Vec::new, |(_, events)| events);
        if let Some(first) = self.pending.iter().find(|pending| pending.id == *id) {
            if first.expected != version {
                return Err(RepositoryError::ConcurrencyConflict {
                    expected: first.expected,
                    actual: version,
                });
            }
            for pending in self.pending.iter().filter(|pending| pending.id == *id) {
                version = pending.expected + pending.events.len() as u64;
                events.extend(pending.events.iter().cloned());
            }
        }
        self.buffered.insert(*id, 0);
        if events.is_empty() {
            return Ok(None);
        }
        let account = Account::reconstitute(AccountState::empty(*id), Some(events));
        Ok(Some(Versioned::at(version, account)))
    }

    async fn save(&mut self, account: &mut Versioned<Account>) -> Result<(), RepositoryError> {
        let id = *account.value.id();
        let buffered = self.buffered.get(&id).copied().unwrap_or(0);
        let events = account.value.events();
        let new_events = events.get(buffered..).ok_or_else(|| {
            RepositoryError::Storage(format!(
                "account {id} has fewer events than the buffered watermark"
            ))
        })?;
        if new_events.is_empty() {
            return Ok(());
        }
        let stored = current_version(&mut self.transaction, &id).await?;
        let base = self
            .pending
            .iter()
            .find(|pending| pending.id == id)
            .map_or(stored, |first| first.expected);
        let expected = self
            .pending
            .iter()
            .rev()
            .find(|pending| pending.id == id)
            .map_or(stored, |last| last.expected + last.events.len() as u64);
        if account.version != base && account.version != expected {
            return Err(RepositoryError::ConcurrencyConflict {
                expected: account.version,
                actual: stored,
            });
        }
        self.pending.push(PendingAppend {
            id,
            expected,
            events: new_events.to_vec(),
        });
        self.buffered.insert(id, events.len());
        Ok(())
    }
}

async fn current_version<DB>(
    transaction: &mut Transaction<'_, DB>,
    id: &AccountId,
) -> Result<u64, RepositoryError>
where
    DB: Database,
    Uuid: sqlx::Type<DB> + for<'q> sqlx::Encode<'q, DB>,
    i64: sqlx::Type<DB> + for<'r> sqlx::Decode<'r, DB>,
    usize: sqlx::ColumnIndex<<DB as Database>::Row>,
    <DB as Database>::Arguments: sqlx::IntoArguments<DB>,
    for<'e> &'e mut <DB as Database>::Connection: sqlx::Executor<'e, Database = DB>,
{
    let mut query =
        QueryBuilder::<DB>::new("SELECT version FROM account_streams WHERE account_id = ");
    query.push_bind(id.as_uuid());
    let version: Option<i64> = query
        .build_query_scalar()
        .fetch_optional(&mut **transaction)
        .await
        .map_err(storage)?;
    version.map_or(Ok(0), version_to_u64)
}

async fn persist<DB>(
    transaction: &mut Transaction<'_, DB>,
    append: &PendingAppend,
) -> Result<(), RepositoryError>
where
    DB: Database,
    Uuid: sqlx::Type<DB> + for<'q> sqlx::Encode<'q, DB>,
    i64: sqlx::Type<DB> + for<'q> sqlx::Encode<'q, DB> + for<'r> sqlx::Decode<'r, DB>,
    String: sqlx::Type<DB> + for<'q> sqlx::Encode<'q, DB>,
    usize: sqlx::ColumnIndex<<DB as Database>::Row>,
    <DB as Database>::Arguments: sqlx::IntoArguments<DB>,
    for<'e> &'e mut <DB as Database>::Connection: sqlx::Executor<'e, Database = DB>,
{
    let id = append.id;
    let expected = version_to_i64(append.expected)?;
    let appended = version_to_i64(append.events.len() as u64)?;

    let mut ensure =
        QueryBuilder::<DB>::new("INSERT INTO account_streams (account_id, version) VALUES (");
    ensure.push_bind(id.as_uuid()).push(", ").push_bind(0_i64);
    ensure.push(") ON CONFLICT (account_id) DO NOTHING");
    ensure
        .build()
        .execute(&mut **transaction)
        .await
        .map_err(storage)?;

    let mut bump = QueryBuilder::<DB>::new("UPDATE account_streams SET version = version + ");
    bump.push_bind(appended)
        .push(" WHERE account_id = ")
        .push_bind(id.as_uuid())
        .push(" AND version = ")
        .push_bind(expected)
        .push(" RETURNING version");
    let new_version: Option<i64> = bump
        .build_query_scalar()
        .fetch_optional(&mut **transaction)
        .await
        .map_err(storage)?;
    if new_version.is_none() {
        return Err(RepositoryError::ConcurrencyConflict {
            expected: append.expected,
            actual: current_version(transaction, &id).await?,
        });
    }

    let rows = append
        .events
        .iter()
        .enumerate()
        .map(|(offset, event)| {
            let version = expected + offset as i64 + 1;
            serde_json::to_string(event)
                .map(|payload| (version, payload))
                .map_err(corrupt)
        })
        .collect::<Result<Vec<(i64, String)>, RepositoryError>>()?;
    let mut insert =
        QueryBuilder::<DB>::new("INSERT INTO account_events (account_id, stream_version, event) ");
    insert.push_values(rows, |mut row, (version, payload)| {
        row.push_bind(id.as_uuid())
            .push_bind(version)
            .push_bind(payload);
    });
    insert
        .build()
        .execute(&mut **transaction)
        .await
        .map_err(storage)?;
    Ok(())
}

fn storage(error: sqlx::Error) -> RepositoryError {
    RepositoryError::Storage(error.to_string())
}

fn corrupt(error: serde_json::Error) -> RepositoryError {
    RepositoryError::Storage(format!("invalid event payload: {error}"))
}

fn version_to_i64(version: u64) -> Result<i64, RepositoryError> {
    i64::try_from(version)
        .map_err(|_| RepositoryError::Storage(format!("stream version {version} is out of range")))
}

fn version_to_u64(version: i64) -> Result<u64, RepositoryError> {
    u64::try_from(version)
        .map_err(|_| RepositoryError::Storage(format!("stream version {version} is out of range")))
}

#[cfg(all(test, any(feature = "sqlite", feature = "postgres")))]
mod tests {
    use std::collections::HashMap;

    #[cfg(feature = "sqlite")]
    use sqlx::SqlitePool;

    use super::*;
    use crate::types::Decimal;

    #[cfg(feature = "sqlite")]
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

    fn rich_account() -> Versioned<Account> {
        let mut account = Versioned::new(Account::default());
        account.value.add_funds(Decimal::from(10000u64)).unwrap();
        let a = account.value.add_category(100).unwrap();
        let b = account.value.add_category(50).unwrap();
        account.value.rollover(Decimal::ZERO).unwrap();
        account.value.spend(&a, Decimal::from(1000u64)).unwrap();
        account
            .value
            .transfer_surplus(&a, &b, Decimal::from(500u64))
            .unwrap();
        account
            .value
            .reallocate_categories(HashMap::from([(a, 80), (b, 70)]))
            .unwrap();
        let debt = account.value.add_debt(Decimal::from(400u64)).unwrap();
        account
            .value
            .repay_debt(&a, &debt, Decimal::from(100u64))
            .unwrap();
        account.value.remove_category(&b).unwrap();
        account.value.remove_funds(Decimal::from(100u64)).unwrap();
        account
    }

    #[cfg(feature = "sqlite")]
    async fn commit_save(pool: &SqlitePool, account: &mut Versioned<Account>) {
        let mut repo = SqlxAccountRepository::begin(pool).await.unwrap();
        repo.save(account).await.unwrap();
        repo.commit([account]).await.unwrap();
    }

    #[cfg(feature = "sqlite")]
    async fn tx_load(pool: &SqlitePool, id: &AccountId) -> Option<Versioned<Account>> {
        let mut repo = SqlxAccountRepository::begin(pool).await.unwrap();
        let loaded = repo.load(id).await.unwrap();
        repo.rollback().await.unwrap();
        loaded
    }

    #[cfg(feature = "sqlite")]
    #[sqlx::test(migrations = "migrations/sqlite")]
    async fn load_empty_returns_none(pool: SqlitePool) {
        assert!(tx_load(&pool, &AccountId::new_v7()).await.is_none());
    }

    #[cfg(feature = "sqlite")]
    #[sqlx::test(migrations = "migrations/sqlite")]
    async fn save_then_load_roundtrips(pool: SqlitePool) {
        let mut account = seeded_account();
        commit_save(&pool, &mut account).await;
        let loaded = tx_load(&pool, account.value.id()).await.unwrap();
        assert_eq!(loaded.value.snapshot(), account.value.snapshot());
        assert_eq!(loaded.version, account.version);
    }

    #[cfg(feature = "sqlite")]
    #[sqlx::test(migrations = "migrations/sqlite")]
    async fn roundtrips_every_event_variant(pool: SqlitePool) {
        let mut account = rich_account();
        let appended = account.value.events().len() as u64;
        assert_eq!(appended, 11);
        commit_save(&pool, &mut account).await;
        let loaded = tx_load(&pool, account.value.id()).await.unwrap();
        assert_eq!(loaded.value.snapshot(), account.value.snapshot());
        assert_eq!(loaded.version, appended);
    }

    #[cfg(feature = "sqlite")]
    #[sqlx::test(migrations = "migrations/sqlite")]
    async fn save_appends_only_new_events(pool: SqlitePool) {
        let mut account = seeded_account();
        commit_save(&pool, &mut account).await;
        let version = account.version;
        account.value.add_funds(Decimal::from(500u64)).unwrap();
        commit_save(&pool, &mut account).await;
        assert!(account.version > version);
        let loaded = tx_load(&pool, account.value.id()).await.unwrap();
        assert_eq!(loaded.value.snapshot(), account.value.snapshot());
        assert_eq!(loaded.version, account.version);
    }

    #[cfg(feature = "sqlite")]
    #[sqlx::test(migrations = "migrations/sqlite")]
    async fn save_without_events_is_noop(pool: SqlitePool) {
        let mut account = seeded_account();
        commit_save(&pool, &mut account).await;
        let version = account.version;
        commit_save(&pool, &mut account).await;
        assert_eq!(account.version, version);
        assert!(tx_load(&pool, account.value.id()).await.is_some());
    }

    #[cfg(feature = "sqlite")]
    #[sqlx::test(migrations = "migrations/sqlite")]
    async fn stale_save_conflicts_and_keeps_events(pool: SqlitePool) {
        let mut account = seeded_account();
        commit_save(&pool, &mut account).await;

        let mut first = tx_load(&pool, account.value.id()).await.unwrap();
        let mut second = tx_load(&pool, account.value.id()).await.unwrap();

        first.value.add_funds(Decimal::from(100u64)).unwrap();
        commit_save(&pool, &mut first).await;

        second.value.add_funds(Decimal::from(200u64)).unwrap();
        let mut repo = SqlxAccountRepository::begin(&pool).await.unwrap();
        let result = repo.save(&mut second).await;
        assert!(matches!(
            result,
            Err(RepositoryError::ConcurrencyConflict { .. })
        ));
        assert!(!second.value.events().is_empty());
        repo.rollback().await.unwrap();
    }

    #[cfg(feature = "sqlite")]
    #[sqlx::test(migrations = "migrations/sqlite")]
    async fn failed_save_does_not_create_stream(pool: SqlitePool) {
        let mut account = Versioned::at(1, Account::default());
        account.value.add_funds(Decimal::from(100u64)).unwrap();
        let mut repo = SqlxAccountRepository::begin(&pool).await.unwrap();
        let result = repo.save(&mut account).await;
        assert!(matches!(
            result,
            Err(RepositoryError::ConcurrencyConflict {
                expected: 1,
                actual: 0
            })
        ));
        assert!(repo.load(account.value.id()).await.unwrap().is_none());
        repo.rollback().await.unwrap();
    }

    #[cfg(feature = "sqlite")]
    #[sqlx::test(migrations = "migrations/sqlite")]
    async fn accounts_are_isolated(pool: SqlitePool) {
        let mut a = Versioned::new(Account::default());
        a.value.add_funds(Decimal::from(1000u64)).unwrap();
        let mut b = Versioned::new(Account::default());
        b.value.add_funds(Decimal::from(2000u64)).unwrap();

        commit_save(&pool, &mut a).await;
        commit_save(&pool, &mut b).await;

        let loaded_a = tx_load(&pool, a.value.id()).await.unwrap();
        let loaded_b = tx_load(&pool, b.value.id()).await.unwrap();
        assert_ne!(loaded_a.value.id(), loaded_b.value.id());
        assert_eq!(loaded_a.value.snapshot().balance, Decimal::from(1000u64));
        assert_eq!(loaded_b.value.snapshot().balance, Decimal::from(2000u64));
    }

    #[cfg(feature = "sqlite")]
    #[sqlx::test(migrations = "migrations/sqlite")]
    async fn rollback_discards_saved_events(pool: SqlitePool) {
        let mut account = seeded_account();
        let id = *account.value.id();
        let mut repo = SqlxAccountRepository::begin(&pool).await.unwrap();
        repo.save(&mut account).await.unwrap();
        repo.rollback().await.unwrap();
        assert!(!account.value.events().is_empty());
        assert!(tx_load(&pool, &id).await.is_none());
    }

    #[cfg(feature = "sqlite")]
    #[sqlx::test(migrations = "migrations/sqlite")]
    async fn load_overlays_pending_saves(pool: SqlitePool) {
        let mut account = seeded_account();
        commit_save(&pool, &mut account).await;
        let id = *account.value.id();
        let committed = account.version;

        let mut repo = SqlxAccountRepository::begin(&pool).await.unwrap();
        let mut loaded = repo.load(&id).await.unwrap().unwrap();
        loaded.value.add_funds(Decimal::from(500u64)).unwrap();
        repo.save(&mut loaded).await.unwrap();
        assert_eq!(loaded.version, committed);

        let reloaded = repo.load(&id).await.unwrap().unwrap();
        assert_eq!(reloaded.value.snapshot(), loaded.value.snapshot());
        assert_eq!(reloaded.version, committed + 1);
        repo.rollback().await.unwrap();

        let stored = tx_load(&pool, &id).await.unwrap();
        assert_eq!(stored.version, committed);
    }

    #[cfg(feature = "sqlite")]
    #[sqlx::test(migrations = "migrations/sqlite")]
    async fn load_over_pending_conflicts_when_stream_advanced(pool: SqlitePool) {
        let mut account = seeded_account();
        commit_save(&pool, &mut account).await;
        let id = *account.value.id();
        let committed = account.version;

        let mut repo = SqlxAccountRepository::begin(&pool).await.unwrap();
        let mut loaded = repo.load(&id).await.unwrap().unwrap();
        loaded.value.add_funds(Decimal::from(500u64)).unwrap();
        repo.save(&mut loaded).await.unwrap();

        let extra = AccountEvent::FundsAdded {
            amount: Decimal::from(1u64),
        };
        sqlx::query(
            "INSERT INTO account_events (account_id, stream_version, event) VALUES (?, ?, ?)",
        )
        .bind(id.as_uuid())
        .bind(committed as i64 + 1)
        .bind(serde_json::to_string(&extra).unwrap())
        .execute(&mut *repo.transaction)
        .await
        .unwrap();
        sqlx::query("UPDATE account_streams SET version = version + 1 WHERE account_id = ?")
            .bind(id.as_uuid())
            .execute(&mut *repo.transaction)
            .await
            .unwrap();

        let result = repo.load(&id).await;
        assert!(matches!(
            result,
            Err(RepositoryError::ConcurrencyConflict { expected, actual })
                if expected == committed && actual == committed + 1
        ));
        repo.rollback().await.unwrap();
    }

    #[cfg(feature = "sqlite")]
    #[sqlx::test(migrations = "migrations/sqlite")]
    async fn successive_saves_commit_together(pool: SqlitePool) {
        let mut account = seeded_account();
        commit_save(&pool, &mut account).await;
        let id = *account.value.id();

        let mut repo = SqlxAccountRepository::begin(&pool).await.unwrap();
        let mut loaded = repo.load(&id).await.unwrap().unwrap();
        loaded.value.add_funds(Decimal::from(500u64)).unwrap();
        repo.save(&mut loaded).await.unwrap();
        loaded.value.add_funds(Decimal::from(300u64)).unwrap();
        repo.save(&mut loaded).await.unwrap();
        repo.commit([&mut loaded]).await.unwrap();

        assert!(loaded.value.events().is_empty());
        let stored = tx_load(&pool, &id).await.unwrap();
        assert_eq!(stored.value.snapshot(), loaded.value.snapshot());
        assert_eq!(stored.version, loaded.version);
    }

    #[cfg(feature = "sqlite")]
    #[sqlx::test(migrations = "migrations/sqlite")]
    async fn failed_commit_keeps_aggregate_events(pool: SqlitePool) {
        let mut account = Versioned::new(Account::default());
        let id = *account.value.id();
        account.value.add_funds(Decimal::from(100u64)).unwrap();

        sqlx::query("INSERT INTO account_streams (account_id, version) VALUES (?, ?)")
            .bind(id.as_uuid())
            .bind(0_i64)
            .execute(&pool)
            .await
            .unwrap();
        let conflicting = AccountEvent::FundsAdded {
            amount: Decimal::from(1u64),
        };
        sqlx::query(
            "INSERT INTO account_events (account_id, stream_version, event) VALUES (?, ?, ?)",
        )
        .bind(id.as_uuid())
        .bind(1_i64)
        .bind(serde_json::to_string(&conflicting).unwrap())
        .execute(&pool)
        .await
        .unwrap();

        let mut repo = SqlxAccountRepository::begin(&pool).await.unwrap();
        repo.save(&mut account).await.unwrap();
        let result = repo.commit([&mut account]).await;
        assert!(matches!(result, Err(RepositoryError::Storage(_))));
        assert_eq!(account.version, 0);
        assert!(!account.value.events().is_empty());
    }

    #[cfg(feature = "sqlite")]
    #[sqlx::test(migrations = "migrations/sqlite")]
    async fn commit_missing_aggregate_is_rejected(pool: SqlitePool) {
        let mut account = Versioned::new(Account::default());
        let id = *account.value.id();
        account.value.add_funds(Decimal::from(100u64)).unwrap();
        let mut other = Versioned::new(Account::default());

        let mut repo = SqlxAccountRepository::begin(&pool).await.unwrap();
        repo.save(&mut account).await.unwrap();
        let result = repo.commit([&mut other]).await;
        assert!(matches!(
            result,
            Err(RepositoryError::MissingAggregate(missing)) if missing == id
        ));
        assert_eq!(account.version, 0);
        assert!(!account.value.events().is_empty());
    }

    #[cfg(feature = "sqlite")]
    #[sqlx::test(migrations = "migrations/sqlite")]
    async fn load_rejects_stream_version_mismatch(pool: SqlitePool) {
        let id = AccountId::new_v7();
        let event = AccountEvent::FundsAdded {
            amount: Decimal::from(1u64),
        };
        sqlx::query("INSERT INTO account_streams (account_id, version) VALUES (?, ?)")
            .bind(id.as_uuid())
            .bind(2_i64)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO account_events (account_id, stream_version, event) VALUES (?, ?, ?)",
        )
        .bind(id.as_uuid())
        .bind(1_i64)
        .bind(serde_json::to_string(&event).unwrap())
        .execute(&pool)
        .await
        .unwrap();

        let mut repo = SqlxAccountRepository::begin(&pool).await.unwrap();
        let result = repo.load(&id).await;
        assert!(matches!(result, Err(RepositoryError::Storage(_))));
        repo.rollback().await.unwrap();
    }

    fn is_postgres_url(url: &str) -> bool {
        url.starts_with("postgres://") || url.starts_with("postgresql://")
    }

    #[test]
    fn recognizes_postgres_urls() {
        assert!(is_postgres_url("postgres://user:pass@localhost/db"));
        assert!(is_postgres_url("postgresql://localhost/db"));
        assert!(!is_postgres_url("sqlite://data.db"));
        assert!(!is_postgres_url(""));
    }

    #[cfg(feature = "postgres")]
    #[tokio::test]
    async fn postgres_roundtrips_when_database_url_is_set() {
        let Some(url) = std::env::var("DATABASE_URL")
            .ok()
            .filter(|url| is_postgres_url(url))
        else {
            return;
        };
        let pool = sqlx::PgPool::connect(&url).await.unwrap();
        sqlx::migrate!("migrations/postgres")
            .run(&pool)
            .await
            .unwrap();

        let mut account = rich_account();
        let id = *account.value.id();
        {
            let mut repo = SqlxAccountRepository::begin(&pool).await.unwrap();
            repo.save(&mut account).await.unwrap();
            repo.commit([&mut account]).await.unwrap();
        }
        let mut repo = SqlxAccountRepository::begin(&pool).await.unwrap();
        let loaded = repo.load(&id).await.unwrap().unwrap();
        assert_eq!(loaded.value.snapshot(), account.value.snapshot());
        assert_eq!(loaded.version, account.version);
        repo.rollback().await.unwrap();

        let mut tx = pool.begin().await.unwrap();
        sqlx::query("DELETE FROM account_events WHERE account_id = $1")
            .bind(id.as_uuid())
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("DELETE FROM account_streams WHERE account_id = $1")
            .bind(id.as_uuid())
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }
}
