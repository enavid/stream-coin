FROM rust:1.93.0-slim AS builder
WORKDIR /app

RUN apt-get update && apt-get install -y pkg-config libssl-dev libcurl4-openssl-dev curl cmake build-essential && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY engine/Cargo.toml engine/Cargo.toml
COPY cli/Cargo.toml cli/Cargo.toml
RUN mkdir -p engine/bin engine/src cli/src \
    && echo "fn main() {}" > engine/bin/http.rs && echo "" > engine/src/lib.rs \
    && echo "fn main() {}" > cli/src/main.rs
RUN cargo build --release --locked -p stream-coin
RUN rm -rf engine/src engine/bin cli/src

COPY . .
RUN touch engine/src/lib.rs engine/bin/http.rs && cargo build --release --locked -p stream-coin

FROM debian:bookworm-slim
WORKDIR /app

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

ENV RUST_LOG=info

COPY --from=builder /app/target/release/stream-coin .

EXPOSE 8080
CMD ["./stream-coin"]
