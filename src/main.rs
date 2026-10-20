#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, unused_must_use)]
use factorio_control::{
    App,
    api::{self, Api},
    process,
};
use std::{net::SocketAddr, path::PathBuf};
use utoipa::OpenApi;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|arg| arg == "--openapi") {
        println!("{}", Api::openapi().to_pretty_json()?);
        return Ok(());
    }
    let bind: SocketAddr = std::env::var("FC_BIND")
        .unwrap_or_else(|_| "127.0.0.1:9400".into())
        .parse()?;
    if !bind.ip().is_loopback() {
        return Err(
            "FC_BIND must use a loopback address; authentication is not implemented".into(),
        );
    }
    let directory = PathBuf::from(std::env::var("FC_DATA_DIR").unwrap_or_else(|_| "./data".into()));
    let binary = std::env::var_os("FC_FACTORIO_BIN").map(PathBuf::from);
    let app = App::new(directory, binary).map_err(|e| e.message)?;
    let monitor = tokio::spawn(process::monitor(app.clone()));
    let listener = tokio::net::TcpListener::bind(bind).await?;
    eprintln!("Factorio Control listening on http://{bind}");
    let shutdown_app = app.clone();
    let result = axum::serve(listener, api::router(app.clone(), std::path::Path::new("web/dist")))
        .with_graceful_shutdown(async move {
            let mut terminate = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(signal) => signal,
                Err(error) => { eprintln!("Unable to subscribe to SIGTERM: {error}"); return; }
            };
            tokio::select! {
                result = tokio::signal::ctrl_c() => { if let Err(error) = result { eprintln!("Signal listener failed: {error}"); } }
                _ = terminate.recv() => {}
            }
            process::shutdown(&shutdown_app).await;
        }).await;
    monitor.abort();
    process::shutdown(&app).await;
    result?;
    Ok(())
}
