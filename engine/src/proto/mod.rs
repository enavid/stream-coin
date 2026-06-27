//! Protobuf wire types for the `candles` and `signals` Kafka topics (ROADMAP
//! Loop 4c).
//!
//! The schema lives in `proto/*.proto` at the repo root and is the single
//! source of truth shared with the Python strategy SDK. The Rust types in
//! [`v1`] are generated from it at build time by `build.rs` (prost + protox,
//! no system `protoc`). Conversions from the JSON wire payloads
//! ([`CandlePayload`](crate::candle::entity::CandlePayload),
//! [`SignalPayload`](crate::wire_message::SignalPayload)) and `encode_*`
//! helpers live in [`conversions`].

/// Generated prost types for protobuf package `stream_coin.v1`.
pub mod v1 {
    include!(concat!(env!("OUT_DIR"), "/stream_coin.v1.rs"));
}

mod conversions;

pub use conversions::{encode_candle, encode_signal};
