//! `nexus-server` process. Configuration is environment variables only.

use std::sync::Arc;

use nexus_server::github::LiveGithub;
use nexus_server::{router, AppState, Config};

#[tokio::main]
async fn main() {
    let config = match Config::from_env() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("configuration error: {err}");
            std::process::exit(2);
        }
    };
    let path = config.database_path.clone();
    let bind = config.bind.clone();
    let db = match nexus_server::db::Database::open(&path) {
        Ok(db) => db,
        Err(err) => {
            eprintln!("database error: {err}");
            std::process::exit(2);
        }
    };
    let github = match LiveGithub::new() {
        Ok(github) => github,
        Err(err) => {
            eprintln!("http client error: {err}");
            std::process::exit(2);
        }
    };
    let state = AppState {
        db: Arc::new(db),
        config: Arc::new(config),
        github: Arc::new(github),
    };
    let listener = match tokio::net::TcpListener::bind(&bind).await {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("bind {bind} failed: {err}");
            std::process::exit(2);
        }
    };
    eprintln!("nexus-server listening on {bind}");
    if let Err(err) = axum::serve(listener, router(state)).await {
        eprintln!("server stopped: {err}");
        std::process::exit(1);
    }
}
