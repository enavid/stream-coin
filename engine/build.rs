//! Generates Rust types for the Kafka `candles`/`signals` protobuf contracts
//! from the `.proto` schemas at the repo root (ROADMAP Loop 4c).
//!
//! Uses `protox` (a pure-Rust protobuf compiler) to parse the schemas into a
//! `FileDescriptorSet`, then `prost-build` to emit the types — so no system
//! `protoc` install is required (keeps the clean-env Docker build working).

use std::path::Path;

fn main() {
    // Schemas live at the repo root so the future Python SDK shares them;
    // the engine crate root is one level down, hence `../proto`.
    let proto_dir = Path::new("../proto");
    let protos = [
        proto_dir.join("candle.proto"),
        proto_dir.join("signal.proto"),
    ];

    for proto in &protos {
        println!("cargo:rerun-if-changed={}", proto.display());
    }

    let descriptors = protox::compile(protos, [proto_dir])
        .expect("failed to compile protobuf schemas with protox");

    prost_build::Config::new()
        .compile_fds(descriptors)
        .expect("failed to generate Rust types from protobuf descriptors");
}
