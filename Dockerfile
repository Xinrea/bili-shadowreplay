# Build frontend
FROM node:20-bullseye AS frontend-builder

WORKDIR /app

# Install system dependencies
RUN apt-get update && apt-get install -y \
    python3 \
    make \
    g++ \
    && rm -rf /var/lib/apt/lists/*

# Copy package files
COPY package.json yarn.lock ./

# Install dependencies with specific flags
RUN yarn install --frozen-lockfile

# Copy source files
COPY . .

# Build frontend
RUN yarn build

# Build Rust backend
FROM rust:1.90-slim AS rust-builder

WORKDIR /app

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
    && rm -rf /var/lib/apt/lists/*

# Copy Rust project files
COPY src-tauri/Cargo.toml src-tauri/Cargo.lock ./src-tauri/
COPY src-tauri/src ./src-tauri/src
COPY src-tauri/crates ./src-tauri/crates

# Build Rust backend
WORKDIR /app/src-tauri
RUN rustup component add rustfmt
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
CMD ["sh", "-c", "nscd && ./bili-shadowreplay"]
