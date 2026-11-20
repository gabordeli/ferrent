//! Small RPC latency benchmark for the M2-T4 R2 measurement.
//!
//! Run against a daemon with 4 000 torrents:
//! `cargo run -p ferrent-daemon --example rpc_bench -- \
//!      --server https://localhost:50130 --ca .../tls-cert.pem --token secret`

use std::env;
use std::path::PathBuf;
use std::time::Instant;

use ferrent_protocol::client::connect;
use ferrent_protocol::v1::{ListTorrentsRequest, SortField};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut server = "https://localhost:50130".to_owned();
    let mut ca = PathBuf::from("/tmp/ferrent-mvp-data/tls-cert.pem");
    let mut token = "secret".to_owned();
    let mut queries = 200usize;
    let mut filter = "payload-".to_owned();
    let mut limit = 50u32;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--server" => server = args.next().expect("--server value"),
            "--ca" => ca = PathBuf::from(args.next().expect("--ca value")),
            "--token" => token = args.next().expect("--token value"),
            "--queries" => queries = args.next().expect("--queries value").parse()?,
            "--filter" => filter = args.next().expect("--filter value"),
            "--limit" => limit = args.next().expect("--limit value").parse()?,
            "--help" | "-h" => {
                println!(
                    "usage: rpc_bench [--server URL] [--ca PEM] [--token T] [--queries N] [--filter S] [--limit N]"
                );
                return Ok(());
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    let pem = std::fs::read_to_string(&ca)?;
    let client = connect(&server, &pem, &token).await?;
    let mut durations = Vec::with_capacity(queries);
    let mut total = 0u32;

    for _ in 0..10 {
        let _ = query(&client, &filter, limit).await?;
    }
    for _ in 0..queries {
        let start = Instant::now();
        let response = query(&client, &filter, limit).await?;
        durations.push(start.elapsed().as_secs_f64() * 1000.0);
        total = response.1;
    }
    durations.sort_by(f64::total_cmp);
    let percentile = |p: f64| {
        let index = ((durations.len() as f64 * p).ceil() as usize)
            .saturating_sub(1)
            .min(durations.len() - 1);
        durations[index]
    };
    println!("queries={queries} total={total} limit={limit} filter={filter:?}");
    println!("rpc_ms_p50={:.2}", percentile(0.50));
    println!("rpc_ms_p95={:.2}", percentile(0.95));
    println!("rpc_ms_p99={:.2}", percentile(0.99));
    println!("rpc_ms_max={:.2}", durations[durations.len() - 1]);
    Ok(())
}

async fn query(
    client: &ferrent_protocol::client::AuthClient,
    filter: &str,
    limit: u32,
) -> Result<(usize, u32), tonic::Status> {
    let mut client = client.clone();
    let response = client
        .list_torrents(ListTorrentsRequest {
            filter: filter.to_owned(),
            sort: SortField::Name as i32,
            descending: false,
            offset: 0,
            limit,
            state_filter: 0,
            label_filter: String::new(),
        })
        .await?;
    let response = response.into_inner();
    Ok((response.torrents.len(), response.total))
}
