# Stream-Coin

> Real-time cryptocurrency arbitrage detection system across multiple Iranian exchanges

[![Rust](https://img.shields.io/badge/rust-%23000000.svg?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Kafka](https://img.shields.io/badge/Apache%20Kafka-000?style=for-the-badge&logo=apachekafka)](https://kafka.apache.org/)
[![Redis](https://img.shields.io/badge/redis-%23DD0031.svg?style=for-the-badge&logo=redis&logoColor=white)](https://redis.io/)
[![MySQL](https://img.shields.io/badge/mysql-%2300f.svg?style=for-the-badge&logo=mysql&logoColor=white)](https://www.mysql.com/)

Stream-Coin is a high-performance, asynchronous cryptocurrency market data aggregator built with Rust. It connects to multiple Iranian cryptocurrency exchanges, streams real-time price data into Kafka, and enables instant arbitrage opportunity detection through stream processing.

---

## Overview

Stream-Coin solves the challenge of identifying price discrepancies across multiple cryptocurrency exchanges in real-time. By leveraging event streaming and async processing, the system can detect arbitrage opportunities within milliseconds.

### Key Features

- **Asynchronous Architecture**: Built on Tokio for non-blocking, concurrent operations
- **Multi-Exchange Support**: Integrates with 8+ Iranian cryptocurrency exchanges
- **Real-time Streaming**: Kafka-based event streaming for low-latency data processing
- **Persistent Storage**: MySQL database with SeaORM for reliable data persistence
- **High-Performance Caching**: Redis integration for fast data access
- **Clean Architecture**: Domain-driven design with clear separation of concerns
- **API-First Design**: RESTful API with Actix-web and comprehensive Swagger documentation

---

## Architecture

```
┌─────────────────┐
│   Exchanges     │
│  (WebSocket/    │
│   REST APIs)    │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Stream-Coin    │
│   (Rust App)    │
│                 │
│  ┌───────────┐  │
│  │  Actix    │  │ ◄── REST API
│  │   Web     │  │
│  └───────────┘  │
│                 │
│  ┌───────────┐  │
│  │  Tokio    │  │ ◄── Async Runtime
│  │  Runtime  │  │
│  └───────────┘  │
└────────┬────────┘
         │
         ├──────────────┐
         ▼              ▼
┌─────────────┐  ┌─────────────┐
│   Kafka     │  │   Redis     │
│  (Streams)  │  │  (Cache)    │
└─────────────┘  └─────────────┘
         │
         ▼
┌─────────────────┐
│     MySQL       │
│  (SeaORM)       │
└─────────────────┘
```

### Technology Stack

| Component | Technology |
|-----------|-----------|
| **Language** | Rust 🦀 |
| **Async Runtime** | Tokio |
| **Web Framework** | Actix-web |
| **Message Broker** | Apache Kafka |
| **Cache** | Redis |
| **Database** | MySQL |
| **ORM** | SeaORM |
| **Container Runtime** | Docker / Nerdctl |

---

## Quick Start

### Installation

1. **Clone the repository**
   ```bash
   git clone https://github.com/yourusername/stream-coin.git
   cd stream-coin
   ```

2. **Set up infrastructure**
   
   Using Docker:
   ```bash
   sudo docker compose up -d
   ```
   
   Using Nerdctl (containerd):
   ```bash
   sudo nerdctl compose -f ./docker-compose.yml up -d
   ```

3. **Configure environment**
   ```bash
   cp .env.example .env
   # Edit .env with your configuration
   ```

4. **Run database migrations**
   ```bash
   cargo run --bin migrate
   ```

5. **Start the application**
   ```bash
   cargo run --bin stream-coin
   ```

---

## Usage

### API Documentation

Once the application is running, access the interactive API documentation:

- **Swagger UI**: http://localhost:8080/swagger-ui/

### Management Interfaces

- **Kafka UI**: http://localhost:8083/ui/
- **Redis Insight**: http://localhost:8082/

### Debugging

Enable debug logging in your code:

```rust
use log::debug;

debug!("Payload: {}", payload);
```

Run with `RUST_LOG` environment variable:
```bash
RUST_LOG=debug cargo run --bin stream-coin
```

---

## 🔌 Supported Exchanges

Stream-Coin currently integrates with the following Iranian cryptocurrency exchanges:

| Exchange | API Documentation | Status |
|----------|------------------|--------|
| **Wallex** | [docs](https://wallex.ir/api-document) | ✅ Active |
| **Bitpin** | [docs](https://docs.bitpin.ir/v1/docs/Introduction/bitpin-api-documentation) | ✅ Active |
| **Aban Tether** | [docs](https://docs.abantether.com/#buy-orders-list) | ✅ Active |
| **Ramzinex** | [docs](https://docs.ramzinex.com/#tag/()/operation/currenciesId) | ✅ Active |
| **Exir** | [docs](https://apidocs.exir.io/#introduction) | ✅ Active |
| **Nobitex** | [docs](https://apidocs.nobitex.ir/#0f4b9b52e8) | ✅ Active |
| **Tetherland** | [docs](https://docs.tetherland.com/docs/tetherland/15fd2beb3a6d3-get-user-info) | ✅ Active |
| **Tabdeal** | [docs](https://docs.tabdeal.org/#00d56275ee) | ✅ Active |
| **Bit24** | [docs](https://docs.bit24.cash/#api-24) | ✅ Active |

---

## 🏗️ Project Structure

```
stream-coin/
├── src/
│   ├── api/              # API layer (Actix-web routes & handlers)
│   ├── application/      # Application services & use cases
│   ├── domain/           # Domain models & business logic
│   ├── infrastructure/   # External integrations (Kafka, Redis, DB)
│   ├── exchanges/        # Exchange-specific implementations
│   └── main.rs           # Application entry point
├── migrations/           # Database migrations
├── docker-compose.yml    # Infrastructure setup
├── Cargo.toml           # Rust dependencies
└── README.md
```

### Clean Architecture Layers

1. **Domain Layer**: Core business logic and entities
2. **Application Layer**: Use cases and application-specific business rules
3. **Infrastructure Layer**: External concerns (database, message brokers, APIs)
4. **API Layer**: HTTP interface and presentation logic

---

## 🧪 Development

### Running Tests

```bash
# Run all tests
cargo test

# Run specific test suite
cargo test --test integration_tests

# Run with output
cargo test -- --nocapture
```

### Code Quality

```bash
# Format code
cargo fmt

# Run linter
cargo clippy

# Check for issues
cargo check
```

### Building for Production

```bash
# Optimized release build
cargo build --release

# Run release binary
./target/release/stream-coin
```

---

## Roadmap

### Current Tasks

- [ ] Update readme
- [ ] Create output variable standardization
- [ ] Fix Redis Insight configuration
- [ ] Add Postman API collection
- [ ] Complete Swagger documentation for all endpoints

### Future Enhancements

- [ ] WebSocket support for real-time client updates
- [ ] Machine learning-based opportunity prediction
- [ ] Multi-region deployment support

---

## Contributing

Contributions are welcome! Please follow these guidelines:

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

### Code Style

- Follow Rust naming conventions
- Write comprehensive tests
- Document public APIs
- Keep functions focused and small
- Use meaningful variable names

---

## License

This project is licensed under the GPL-3.0 License - see the [LICENSE](LICENSE) file for details.

