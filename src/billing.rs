//! Redis-backed API-key authentication and atomic credit accounting.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::AppError;

#[derive(Debug, Clone, Deserialize)]
pub struct SeedAccount {
    pub email: String,
    pub api_key: String,
    pub initial_credit_cents: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Account {
    pub email: String,
    pub balance_cents: i64,
}

#[derive(Debug, Clone)]
pub struct BillingIdentity {
    pub redis_key: String,
    pub email: String,
}

#[derive(Debug, Clone)]
pub struct BillingReservation {
    pub identity: BillingIdentity,
    pub reserved_cents: i64,
}

#[derive(Clone)]
pub enum BillingStore {
    Disabled,
    Redis(redis::aio::MultiplexedConnection),
}

impl BillingStore {
    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        let client = redis::Client::open(url)?;
        let connection = client.get_multiplexed_async_connection().await?;
        Ok(Self::Redis(connection))
    }

    pub fn enabled(&self) -> bool {
        matches!(self, Self::Redis(_))
    }

    pub async fn seed_from_file(&self, path: &str) -> anyhow::Result<usize> {
        let raw = tokio::fs::read_to_string(path).await?;
        let accounts: Vec<SeedAccount> = serde_json::from_str(&raw)?;
        let mut connection = self.connection()?;

        for account in &accounts {
            if !account.api_key.starts_with("tross_sk_") || account.initial_credit_cents < 0 {
                anyhow::bail!("invalid account seed for {}", account.email);
            }
            let redis_key = account_key(&account.api_key);
            let _: () = redis::pipe()
                .atomic()
                .cmd("HSETNX")
                .arg(&redis_key)
                .arg("email")
                .arg(&account.email)
                .ignore()
                .cmd("HSETNX")
                .arg(&redis_key)
                .arg("balance_cents")
                .arg(account.initial_credit_cents)
                .ignore()
                .cmd("HSETNX")
                .arg(&redis_key)
                .arg("enabled")
                .arg(1)
                .ignore()
                .query_async(&mut connection)
                .await?;
        }
        Ok(accounts.len())
    }

    pub async fn account(&self, identity: &BillingIdentity) -> Result<Account, AppError> {
        let mut connection = self.connection_for_request()?;
        let (email, balance): (Option<String>, Option<i64>) = redis::pipe()
            .cmd("HGET")
            .arg(&identity.redis_key)
            .arg("email")
            .cmd("HGET")
            .arg(&identity.redis_key)
            .arg("balance_cents")
            .query_async(&mut connection)
            .await
            .map_err(redis_unavailable)?;
        match (email, balance) {
            (Some(email), Some(balance_cents)) => Ok(Account {
                email,
                balance_cents,
            }),
            _ => Err(AppError::ApiKeyInvalid),
        }
    }

    pub async fn authenticate(&self, api_key: &str) -> Result<BillingIdentity, AppError> {
        if !api_key.starts_with("tross_sk_") {
            return Err(AppError::ApiKeyInvalid);
        }
        let redis_key = account_key(api_key);
        let mut connection = self.connection_for_request()?;
        let (email, enabled): (Option<String>, Option<i64>) = redis::pipe()
            .cmd("HGET")
            .arg(&redis_key)
            .arg("email")
            .cmd("HGET")
            .arg(&redis_key)
            .arg("enabled")
            .query_async(&mut connection)
            .await
            .map_err(redis_unavailable)?;
        match (email, enabled) {
            (Some(email), Some(1)) => Ok(BillingIdentity { redis_key, email }),
            _ => Err(AppError::ApiKeyInvalid),
        }
    }

    pub async fn reserve(
        &self,
        identity: BillingIdentity,
        cents: i64,
    ) -> Result<BillingReservation, AppError> {
        let script = redis::Script::new(
            r#"
local balance = tonumber(redis.call('HGET', KEYS[1], 'balance_cents'))
if not balance then return -1 end
if balance < tonumber(ARGV[1]) then return -2 end
return redis.call('HINCRBY', KEYS[1], 'balance_cents', -tonumber(ARGV[1]))
"#,
        );
        let mut connection = self.connection_for_request()?;
        let balance: i64 = script
            .key(&identity.redis_key)
            .arg(cents)
            .invoke_async(&mut connection)
            .await
            .map_err(redis_unavailable)?;
        match balance {
            -1 => Err(AppError::ApiKeyInvalid),
            -2 => {
                let account = self.account(&identity).await?;
                Err(AppError::InsufficientCredit {
                    balance_cents: account.balance_cents,
                    required_cents: cents,
                })
            }
            _ => Ok(BillingReservation {
                identity,
                reserved_cents: cents,
            }),
        }
    }

    pub async fn settle(
        &self,
        reservation: &BillingReservation,
        actual_cents: i64,
    ) -> Result<i64, AppError> {
        let delta = reservation.reserved_cents - actual_cents;
        let script = redis::Script::new(
            r#"
local balance = tonumber(redis.call('HGET', KEYS[1], 'balance_cents'))
if not balance then return -1 end
local delta = tonumber(ARGV[1])
if delta < 0 and balance < -delta then
  redis.call('HINCRBY', KEYS[1], 'balance_cents', tonumber(ARGV[2]))
  return -2
end
return redis.call('HINCRBY', KEYS[1], 'balance_cents', delta)
"#,
        );
        let mut connection = self.connection_for_request()?;
        let balance: i64 = script
            .key(&reservation.identity.redis_key)
            .arg(delta)
            .arg(reservation.reserved_cents)
            .invoke_async(&mut connection)
            .await
            .map_err(redis_unavailable)?;
        match balance {
            -1 => Err(AppError::ApiKeyInvalid),
            -2 => {
                let account = self.account(&reservation.identity).await?;
                Err(AppError::InsufficientCredit {
                    balance_cents: account.balance_cents,
                    required_cents: actual_cents,
                })
            }
            _ => Ok(balance),
        }
    }

    fn connection(&self) -> anyhow::Result<redis::aio::MultiplexedConnection> {
        match self {
            Self::Redis(connection) => Ok(connection.clone()),
            Self::Disabled => anyhow::bail!("billing is disabled"),
        }
    }

    fn connection_for_request(&self) -> Result<redis::aio::MultiplexedConnection, AppError> {
        match self {
            Self::Redis(connection) => Ok(connection.clone()),
            Self::Disabled => Err(AppError::BillingUnavailable),
        }
    }
}

fn account_key(api_key: &str) -> String {
    format!(
        "tross:account:{}",
        hex::encode(Sha256::digest(api_key.as_bytes()))
    )
}

fn redis_unavailable(error: redis::RedisError) -> AppError {
    tracing::error!(error = %error, "billing.redis_unavailable");
    AppError::BillingUnavailable
}
