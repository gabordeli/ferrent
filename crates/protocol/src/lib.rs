//! Wire protocol shared by `ferrentd`, `ferrent-cli`, and `ferrent-gui`.
//!
//! The concrete gRPC schema lands in M1-T3; this crate is the single source of
//! truth for request and response types.

/// Version advertised during the client handshake.
pub const PROTOCOL_VERSION: u32 = 1;
