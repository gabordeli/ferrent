//! Measures the RPC list path against a running daemon.
//!
//! Usage: perf_probe <uri> <ca.pem> <token> [calls]
//!
//! Prints one JSON object: torrent count plus p50/p95/max of the list call in
//! milliseconds. `ci/perf.sh` compares it with the stored baseline.

use std::time::Instant;

use ferrent_protocol::v1::{ListTorrentsRequest, SortField};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut args = std::env::args().skip(1);
    let uri = args.next().expect("uri");
    let ca_path = args.next().expect("ca pem");
    let token = args.next().expect("token");
    let calls: usize = args
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(30);

    let ca = std::fs::read_to_string(ca_path)?;
    let mut client = ferrent_protocol::client::connect(&uri, &ca, &token).await?;

    // Warm the connection and the state mirror before timing.
    let first = client.list_torrents(request()).await?.into_inner();
    let total = first.total;

    let mut samples = Vec::with_capacity(calls);
    for _ in 0..calls {
        let started = Instant::now();
        let response = client.list_torrents(request()).await?.into_inner();
        assert_eq!(response.total, total, "the torrent count changed mid-run");
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
    let percentile = |value: f64| {
        let index = ((samples.len() as f64 - 1.0) * value).round() as usize;
        samples[index]
    };
    println!(
        "{{\"torrents\":{},\"calls\":{},\"list_p50_ms\":{:.1},\"list_p95_ms\":{:.1},\"list_max_ms\":{:.1}}}",
        total,
        calls,
        percentile(0.50),
        percentile(0.95),
        samples.last().copied().unwrap_or_default()
    );
    Ok(())
}

fn request() -> ListTorrentsRequest {
    ListTorrentsRequest {
        filter: String::new(),
        sort: SortField::Id as i32,
        descending: false,
        offset: 0,
        limit: 100,
        state_filter: 0,
        label_filter: String::new(),
    }
}
