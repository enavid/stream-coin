FROM rust:1.93.0-slim AS builder
WORKDIR /app

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
RUN mkdir src bin && echo "fn main() {}" > bin/http.rs && echo "" > src/lib.rs
RUN cargo build --release --locked
RUN rm -rf src bin

COPY . .
RUN touch src/lib.rs bin/http.rs && cargo build --release --locked

FROM debian:bookworm-slim
WORKDIR /app

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

ENV RUST_LOG=info

COPY --from=builder /app/target/release/stream-coin .

EXPOSE 8080
CMD ["./stream-coin"]
