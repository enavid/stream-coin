# ---- Step 1: Build Rust Application ----
FROM rust:latest AS builder
WORKDIR /app

# Cache dependencies for faster rebuilds
COPY Cargo.toml Cargo.lock ./
RUN cargo fetch

# Copy the entire source code
COPY . .

# Build the project in release mode
RUN cargo build --release

# ---- Step 2: Create a Minimal Runtime Image ----
FROM debian:bullseye-slim
WORKDIR /app

# Set environment variables
ENV RUST_LOG=info
ENV APP_ENV=production

# Copy the compiled binary from the builder stage
COPY --from=builder /app/target/release/stream-coin .

# Expose the application port
EXPOSE 8080

# Run the application
CMD ["./stream-coin"]
