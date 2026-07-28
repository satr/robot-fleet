FROM rust:1.80-bookworm AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
COPY backend ./backend
COPY robot-simulator ./robot-simulator
RUN cargo build --release -p robot-simulator

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /app/target/release/robot-simulator /usr/local/bin/robot-simulator
RUN mkdir -p /state
EXPOSE 9100
CMD ["robot-simulator"]
