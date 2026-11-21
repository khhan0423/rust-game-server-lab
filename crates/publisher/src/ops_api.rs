//! Private, authenticated publisher operator endpoints.

mod response;

use std::sync::Arc;

use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{Path, Query, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use db::login::{
    AccountId,
    ops::{self, AttributionError, AuditReason, OperatorName},
};
use infra::http::api::{ApiError, ApiJson, ApiKey, operator::NAME_HEADER};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

struct OpsState {
    pool: db::Pool,
    key: ApiKey,
}

pub(super) fn router(pool: db::Pool, key: ApiKey) -> Router {
    let state = Arc::new(OpsState { pool, key });
    Router::new()
        .route("/v1/accounts", get(accounts))
        .route("/v1/accounts/{id}", get(detail))
        .route("/v1/accounts/{id}/status", post(set_status))
        .route("/v1/audit", get(audit))
        .fallback(ApiError::not_found)
        .method_not_allowed_fallback(ApiError::method_not_allowed)
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            authenticate,
        ))
        .with_state(state)
}

async fn authenticate(
    State(state): State<Arc<OpsState>>,
    mut request: Request,
    next: Next,
) -> Response {
    if let Err(error) = state.key.verify(request.headers()) {
        return error.into_response();
    }
    let mut names = request.headers().get_all(NAME_HEADER).iter();
    let value = names
        .next()
        .and_then(|v| v.to_str().ok())
        .filter(|_| names.next().is_none())
        .unwrap_or_default();
    let operator = match OperatorName::try_from(value) {
        Ok(value) => value,
        Err(error) => return attribution_error(error).into_response(),
    };
    request.extensions_mut().insert(operator);
    next.run(request).await
}

#[derive(Default, Deserialize)]
struct Listing {
    page: Option<u32>,
    search: Option<String>,
}

impl Listing {
    const PAGE_SIZE: usize = 25;

    fn window(&self) -> Result<ops::Page, ApiError> {
        ops::Page::new(self.page.unwrap_or(1), Self::PAGE_SIZE)
            .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "invalid_page"))
    }

    fn project<T, U>(mut rows: Vec<T>) -> (Vec<U>, bool)
    where
        U: From<T>,
    {
        let has_next = rows.len() > Self::PAGE_SIZE;
        rows.truncate(Self::PAGE_SIZE);
        (rows.into_iter().map(Into::into).collect(), has_next)
    }
}

#[derive(Serialize)]
struct Accounts {
    accounts: Vec<response::Account>,
    has_next: bool,
}
#[derive(Serialize)]
struct Audit {
    records: Vec<response::Record>,
    has_next: bool,
}
#[derive(Serialize)]
struct Changed {
    changed: bool,
}

async fn accounts(
    State(state): State<Arc<OpsState>>,
    query: Result<Query<Listing>, QueryRejection>,
) -> Result<Json<Accounts>, ApiError> {
    let Query(query) = query.map_err(|_| ApiError::INVALID)?;
    let rows = ops::accounts(&state.pool, query.search.as_deref(), query.window()?)
        .await
        .map_err(ApiError::internal)?;
    let (accounts, has_next) = Listing::project(rows);
    Ok(Json(Accounts { accounts, has_next }))
}

async fn audit(
    State(state): State<Arc<OpsState>>,
    query: Result<Query<Listing>, QueryRejection>,
) -> Result<Json<Audit>, ApiError> {
    let Query(query) = query.map_err(|_| ApiError::INVALID)?;
    let rows = ops::audit(&state.pool, query.window()?)
        .await
        .map_err(ApiError::internal)?;
    let (records, has_next) = Listing::project(rows);
    Ok(Json(Audit { records, has_next }))
}

async fn detail(
    State(state): State<Arc<OpsState>>,
    id: Result<Path<Uuid>, PathRejection>,
) -> Result<Json<response::Detail>, ApiError> {
    let Path(id) = id.map_err(|_| ApiError::INVALID)?;
    let detail = ops::detail(&state.pool, &AccountId::new(id))
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "account_not_found"))?;
    Ok(Json(detail.into()))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Active,
    Sanctioned,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Change {
    status: Status,
    reason: String,
}

async fn set_status(
    State(state): State<Arc<OpsState>>,
    Extension(operator): Extension<OperatorName>,
    id: Result<Path<Uuid>, PathRejection>,
    change: ApiJson<Change>,
) -> Result<Json<Changed>, ApiError> {
    let Path(id) = id.map_err(|_| ApiError::INVALID)?;
    let change = change.into_inner();
    let status = match change.status {
        Status::Active => ops::SanctionStatus::Active,
        Status::Sanctioned => ops::SanctionStatus::Sanctioned,
    };
    let reason = AuditReason::try_from(change.reason.as_str()).map_err(attribution_error)?;
    let id = AccountId::new(id);
    let change = ops::StatusChange::new(status, &operator, reason);
    match ops::set_status(&state.pool, &id, &change)
        .await
        .map_err(ApiError::internal)?
    {
        ops::StatusOutcome::Changed => Ok(Json(Changed { changed: true })),
        ops::StatusOutcome::Unchanged => Ok(Json(Changed { changed: false })),
        ops::StatusOutcome::NotFound => {
            Err(ApiError::new(StatusCode::NOT_FOUND, "account_not_found"))
        }
        ops::StatusOutcome::Closing => Err(ApiError::new(StatusCode::CONFLICT, "account_closing")),
    }
}

fn attribution_error(error: AttributionError) -> ApiError {
    let code = match error {
        AttributionError::Operator => "operator_required",
        AttributionError::Reason => "reason_required",
    };
    ApiError::new(StatusCode::BAD_REQUEST, code)
}
