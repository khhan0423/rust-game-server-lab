//! HTML rendering and the static assets used by page templates.

use askama::Template;
use axum::{
    Router,
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};

pub(super) fn assets<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route(
            "/static/htmx.min.js",
            get(|| {
                std::future::ready((
                    [(header::CONTENT_TYPE, "text/javascript")],
                    include_bytes!("../assets/htmx.min.js").as_slice(),
                ))
            }),
        )
        .route(
            "/static/ops.css",
            get(|| {
                std::future::ready((
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    include_str!("../assets/ops.css"),
                ))
            }),
        )
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
