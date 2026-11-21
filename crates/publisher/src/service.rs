//! Binds the publisher public and operator routers to separate ports.
use crate::{LOGIN_DB_URL_ENV, api, config::Config, ops_api};
use anyhow::Context;
use db::login::LOGIN_MIGRATOR;
use infra::{
    config::Secret,
    http::{
        Exposure, HttpListener,
        api::{ApiKey, operator},
    },
    server::Server,
};
use std::sync::Arc;
const GAME_API_KEY_ENV: &str = "PUBLISHER_GAME_API_KEY";

struct Startup {
    pool: db::Pool,
    public: HttpListener,
    operator: HttpListener,
    game_key: ApiKey,
    ops_key: ApiKey,
}

async fn startup_barrier(config: &Config) -> anyhow::Result<Startup> {
    let key = ApiKey::from_env("x-api-key", GAME_API_KEY_ENV)?;
    let ops_key = ApiKey::from_env(operator::KEY_HEADER, operator::KEY_ENV)?;
    let url = Secret::from_env(LOGIN_DB_URL_ENV)?;
    let pool = db::open(url.expose(), &config.db, &LOGIN_MIGRATOR).await?;
    let listener = HttpListener::bind("http", &config.http, Exposure::Public)
        .await
        .context("binding publisher API listener")?;
    let operator = HttpListener::bind("ops_api", &config.ops_api, Exposure::Loopback)
        .await
        .context("binding publisher operator API listener")?;
    Ok(Startup {
        pool,
        public: listener,
        operator,
        game_key: key,
        ops_key,
    })
}

pub(super) async fn start(server: &mut Server, config: &Config) -> anyhow::Result<db::Pool> {
    let Startup {
        pool,
        public,
        operator,
        game_key,
        ops_key,
    } = startup_barrier(config).await?;
    let router = api::router(Arc::new(api::AppState {
        pool: pool.clone(),
        games: config
            .games
            .iter()
            .map(|(id, game)| (db::login::GameId::new(id.clone()), game.clone()))
            .collect(),
        ttl_secs: config.tickets.ttl_secs,
        key: game_key,
    }));
    let ops_router = ops_api::router(pool.clone(), ops_key);
    server.spawn_http(public, router);
    server.spawn_http(operator, ops_router);
    Ok(pool)
}
