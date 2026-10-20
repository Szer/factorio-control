#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, unused_must_use)]
use factorio_control::{
    App,
    api::{self, Api},
    auth, process,
};
use std::{io::Read, net::SocketAddr, path::PathBuf};
use utoipa::OpenApi;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|arg| arg == "--openapi") {
        let mut document = Api::openapi();
        document.merge(auth::Api::openapi());
        println!("{}", document.to_pretty_json()?);
        return Ok(());
    }
    let bind: SocketAddr = std::env::var("FC_BIND")
        .unwrap_or_else(|_| "127.0.0.1:9400".into())
        .parse()?;
    if std::env::args().any(|arg| arg == "--healthcheck") {
        let address = SocketAddr::new(
            if bind.ip().is_unspecified() {
                if bind.is_ipv4() {
                    std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
                } else {
                    std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)
                }
            } else {
                bind.ip()
            },
            bind.port(),
        );
        let host = match std::env::var("FC_PUBLIC_URL") {
            Ok(value) => {
                let origin = url::Url::parse(&value)?;
                origin[url::Position::BeforeHost..url::Position::AfterPort].to_owned()
            }
            Err(_) => address.to_string(),
        };
        let _: auth::SessionStatus = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .no_proxy()
            .build()?
            .get(format!("http://{address}/api/auth/status"))
            .header(reqwest::header::HOST, host)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        return Ok(());
    }
    let directory = PathBuf::from(std::env::var("FC_DATA_DIR").unwrap_or_else(|_| "./data".into()));
    let binary = std::env::var_os("FC_FACTORIO_BIN").map(PathBuf::from);
    let app = App::new(directory, binary).map_err(|e| e.message)?;
    let arguments = std::env::args().collect::<Vec<_>>();
    if let Some(position) = arguments
        .iter()
        .position(|argument| argument == "--create-admin")
    {
        let username = arguments
            .get(position + 1)
            .ok_or("--create-admin requires an account name")?;
        let mut password = String::new();
        std::io::stdin().take(1026).read_to_string(&mut password)?;
        let password = password.trim_end_matches(['\r', '\n']);
        auth::bootstrap_admin(&app.directory, username, password).map_err(|error| error.message)?;
        eprintln!("Administrator created. Restart the manager to sign in.");
        return Ok(());
    }
    let public_url = std::env::var("FC_PUBLIC_URL").ok();
    let trusted_proxies = std::env::var_os("FC_TRUSTED_PROXIES")
        .map(|value| {
            value
                .into_string()
                .map_err(|_| "FC_TRUSTED_PROXIES must contain UTF-8 IP addresses")
        })
        .transpose()?;
    let authentication = auth::Auth::load_with_trusted_proxies(
        &app.directory,
        public_url.as_deref(),
        trusted_proxies
            .as_deref()
            .filter(|value| !value.trim().is_empty()),
    )
    .map_err(|error| error.message)?;
    authentication
        .validate_bind(bind)
        .await
        .map_err(|error| error.message)?;
    if let Some(binary) = &app.binary {
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tokio::process::Command::new(binary)
                .arg("--version")
                .kill_on_drop(true)
                .output(),
        )
        .await;
        match result {
            Ok(Ok(output)) if output.status.success() => {
                let output = String::from_utf8_lossy(&output.stdout);
                let version = output
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("Version: ")
                            .and_then(|value| value.split_whitespace().next())
                    })
                    .filter(|value| {
                        value.split('.').count() == 3
                            && value.chars().all(|c| c.is_ascii_digit() || c == '.')
                    });
                app.inner.lock().await.status.version = version.map(str::to_owned);
            }
            _ => eprintln!(
                "Unable to detect Factorio version; mod changes require a working configured binary"
            ),
        }
    }
    let monitor = tokio::spawn(process::monitor(app.clone()));
    let listener = tokio::net::TcpListener::bind(bind).await?;
    eprintln!("Factorio Control listening on http://{bind}");
    let shutdown_app = app.clone();
    let result = axum::serve(listener, api::router_with_auth(app.clone(), std::path::Path::new("web/dist"), authentication).into_make_service_with_connect_info::<SocketAddr>())
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
