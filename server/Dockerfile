# Build stage
FROM rust:1.94-slim AS builder

WORKDIR /app

# Install dependencies for building
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

# Copy manifests first for dependency caching
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release 2>/dev/null || true
# Drop the dummy-main artifacts so cargo re-links against the real sources
RUN rm -rf src target/release/tomato-server target/release/deps/tomato_server*

# Copy source and build
COPY src ./src
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

# tzdata: needed so the TZ=Asia/Shanghai env var actually resolves to a
# real zoneinfo file. Without it chrono::Local falls back to UTC silently.
RUN apt-get update && apt-get install -y ca-certificates tzdata && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/tomato-server /usr/local/bin/tomato-server
# Migrations are loaded at runtime from ./src/db/migrations (see db/pool.rs)
COPY --from=builder /app/src/db/migrations /app/src/db/migrations
WORKDIR /app

EXPOSE 8099

ENV RUST_LOG=tomato_server=info,actix_web=info

CMD ["tomato-server"]
