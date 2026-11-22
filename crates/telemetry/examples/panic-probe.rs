//! Symbolization probe for the Bugsink integration.
//!
//! Run it with the same profile as the shipped binary:
//!
//! ```text
//! FERRENT_ENV=probe cargo run --release -p ferrent-telemetry --example panic-probe
//! ```
//!
//! The panic lands in the Bugsink `ferrent` project (id 5) tagged
//! `service=panic-probe`; the frames show whether the binary's debug info
//! survived the packaging and whether the SDK attached it.

fn main() {
    let _guard = ferrent_telemetry::init("panic-probe");
    // The env lookup keeps clippy from folding the Option into a literal.
    let missing: Option<u8> = std::env::var("FERRENT_PROBE_VALUE")
        .ok()
        .and_then(|value| value.parse().ok());
    println!("about to panic on a missing value");
    missing.expect("symbolization probe");
}
