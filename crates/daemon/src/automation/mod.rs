//! Ratio and seed-time policy (M8-T3).
//!
//! The decision is a pure function of the policy and the torrent's counters,
//! so the rules are unit-tested without an engine. The daemon side only
//! supplies the config, the mirror and the actions.

pub mod ratio;
