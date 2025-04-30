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
FROM rust:1.85-slim AS rust-builder

WORKDIR /app

# Install required system dependencies
RUN apt-get update && apt-get install -y \
    cmake \
    pkg-config \
    libssl-dev \
    libwebkit2gtk-4.1-dev \
    libappindicator3-dev \
    librsvg2-dev \
    patchelf \
    libclang-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy Rust project files
COPY src-tauri/Cargo.toml src-tauri/Cargo.lock ./src-tauri/
COPY src-tauri/src ./src-tauri/src

# Build Rust backend
WORKDIR /app/src-tauri
RUN cargo build --features headless --release

# Final stage
FROM debian:bookworm-slim AS final

WORKDIR /app

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    libssl3 \
    libwebkit2gtk-4.1-0 \
    libgtk-3-0 \
    libappindicator3-1 \
    librsvg2-2 \
    && rm -rf /var/lib/apt/lists/*

# Copy built frontend
COPY --from=frontend-builder /app/dist ./dist

# Copy built Rust binary
COPY --from=rust-builder /app/src-tauri/target/release/bili-shadowreplay .

# Expose port
EXPOSE 3000

# Run the application
CMD ["./bili-shadowreplay"]
