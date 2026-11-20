//! `ferrent-web` binary: loads the configuration and serves the gateway.

use std::path::PathBuf;
use std::sync::Arc;

use ferrent_web::{WebConfig, app, sse};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut path = std::env::var("FERRENT_WEB_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/etc/ferrent/web.toml"));
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => {
                path = PathBuf::from(args.next().ok_or("--config needs a path")?);
            }
            "--help" | "-h" => {
                println!("usage: ferrent-web [--config /etc/ferrent/web.toml]");
                return Ok(());
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    let config = WebConfig::load(&path)?;
    // The CA is the trust anchor; the optional fingerprint makes the pin
    // explicit, exactly like the native clients.
    let pem = std::fs::read_to_string(&config.daemon.ca)?;
    if let Some(expected) = &config.daemon.fingerprint {
        let actual = ferrent_protocol::tls::fingerprint_from_pem(&pem)?;
        if !expected.eq_ignore_ascii_case(&actual) {
            return Err(format!(
                "daemon CA fingerprint mismatch: configured {expected}, certificate {actual}"
            )
            .into());
        }
    }

    let tls = match &config.tls {
        Some(settings) => Some(ferrent_web::tls::load_or_generate(settings).await?),
        None => None,
    };

    let (state, router) = app(config.clone());
    sse::spawn_watch(Arc::clone(&state));

    match tls {
        Some(tls) => {
            let address: std::net::SocketAddr = config.listen.parse()?;
            eprintln!(
                "ferrent-web https://{} (read_only={}, daemon={}, cert={}, fingerprint={})",
                config.listen,
                config.read_only,
                config.daemon.url,
                tls.cert_path.display(),
                tls.fingerprint
            );
            axum_server::bind_rustls(address, tls.config)
                .serve(router.into_make_service())
                .await?;
        }
        None => {
            let listener = tokio::net::TcpListener::bind(&config.listen).await?;
            eprintln!(
                "ferrent-web http://{} (read_only={}, daemon={})",
                config.listen, config.read_only, config.daemon.url
            );
            axum::serve(listener, router)
                .with_graceful_shutdown(shutdown())
                .await?;
        }
    }
    Ok(())
}

async fn shutdown() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}
