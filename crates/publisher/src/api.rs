//! HTTP boundary for guest accounts, login, and single-use ticket verification.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{State, rejection::JsonRejection};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use db::login::{
    self, AccountId, AccountStatus, EntryGrant, GameId, GuestCredentials, GuestId, GuestSecret,
    LoginOutcome, MembershipStatus, Ticket,
};
use infra::config::Secret;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::config::GameConfig;
use crate::credentials::{decode, encode, key_matches, random_bytes};

pub(super) struct AppState {
    pub(super) pool: db::Pool,
    pub(super) games: HashMap<GameId, GameConfig>,
    pub(super) ttl_secs: u32,
    pub(super) key: Secret,
}

pub(super) fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/guest/accounts", post(create_guest))
        .route("/v1/guest/login", post(login_guest))
        .route("/v1/tickets/verify", post(verify_ticket))
        .with_state(state)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateRequest {}

#[derive(Serialize)]
struct Created {
    account_id: String,
    guest_id: String,
    secret: String,
}

#[derive(Deserialize)]
struct LoginRequest {
    guest_id: String,
    secret: String,
    game_id: String,
}

#[derive(Serialize)]
struct LoggedIn {
    account_id: String,
    ticket: String,
    expires_in_secs: u32,
    entry_url: String,
}

#[derive(Deserialize)]
struct VerifyRequest {
    ticket: String,
    game_id: String,
}

#[derive(Serialize)]
struct Verified {
    account_id: String,
    membership: &'static str,
    sanction: &'static str,
}

#[derive(Serialize)]
struct ErrorBody {
    error: &'static str,
}

enum ApiError {
    InvalidRequest,
    InvalidCredentials,
    Unauthorized,
    AccountBlocked,
    UnknownGame,
    TicketInvalid,
    Database(db::DbError),
    Random(rand::rngs::SysError),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error) = match self {
            Self::InvalidRequest => (StatusCode::BAD_REQUEST, "invalid_request"),
            Self::InvalidCredentials => (StatusCode::UNAUTHORIZED, "invalid_credentials"),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Self::AccountBlocked => (StatusCode::FORBIDDEN, "account_blocked"),
            Self::UnknownGame => (StatusCode::NOT_FOUND, "unknown_game"),
            Self::TicketInvalid => (StatusCode::NOT_FOUND, "ticket_invalid"),
            Self::Database(error) => {
                // Database error details can include values bound to a query.
                tracing::error!(%error, "publisher database operation failed");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
            }
            Self::Random(error) => {
                tracing::error!(%error, "credential generation failed");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
            }
        };
        (status, Json(ErrorBody { error })).into_response()
    }
}

impl From<JsonRejection> for ApiError {
    fn from(_: JsonRejection) -> Self {
        Self::InvalidRequest
    }
}

fn parse_body<T: DeserializeOwned>(
    request: Result<Json<Map<String, Value>>, JsonRejection>,
) -> Result<T, ApiError> {
    let Json(object) = request?;
    serde_json::from_value(Value::Object(object)).map_err(|_| ApiError::InvalidRequest)
}

async fn create_guest(
    State(state): State<Arc<AppState>>,
    request: Result<Json<Map<String, Value>>, JsonRejection>,
) -> Result<(StatusCode, Json<Created>), ApiError> {
    let CreateRequest {} = parse_body(request)?;
    let account_id = AccountId::new(Uuid::now_v7().to_string());
    let guest_id = GuestId::new(Uuid::now_v7().to_string());
    let secret = GuestSecret::new(random_bytes().map_err(ApiError::Random)?);
    login::create_guest(
        &state.pool,
        &account_id,
        &GuestCredentials {
            guest_id: &guest_id,
            secret: &secret,
        },
    )
    .await
    .map_err(ApiError::Database)?;
    tracing::info!(account_id = account_id.as_str(), "guest account created");
    Ok((
        StatusCode::CREATED,
        Json(Created {
            account_id: account_id.as_str().to_owned(),
            guest_id: guest_id.as_str().to_owned(),
            secret: encode(secret.expose()),
        }),
    ))
}

async fn login_guest(
    State(state): State<Arc<AppState>>,
    request: Result<Json<Map<String, Value>>, JsonRejection>,
) -> Result<Json<LoggedIn>, ApiError> {
    let request: LoginRequest = parse_body(request)?;
    let guest_id = Uuid::parse_str(&request.guest_id).map_err(|_| ApiError::InvalidRequest)?;
    let guest_id = GuestId::new(guest_id.to_string());
    let secret = GuestSecret::new(decode(&request.secret).ok_or(ApiError::InvalidRequest)?);
    let (game_id, game) = state
        .games
        .get_key_value(request.game_id.as_str())
        .ok_or(ApiError::UnknownGame)?;
    let ticket = Ticket::new(random_bytes().map_err(ApiError::Random)?);
    let outcome = login::login_guest(
        &state.pool,
        &GuestCredentials {
            guest_id: &guest_id,
            secret: &secret,
        },
        &EntryGrant {
            game_id,
            ticket: &ticket,
            ttl_secs: state.ttl_secs,
        },
    )
    .await
    .map_err(ApiError::Database)?;
    let account_id = match outcome {
        LoginOutcome::Issued(account_id) => account_id,
        LoginOutcome::InvalidCredentials => return Err(ApiError::InvalidCredentials),
        LoginOutcome::AccountBlocked(account_id) => {
            tracing::info!(
                account_id = account_id.as_str(),
                game_id = game_id.as_str(),
                "guest login blocked"
            );
            return Err(ApiError::AccountBlocked);
        }
    };
    tracing::info!(
        account_id = account_id.as_str(),
        game_id = game_id.as_str(),
        "game entry issued"
    );
    Ok(Json(LoggedIn {
        account_id: account_id.as_str().to_owned(),
        ticket: encode(ticket.expose()),
        expires_in_secs: state.ttl_secs,
        entry_url: game.entry_url.clone(),
    }))
}

async fn verify_ticket(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    request: Result<Json<Map<String, Value>>, JsonRejection>,
) -> Result<Json<Verified>, ApiError> {
    let key = headers.get("x-api-key").ok_or(ApiError::Unauthorized)?;
    if !key_matches(state.key.expose().as_bytes(), key.as_bytes()) {
        return Err(ApiError::Unauthorized);
    }
    let request: VerifyRequest = parse_body(request)?;
    let ticket = Ticket::new(decode(&request.ticket).ok_or(ApiError::InvalidRequest)?);
    let (game_id, _) = state
        .games
        .get_key_value(request.game_id.as_str())
        .ok_or(ApiError::UnknownGame)?;
    let verified = login::consume_ticket(&state.pool, &ticket, game_id)
        .await
        .map_err(ApiError::Database)?
        .ok_or(ApiError::TicketInvalid)?;
    tracing::info!(
        account_id = verified.account_id.as_str(),
        game_id = game_id.as_str(),
        "game entry verified"
    );
    Ok(Json(Verified {
        account_id: verified.account_id.as_str().to_owned(),
        membership: match verified.membership {
            MembershipStatus::Member => "member",
            MembershipStatus::Canceled => "canceled",
            MembershipStatus::Sanctioned => "sanctioned",
        },
        sanction: match verified.account_status {
            AccountStatus::Active => "none",
            AccountStatus::Sanctioned => "sanctioned",
            AccountStatus::Closing => "closing",
        },
    }))
}
