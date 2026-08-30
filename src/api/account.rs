//! Authenticated account balance endpoint used by the API playground.

use axum::Json;
use axum::extract::{Extension, State};
use serde::Serialize;

use crate::billing::BillingIdentity;
use crate::error::AppError;
use crate::linkedin::client::Upstream;
use crate::state::AppState;

#[derive(Serialize)]
pub struct AccountResponse {
    pub success: bool,
    pub email: String,
    pub balance_cents: i64,
    pub cache_hit_cost_cents: i64,
    pub cache_miss_cost_cents: i64,
}

pub async fn get_account<U: Upstream>(
    State(state): State<std::sync::Arc<AppState<U>>>,
    identity: Option<Extension<BillingIdentity>>,
) -> Result<Json<AccountResponse>, AppError> {
    let identity = identity.ok_or(AppError::BillingUnavailable)?.0;
    let account = state.billing.account(&identity).await?;
    Ok(Json(AccountResponse {
        success: true,
        email: account.email,
        balance_cents: account.balance_cents,
        cache_hit_cost_cents: state.settings.cache_hit_cost_cents,
        cache_miss_cost_cents: state.settings.cache_miss_cost_cents,
    }))
}
