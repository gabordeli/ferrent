//! Wire protocol shared by `ferrentd`, `ferrent-cli`, and `ferrent-gui`.

pub mod client;

pub mod v1 {
    //! Generated protobuf and gRPC types.
    #![allow(clippy::all)]

    tonic::include_proto!("ferrent.v1");
}

pub use v1::*;

/// Version advertised during the client handshake.
pub const PROTOCOL_VERSION: u32 = 1;
