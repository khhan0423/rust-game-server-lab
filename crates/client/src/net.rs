//! Calls the publisher and saves newly issued credentials off the UI thread.

use std::path::PathBuf;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use bevy::prelude::Resource;
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::sync::mpsc::{self, Receiver, Sender};

use crate::login::{Outcome, Ticket};
use crate::profile::{self, AccountId, Credentials, Guest, GuestId, Secret};

const ACCOUNTS: &str = "/v1/guest/accounts";
const LOGIN: &str = "/v1/guest/login";

/// One request from the login panel.
#[derive(Debug)]
pub(super) enum Command {
    Create {
        publisher_url: String,
    },
    LogIn {
        credentials: Credentials,
        game_id: String,
    },
}

/// Request activity and completed operations consumed by the UI.
#[derive(Debug)]
pub(super) enum Event {
    Sent(&'static str),
    Received {
        path: &'static str,
        status: StatusCode,
    },
    Finished(Outcome),
}

/// Bounded channels between the window and the publisher worker.
#[derive(Debug, Resource)]
pub(super) struct Net {
    pub(super) commands: Sender<Command>,
    pub(super) events: Receiver<Event>,
}

/// Reasons the worker could not start.
#[derive(Debug, thiserror::Error)]
pub(super) enum StartError {
    #[error("cannot initialize publisher HTTP client")]
    Client(#[from] reqwest::Error),
    #[error("cannot initialize publisher runtime or thread")]
    Runtime(#[from] std::io::Error),
}

/// Starts the single-thread runtime and returns the window-facing channels.
pub(super) fn start(profile_path: PathBuf) -> Result<(Net, JoinHandle<()>), StartError> {
    let (commands, requests) = mpsc::channel(4);
    let (updates, events) = mpsc::channel(16);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let worker = thread::Builder::new()
        .name("publisher-http".into())
        .spawn(move || {
            runtime.block_on(
                Worker {
                    client,
                    updates,
                    profile_path,
                }
                .run(requests),
            );
        })?;
    Ok((Net { commands, events }, worker))
}

#[derive(Debug, thiserror::Error)]
enum RequestError {
    #[error("Publisher URL must be an HTTP origin")]
    InvalidPublisher,
    #[error("Publisher request timed out")]
    Timeout,
    #[error("Publisher connection failed")]
    Connection,
    #[error("Publisher response did not match the login API (HTTP {0})")]
    InvalidResponse(StatusCode),
    #[error("Publisher returned {0} (HTTP {1})")]
    Server(ServerCode, StatusCode),
    #[error("The publisher returned a ticket for a different account")]
    AccountMismatch,
    #[error("The client window closed")]
    WindowClosed,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ServerCode {
    InvalidRequest,
    InvalidCredentials,
    Unauthorized,
    AccountBlocked,
    UnknownGame,
    TicketInvalid,
    InternalError,
}

impl std::fmt::Display for ServerCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRequest => "invalid_request",
            Self::InvalidCredentials => "invalid_credentials",
            Self::Unauthorized => "unauthorized",
            Self::AccountBlocked => "account_blocked",
            Self::UnknownGame => "unknown_game",
            Self::TicketInvalid => "ticket_invalid",
            Self::InternalError => "internal_error",
        })
    }
}

#[derive(Deserialize)]
struct ErrorBody {
    error: ServerCode,
}

#[derive(Serialize)]
struct EmptyBody {}

#[derive(Serialize)]
struct LoginBody<'a> {
    guest_id: GuestId,
    secret: &'a Secret,
    game_id: &'a str,
}

#[derive(Deserialize)]
struct LoginReply {
    account_id: AccountId,
    ticket: String,
    expires_in_secs: u64,
    entry_url: String,
}

struct Worker {
    client: Client,
    updates: Sender<Event>,
    profile_path: PathBuf,
}

impl Worker {
    async fn run(self, mut requests: Receiver<Command>) {
        while let Some(command) = requests.recv().await {
            let result = match command {
                Command::Create { publisher_url } => self.create(publisher_url).await,
                Command::LogIn {
                    credentials,
                    game_id,
                } => self.log_in(&credentials, &game_id).await,
            };
            let outcome = match result {
                Ok(outcome) => outcome,
                Err(RequestError::WindowClosed) => break,
                Err(error) => {
                    tracing::warn!(%error, "publisher operation failed");
                    Outcome::Failed(error.to_string())
                }
            };
            if self.updates.send(Event::Finished(outcome)).await.is_err() {
                break;
            }
        }
    }

    async fn create(&self, publisher_url: String) -> Result<Outcome, RequestError> {
        let guest: Guest = self
            .post(&publisher_url, Route::Create, &EmptyBody {})
            .await?;
        let credentials = Credentials {
            publisher_url,
            guest,
        };
        match profile::save(&self.profile_path, &credentials).await {
            Ok(()) => Ok(Outcome::Created(credentials)),
            Err(error) => {
                tracing::warn!(%error, "could not save guest credentials");
                Ok(Outcome::Unsaved {
                    credentials,
                    error: error.to_string(),
                })
            }
        }
    }

    async fn log_in(
        &self,
        credentials: &Credentials,
        game_id: &str,
    ) -> Result<Outcome, RequestError> {
        let body = LoginBody {
            guest_id: credentials.guest.id,
            secret: &credentials.guest.secret,
            game_id,
        };
        let reply: LoginReply = self
            .post(&credentials.publisher_url, Route::LogIn, &body)
            .await?;
        if reply.account_id != credentials.guest.account_id {
            return Err(RequestError::AccountMismatch);
        }
        if reply.ticket.len() != 64 || !reply.ticket.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(RequestError::InvalidResponse(StatusCode::OK));
        }
        Ok(Outcome::LoggedIn(Ticket {
            prefix: reply.ticket.chars().take(8).collect(),
            entry_url: reply.entry_url,
            received: Instant::now(),
            ttl: Duration::from_secs(reply.expires_in_secs),
        }))
    }

    async fn post<T: DeserializeOwned>(
        &self,
        publisher_url: &str,
        route: Route,
        body: &impl Serialize,
    ) -> Result<T, RequestError> {
        let (path, success) = match route {
            Route::Create => (ACCOUNTS, StatusCode::CREATED),
            Route::LogIn => (LOGIN, StatusCode::OK),
        };
        let mut url = Url::parse(publisher_url).map_err(|_| RequestError::InvalidPublisher)?;
        if url.scheme() != "http"
            || url.host().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            return Err(RequestError::InvalidPublisher);
        }
        url.set_path(path);
        self.updates
            .send(Event::Sent(path))
            .await
            .map_err(|_| RequestError::WindowClosed)?;
        tracing::info!(path, "sending publisher request");
        let response = self
            .client
            .post(url)
            .json(body)
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    RequestError::Timeout
                } else {
                    RequestError::Connection
                }
            })?;
        let status = response.status();
        self.updates
            .send(Event::Received { path, status })
            .await
            .map_err(|_| RequestError::WindowClosed)?;
        tracing::info!(
            path,
            status = status.as_u16(),
            "received publisher response"
        );
        if status == success {
            response.json().await.map_err(|error| {
                if error.is_timeout() {
                    RequestError::Timeout
                } else {
                    RequestError::InvalidResponse(status)
                }
            })
        } else {
            let body: ErrorBody = response.json().await.map_err(|error| {
                if error.is_timeout() {
                    RequestError::Timeout
                } else {
                    RequestError::InvalidResponse(status)
                }
            })?;
            Err(RequestError::Server(body.error, status))
        }
    }
}

enum Route {
    Create,
    LogIn,
}
