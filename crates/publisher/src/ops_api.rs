//! Private, authenticated publisher operator endpoints.

mod response;

use std::sync::Arc;

use axum::extract::{Path, Query, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use db::login::{AccountId, ops};
use infra::config::Secret;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

struct OpsState {
    pool: db::Pool,
    key: Secret,
}

#[derive(Clone)]
struct Operator(String);

pub(super) fn router(pool: db::Pool, key: Secret) -> Router {
    let state = Arc::new(OpsState { pool, key });
    Router::new()
        .route("/v1/accounts", get(accounts))
        .route("/v1/accounts/{id}", get(detail))
        .route("/v1/accounts/{id}/status", post(set_status))
        .route("/v1/audit", get(audit))
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
    let matches = request.headers().get("x-ops-key").is_some_and(|key| {
        crate::credentials::key_matches(state.key.expose().as_bytes(), key.as_bytes())
    });
    if !matches {
        return failure(StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    let name = request
        .headers()
        .get("x-operator")
        .and_then(|header| header.to_str().ok())
        .map(str::trim);
    let Some(name) = name.filter(|name| !name.is_empty() && name.len() <= 128) else {
        return failure(StatusCode::BAD_REQUEST, "operator_required").into_response();
    };
    let operator = Operator(name.to_owned());
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

    fn window(&self) -> Result<ops::Page, OpsError> {
        let offset = self
            .page
            .unwrap_or(1)
            .checked_sub(1)
            .ok_or_else(|| failure(StatusCode::BAD_REQUEST, "invalid_page"))?;
        Ok(ops::Page {
            limit: 26,
            offset: i64::from(offset) * 25,
        })
    }
}

async fn accounts(
    State(state): State<Arc<OpsState>>,
    Query(query): Query<Listing>,
) -> Result<Json<Value>, OpsError> {
    let mut rows = ops::accounts(&state.pool, query.search.as_deref(), query.window()?)
        .await
        .map_err(OpsError::Database)?;
    let has_next = rows.len() > Listing::PAGE_SIZE;
    rows.truncate(Listing::PAGE_SIZE);
    Ok(Json(
        json!({"accounts": rows.iter().map(response::account).collect::<Vec<_>>(), "has_next": has_next}),
    ))
}

async fn audit(
    State(state): State<Arc<OpsState>>,
    Query(query): Query<Listing>,
) -> Result<Json<Value>, OpsError> {
    let mut rows = ops::audit(&state.pool, query.window()?)
        .await
        .map_err(OpsError::Database)?;
    let has_next = rows.len() > Listing::PAGE_SIZE;
    rows.truncate(Listing::PAGE_SIZE);
    Ok(Json(
        json!({"records": rows.iter().map(response::audit_record).collect::<Vec<_>>(), "has_next": has_next}),
    ))
}

async fn detail(
    State(state): State<Arc<OpsState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, OpsError> {
    let detail = ops::detail(&state.pool, &AccountId::new(id.to_string()))
        .await
        .map_err(OpsError::Database)?
        .ok_or_else(|| failure(StatusCode::NOT_FOUND, "account_not_found"))?;
    Ok(Json(response::detail(detail)))
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
    Extension(operator): Extension<Operator>,
    Path(id): Path<Uuid>,
    Json(change): Json<Change>,
) -> Result<Json<Value>, OpsError> {
    let status = match change.status {
        Status::Active => ops::SanctionStatus::Active,
        Status::Sanctioned => ops::SanctionStatus::Sanctioned,
    };
    let reason = change.reason.trim();
    if reason.is_empty() || reason.chars().count() > 1000 || reason.contains('\0') {
        return Err(failure(StatusCode::BAD_REQUEST, "reason_required"));
    }
    let id = AccountId::new(id.to_string());
    let change = ops::StatusChange {
        status,
        operator: &operator.0,
        reason,
    };
    match ops::set_status(&state.pool, &id, &change)
        .await
        .map_err(OpsError::Database)?
    {
        ops::StatusOutcome::Changed => Ok(Json(json!({"changed": true}))),
        ops::StatusOutcome::Unchanged => Ok(Json(json!({"changed": false}))),
        ops::StatusOutcome::NotFound => Err(failure(StatusCode::NOT_FOUND, "account_not_found")),
        ops::StatusOutcome::Closing => Err(failure(StatusCode::CONFLICT, "account_closing")),
    }
}

enum OpsError {
    Rejected(StatusCode, &'static str),
    Database(db::DbError),
}

impl IntoResponse for OpsError {
    fn into_response(self) -> Response {
        let (status, code) = match self {
            Self::Rejected(status, code) => (status, code),
            Self::Database(error) => {
                tracing::error!(%error, "publisher operator database operation failed");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
            }
        };
        (status, Json(json!({"error": code}))).into_response()
    }
}

const fn failure(status: StatusCode, code: &'static str) -> OpsError {
    OpsError::Rejected(status, code)
}
