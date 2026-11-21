//! HTTP boundary for guest accounts, login, and single-use ticket verification.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use db::login::{
    self, AccountId, EntryGrant, GameId, GuestCredentials, GuestId, GuestSecret, LoginOutcome,
    Ticket,
};
use infra::http::api::{ApiError, ApiJson, ApiKey};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::GameConfig;
use crate::credentials::{decode, encode, random_bytes};

pub(super) struct AppState {
    pub(super) pool: db::Pool,
    pub(super) games: HashMap<GameId, GameConfig>,
    pub(super) ttl_secs: u32,
    pub(super) key: ApiKey,
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
    guest_id: String,
    secret: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoginRequest {
    guest_id: String,
    secret: String,
    game_id: String,
}

#[derive(Serialize)]
struct LoggedIn {
    ticket: String,
    expires_in_secs: u32,
    entry_url: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
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

async fn create_guest(
    State(state): State<Arc<AppState>>,
    request: ApiJson<CreateRequest>,
) -> Result<(StatusCode, Json<Created>), ApiError> {
    let CreateRequest {} = request.into_inner();
    let account_id = AccountId::new(Uuid::now_v7());
    let guest_id = GuestId::new(Uuid::now_v7());
    let secret = GuestSecret::new(random_bytes().map_err(ApiError::internal)?);
    login::create_guest(
        &state.pool,
        &account_id,
        &GuestCredentials {
            guest_id: &guest_id,
            secret: &secret,
        },
    )
    .await
    .map_err(ApiError::internal)?;
    tracing::info!(account_id = %account_id, "guest account created");
    Ok((
        StatusCode::CREATED,
        Json(Created {
            guest_id: guest_id.to_string(),
            secret: encode(secret.expose()),
        }),
    ))
}

async fn login_guest(
    State(state): State<Arc<AppState>>,
    request: ApiJson<LoginRequest>,
) -> Result<Json<LoggedIn>, ApiError> {
    let request: LoginRequest = request.into_inner();
    let guest_id = GuestId::parse(&request.guest_id).map_err(|_| ApiError::INVALID)?;
    let secret = GuestSecret::new(decode(&request.secret).ok_or(ApiError::INVALID)?);
    let (game_id, game) = state
        .games
        .get_key_value(request.game_id.as_str())
        .ok_or(ApiError::new(StatusCode::NOT_FOUND, "unknown_game"))?;
    let ticket = Ticket::new(random_bytes().map_err(ApiError::internal)?);
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
    .map_err(ApiError::internal)?;
    let account_id = match outcome {
        LoginOutcome::Issued(account_id) => account_id,
        LoginOutcome::InvalidCredentials => {
            return Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "invalid_credentials",
            ));
        }
        LoginOutcome::AccountBlocked(account_id) => {
            tracing::info!(
                account_id = %account_id,
                game_id = game_id.as_str(),
                "guest login blocked"
            );
            return Err(ApiError::new(StatusCode::FORBIDDEN, "account_blocked"));
        }
    };
    tracing::info!(
        account_id = %account_id,
        game_id = game_id.as_str(),
        "game entry issued"
    );
    Ok(Json(LoggedIn {
        ticket: encode(ticket.expose()),
        expires_in_secs: state.ttl_secs,
        entry_url: game.entry_url.clone(),
    }))
}

async fn verify_ticket(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    request: Result<ApiJson<VerifyRequest>, ApiError>,
) -> Result<Json<Verified>, ApiError> {
    state.key.verify(&headers)?;
    let request = request?.into_inner();
    let ticket = Ticket::new(decode(&request.ticket).ok_or(ApiError::INVALID)?);
    let (game_id, _) = state
        .games
        .get_key_value(request.game_id.as_str())
        .ok_or(ApiError::new(StatusCode::NOT_FOUND, "unknown_game"))?;
    let verified = login::consume_ticket(&state.pool, &ticket, game_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or(ApiError::new(StatusCode::NOT_FOUND, "ticket_invalid"))?;
    tracing::info!(
        account_id = %verified.account_id,
        game_id = game_id.as_str(),
        "game entry verified"
    );
    Ok(Json(Verified {
        account_id: verified.account_id.to_string(),
        membership: crate::wire::membership(verified.membership),
        sanction: crate::wire::ticket_sanction(verified.account_status),
    }))
}
