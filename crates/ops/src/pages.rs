//! Publisher pages and shared HTML presentation.

use std::sync::Arc;

use askama::Template;
use axum::Router;
use axum::extract::{Form, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::get;
use reqwest::Method;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::publisher::{AccountStatus, Accounts, Audit, Detail, SanctionStatus};

const HTMX: &[u8] = include_bytes!("../assets/htmx.min.js");
const CSS: &str = include_str!("../assets/ops.css");

pub(super) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/accounts", get(accounts))
        .route("/accounts/{id}", get(detail).post(change))
        .route("/audit", get(audit))
        .route(
            "/static/htmx.min.js",
            get(|| async { ([(header::CONTENT_TYPE, "text/javascript")], HTMX) }),
        )
        .route(
            "/static/ops.css",
            get(|| async { ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], CSS) }),
        )
}

#[derive(Template)]
#[template(path = "accounts.html")]
struct AccountsPage<'a> {
    operator: &'a str,
    rows: Accounts,
    search: &'a str,
    paging: Paging,
}

#[derive(Template)]
#[template(path = "detail.html")]
struct DetailPage<'a> {
    operator: &'a str,
    detail: Detail,
    token: &'a str,
    action: Option<(&'static str, &'static str)>,
}

#[derive(Template)]
#[template(path = "audit.html")]
struct AuditPage<'a> {
    operator: &'a str,
    rows: Audit,
    paging: Paging,
}

#[derive(Template)]
#[template(path = "error.html")]
struct ErrorPage<'a> {
    operator: &'a str,
    message: &'static str,
}

#[derive(Deserialize)]
struct Listing {
    page: Option<u32>,
    search: Option<String>,
}

struct Paging {
    number: u32,
    previous: Option<u32>,
    next: Option<u32>,
}

impl Paging {
    fn new(number: u32, has_next: bool) -> Self {
        Self {
            number,
            previous: if number > 1 { Some(number - 1) } else { None },
            next: if has_next {
                number.checked_add(1)
            } else {
                None
            },
        }
    }
}

#[derive(Deserialize)]
struct StatusForm {
    csrf: String,
    status: SanctionStatus,
    reason: String,
}

#[derive(Serialize)]
struct Change<'a> {
    status: SanctionStatus,
    reason: &'a str,
}

async fn accounts(State(state): State<Arc<AppState>>, Query(query): Query<Listing>) -> Response {
    let operator = state.publisher.operator();
    let page = query.page.unwrap_or(1);
    if page == 0 {
        return error(
            operator,
            StatusCode::BAD_REQUEST,
            "Page numbers start at one.",
        );
    }
    // IDs are UUIDs, so the canonical value can be carried in a paging link.
    let search = query.search.as_deref().unwrap_or_default().trim();
    let search = if search.is_empty() {
        String::new()
    } else if let Ok(id) = uuid::Uuid::parse_str(search) {
        id.to_string()
    } else {
        return error(
            operator,
            StatusCode::BAD_REQUEST,
            "Enter an exact account ID or guest ID.",
        );
    };
    let mut request = state
        .publisher
        .request(Method::GET, "/v1/accounts")
        .query(&[("page", page)]);
    if !search.is_empty() {
        request = request.query(&[("search", &search)]);
    }
    match request
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
    {
        Ok(response) => match response.json::<Accounts>().await {
            Ok(rows) => render(&AccountsPage {
                operator,
                paging: Paging::new(page, rows.has_next),
                rows,
                search: &search,
            }),
            Err(error) => upstream(operator, &error),
        },
        Err(error) => upstream(operator, &error),
    }
}

async fn detail(State(state): State<Arc<AppState>>, Path(id): Path<uuid::Uuid>) -> Response {
    let operator = state.publisher.operator();
    let path = format!("/v1/accounts/{id}");
    let response = match state
        .publisher
        .request(Method::GET, &path)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
    {
        Ok(response) => response,
        Err(error) => return upstream(operator, &error),
    };
    match response.json::<Detail>().await {
        Ok(detail) => {
            let action = match detail.account.status {
                AccountStatus::Active => Some(("sanctioned", "Sanction account")),
                AccountStatus::Sanctioned => Some(("active", "Lift sanction")),
                AccountStatus::Closing => None,
            };
            render(&DetailPage {
                operator,
                detail,
                token: state.security.token(),
                action,
            })
        }
        Err(error) => upstream(operator, &error),
    }
}

async fn change(
    State(state): State<Arc<AppState>>,
    Path(id): Path<uuid::Uuid>,
    headers: HeaderMap,
    Form(form): Form<StatusForm>,
) -> Response {
    let operator = state.publisher.operator();
    if !state.security.accepts(&headers, &form.csrf) {
        return error(
            operator,
            StatusCode::FORBIDDEN,
            "This request did not pass the form security check.",
        );
    }
    let Ok(reason) = infra::http::api::operator::AuditReason::try_from(form.reason.as_str()) else {
        return error(
            operator,
            StatusCode::BAD_REQUEST,
            "Enter a reason up to 1,000 characters.",
        );
    };
    let path = format!("/v1/accounts/{id}/status");
    match state
        .publisher
        .request(Method::POST, &path)
        .json(&Change {
            status: form.status,
            reason: reason.as_str(),
        })
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
    {
        Ok(_) => Redirect::to(&format!("/accounts/{id}")).into_response(),
        Err(error) => upstream(operator, &error),
    }
}

async fn audit(State(state): State<Arc<AppState>>, Query(query): Query<Listing>) -> Response {
    let operator = state.publisher.operator();
    let page = query.page.unwrap_or(1);
    if page == 0 {
        return error(
            operator,
            StatusCode::BAD_REQUEST,
            "Page numbers start at one.",
        );
    }
    let response = match state
        .publisher
        .request(Method::GET, "/v1/audit")
        .query(&[("page", page)])
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
    {
        Ok(response) => response,
        Err(error) => return upstream(operator, &error),
    };
    match response.json::<Audit>().await {
        Ok(rows) => render(&AuditPage {
            operator,
            paging: Paging::new(page, rows.has_next),
            rows,
        }),
        Err(error) => upstream(operator, &error),
    }
}

fn upstream(operator: &str, failure: &reqwest::Error) -> Response {
    let (status, message) = match failure.status() {
        Some(StatusCode::NOT_FOUND) => (
            StatusCode::NOT_FOUND,
            "The publisher could not find this account.",
        ),
        Some(StatusCode::CONFLICT) => (
            StatusCode::CONFLICT,
            "This account cannot be changed through the sanction form.",
        ),
        Some(StatusCode::BAD_REQUEST) => (
            StatusCode::BAD_REQUEST,
            "The publisher rejected the request.",
        ),
        _ => (
            StatusCode::BAD_GATEWAY,
            "The publisher operator API is unavailable or returned an unexpected response.",
        ),
    };
    error(operator, status, message)
}

fn error(operator: &str, status: StatusCode, message: &'static str) -> Response {
    (status, render(&ErrorPage { operator, message })).into_response()
}

pub(super) fn render(template: &impl Template) -> Response {
    match template.render() {
        Ok(html) => Html(html).into_response(),
        Err(error) => {
            tracing::error!(%error, "operations page rendering failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "Page rendering failed").into_response()
        }
    }
}
