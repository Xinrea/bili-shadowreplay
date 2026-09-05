# Build the architecture-independent frontend once on the native builder.
FROM --platform=$BUILDPLATFORM node:22-bookworm AS frontend-builder

WORKDIR /app

# Install system dependencies
RUN apt-get update && apt-get install -y \
    python3 \
    make \
    g++ \
    && rm -rf /var/lib/apt/lists/*

# Copy package files
COPY package.json yarn.lock ./

# Allow enough time for large packages when the registry is slow.
RUN yarn install --frozen-lockfile --network-timeout 600000

# Copy source files
COPY . .

# Build frontend
RUN yarn build

# Build Rust backend. cargo-chef keeps third-party dependencies in a separate
# Docker layer, so application source changes do not force a full rebuild.
FROM rust:1.90-slim AS rust-base

WORKDIR /app/src-tauri

# Install required system dependencies
RUN apt-get update && apt-get install -y \
    cmake \
    pkg-config \
    libssl-dev \
    glib-2.0-dev \
    libclang-dev \
    g++ \
    wget \
    xz-utils \
    git \
    && rm -rf /var/lib/apt/lists/*

RUN cargo install cargo-chef --version 0.1.78 --locked

FROM rust-base AS rust-planner

COPY src-tauri .
RUN cargo chef prepare --recipe-path recipe.json

FROM rust-base AS rust-builder

COPY --from=rust-planner /app/src-tauri/recipe.json recipe.json
RUN cargo chef cook \
    --no-default-features \
    --features headless \
    --release \
    --recipe-path recipe.json

COPY src-tauri .

# Sentry DSN baked into the binary at build time (option_env! in main.rs).
# Empty by default so Sentry stays disabled unless a DSN is provided.
ARG SENTRY_ENDPOINT=""
ENV SENTRY_ENDPOINT=${SENTRY_ENDPOINT}

# Build Rust backend
RUN cargo build --no-default-features --features headless --release

# Final stage
FROM debian:trixie-slim AS final

WORKDIR /app

# Install runtime dependencies, SSL certificates and Chinese fonts
RUN apt-get update && apt-get install -y \
    libssl3 \
    ca-certificates \
    fonts-wqy-microhei \
    netbase \
    nscd \
    ffmpeg \
    && update-ca-certificates \
    && rm -rf /var/lib/apt/lists/*


RUN touch /etc/netgroup
RUN mkdir -p /var/run/nscd && chmod 755 /var/run/nscd

# Add /app to PATH
ENV PATH="/app:${PATH}"

# Copy built frontend
COPY --from=frontend-builder /app/dist ./dist

# Copy built Rust binary
COPY --from=rust-builder /app/src-tauri/target/release/bili-shadowreplay .

# Expose port
EXPOSE 3000

# Run the application
CMD ["sh", "-c", "nscd && exec ./bili-shadowreplay"]
