# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

BiliBili ShadowReplay is a Tauri-based desktop application for caching live streams and performing real-time editing and submission. It supports multiple streaming platforms including Bilibili, Douyin (TikTok China), Huya, Kuaishou, and TikTok.

**Architecture**: Hybrid application with Svelte 3 frontend and Rust backend, using Tauri 2 for desktop integration.

## Development Commands

### Frontend Development
- `yarn dev` - Start Vite development server (frontend only)
- `yarn build` - Build production frontend
- `yarn check` - Run TypeScript and Svelte type checking

### Full Application Development
- `yarn tauri dev` - Start Tauri development with hot reload (recommended for full-stack development)
- `yarn tauri build` - Build production desktop application

### Rust Backend
- `cd src-tauri && cargo check` - Check Rust code without building
- `cd src-tauri && cargo test` - Run all Rust tests
- `cd src-tauri && cargo test <test_name>` - Run specific test

### Platform-Specific Builds
- **Windows CPU**: `yarn tauri dev` (default)
- **Windows CUDA**: `yarn tauri dev --features cuda` (requires CUDA Toolkit and LLVM)
- **macOS**: Requires `SDKROOT` and `CMAKE_OSX_DEPLOYMENT_TARGET=13.3` environment variables
- **Linux**: No special configuration needed

### Documentation
- `yarn docs:dev` - Start VitePress documentation server
- `yarn docs:build` - Build documentation site
- `yarn docs:preview` - Preview built documentation

### Version Management
- `yarn bump` - Run version bump script

## Architecture Overview

### Frontend (Svelte 3 + TypeScript)

**Entry Points**:
- `src/main.ts` - Main application entry
- `src/main_clip.ts` - Clip editing interface
- `src/main_live.ts` - Live streaming interface

**Key Directories**:
- `src/page/` - Page components (Room, Task, AI, etc.)
- `src/lib/components/` - Reusable UI components
- `src/lib/stores/` - Svelte stores for global state management
- `src/lib/agent/` - AI agent implementation using LangChain
- `src/lib/db.ts` - Frontend database interface

**Styling**: Tailwind CSS with Flowbite components

### Backend (Rust + Tauri 2)

**Main Entry**: `src-tauri/src/main.rs`

**Core Modules**:
- `src-tauri/src/recorder_manager.rs` - Main recording orchestration
- `src-tauri/src/handlers/` - Tauri command handlers (frontend-backend bridge)
- `src-tauri/src/database/` - SQLite database operations using sqlx
- `src-tauri/src/subtitle_generator/` - AI-powered subtitle generation with Whisper
- `src-tauri/src/ffmpeg/` - FFmpeg integration for video processing
- `src-tauri/src/progress/` - Progress tracking for recording tasks
- `src-tauri/src/http_server/` - HTTP server for streaming
- `src-tauri/src/migration/` - Database schema migration system

**Custom Workspace Crates**:
- `src-tauri/crates/danmu_stream/` - Danmaku (bullet comment) stream processing library
- `src-tauri/crates/recorder/` - Core recording functionality with platform-specific implementations

**Platform Support** (`src-tauri/crates/recorder/src/platforms/`):
- `bilibili/` - Bilibili live stream recording
- `douyin/` - Douyin (TikTok China) recording
- `huya/` - Huya platform support
- `kuaishou/` - Kuaishou platform support
- `tiktok/` - TikTok international support

### Key Technologies

**Frontend**:
- Svelte 3 with TypeScript
- Vite for build tooling
- Tailwind CSS + Flowbite for UI
- LangChain for AI features (@langchain/core, @langchain/deepseek, @langchain/ollama)
- WaveSurfer.js for audio visualization
- Socket.io for real-time communication

**Backend**:
- Tauri 2 for desktop integration
- SQLite with sqlx (async runtime, WAL mode)
- FFmpeg via async-ffmpeg-sidecar
- Whisper-rs for speech-to-text (with optional CUDA/Metal acceleration)
- M3U8-rs for HLS stream processing
- Tokio async runtime
- Axum for HTTP server
- Socketioxide for WebSocket support

### Database Architecture

- **Primary Storage**: SQLite with Write-Ahead Logging (WAL mode)
- **Location**: `src-tauri/data/data_v2.db`
- **Migration System**: Automatic schema updates via `src-tauri/src/migration.rs`
- **Data Models**: Recording metadata, room configurations, task status, user preferences

### AI Features

**Whisper Integration**:
- Local speech-to-text transcription
- Platform-specific acceleration:
  - Windows: Optional CUDA support via `cuda` feature flag
  - macOS: Metal acceleration enabled by default
  - Linux: CPU-based inference

**LangChain Integration**:
- AI agent for content analysis and summarization
- Support for multiple LLM providers (DeepSeek, Ollama)
- Located in `src/lib/agent/` directory

## Development Guidelines

### Frontend Development

- Use Svelte 3 syntax with `<script>` tags
- Prefer reactive statements with `$:` for derived state
- Use stores from `src/lib/stores/` for global state
- Follow TypeScript strict mode configuration
- Use Tailwind CSS classes for styling

### Rust Backend Development

- Follow workspace structure with custom crates
- Use async/await with Tokio runtime
- Implement proper error handling with thiserror
- Use prepared statements for SQL to prevent injection
- Follow Clippy lints (correctness, suspicious, complexity, style, perf all set to "deny")

### Recording System

- Each platform has its own implementation in `src-tauri/crates/recorder/src/platforms/`
- Platform implementations must handle:
  - Stream URL extraction
  - M3U8/HLS stream processing
  - Danmaku capture
  - Quality selection
- Recording orchestration happens in `recorder_manager.rs`
- FFmpeg handles actual video/audio processing

### Testing

- Rust tests: Use `cargo test` in `src-tauri/` directory
- Frontend type checking: Use `yarn check`
- Test individual platform implementations in `src-tauri/crates/recorder/`

## Configuration Files

- `src-tauri/config.example.toml` - User configuration template (FFmpeg path, storage paths, AI settings)
- `src-tauri/tauri.conf.json` - Main Tauri configuration
- Platform-specific Tauri configs: `tauri.{macos,linux,windows,windows.cuda}.conf.json`
- `tsconfig.json` - TypeScript configuration with strict mode
- `tailwind.config.cjs` - Tailwind CSS configuration
- `vite.config.ts` - Vite build configuration

## Common Patterns

### Adding a New Streaming Platform

1. Create platform directory in `src-tauri/crates/recorder/src/platforms/<platform>/`
2. Implement stream info extraction and recording logic
3. Add platform module to `src-tauri/crates/recorder/src/platforms/mod.rs`
4. Update recorder manager to handle new platform
5. Add frontend UI components for platform-specific features

### Tauri Command Handlers

- Located in `src-tauri/src/handlers/`
- Use `#[tauri::command]` attribute
- Return `Result<T, String>` for error handling
- Register in `main.rs` with `.invoke_handler()`

### Database Operations

- Use sqlx with async/await
- Implement migrations in `src-tauri/src/migration/`
- Use transactions for multi-step operations
- Follow WAL mode for concurrent access

## Build Requirements

### Windows
- LLVM with environment variables configured
- CUDA Toolkit (for `cuda` feature, with Visual Studio integration)
- Set `CMAKE_CXX_FLAGS="/utf-8"` if encountering C3688 error

### macOS
- Set `SDKROOT=$(xcrun --sdk macosx --show-sdk-path)`
- Set `CMAKE_OSX_DEPLOYMENT_TARGET=13.3` or higher

### Linux
- No special requirements

### CUDA Builds
- Configure `CMAKE_CUDA_ARCHITECTURES` environment variable
- Reference GitHub Actions workflows for proper configuration

## Documentation

### Documentation Structure

The project maintains comprehensive documentation in the `docs/` directory using VitePress:

- **User Documentation** (`docs/getting-started/`, `docs/usage/`):
  - Installation guides (desktop and Docker)
  - Configuration guides (account, FFmpeg, Whisper, LLM)
  - Feature documentation (workflow, room management, clips, subtitles, danmaku, webhook)
  - FAQ

- **Development Documentation** (`docs/development/`):
  - **Architecture** (`docs/development/architecture/`):
    - `overview.md` - System architecture, tech stack, data flow, and module overview
  - **Frontend** (`docs/development/frontend/`):
    - `stores.md` - Svelte stores and state management patterns
    - `invoker.md` - Tauri command invocation and frontend-backend communication
    - `agent.md` - AI agent implementation with LangChain
  - **Backend** (`docs/development/backend/`):
    - `recorder-manager.md` - Recording orchestration and task management
    - `database.md` - SQLite database schema and operations
  - **Platforms** (`docs/development/platforms/`):
    - `implementation-guide.md` - Guide for implementing new platform support

### Documentation Maintenance Requirements

**IMPORTANT**: When making changes to the codebase, you MUST update the corresponding documentation:

1. **Architecture Changes**:
   - Update `docs/development/architecture/overview.md` when modifying system architecture
   - Update diagrams (Mermaid) to reflect structural changes
   - Document new modules or significant refactoring

2. **Frontend Changes**:
   - Update `docs/development/frontend/stores.md` when adding/modifying stores
   - Update `docs/development/frontend/invoker.md` when adding new Tauri commands
   - Update `docs/development/frontend/agent.md` when modifying AI agent functionality

3. **Backend Changes**:
   - Update `docs/development/backend/recorder-manager.md` when modifying recording logic
   - Update `docs/development/backend/database.md` when changing database schema or operations
   - Add migration documentation when creating new database migrations

4. **Platform Changes**:
   - Update `docs/development/platforms/implementation-guide.md` when adding new platforms
   - Document platform-specific quirks and requirements

5. **User-Facing Changes**:
   - Update user documentation in `docs/getting-started/` and `docs/usage/` for any UI/UX changes
   - Update configuration guides when adding new settings
   - Add FAQ entries for common issues

6. **CLAUDE.md Updates**:
   - Keep this file synchronized with major architectural changes
   - Update development commands when adding new scripts
   - Update common patterns when establishing new conventions

### Documentation Commands

- `yarn docs:dev` - Start VitePress development server at http://localhost:5173
- `yarn docs:build` - Build documentation for production
- `yarn docs:preview` - Preview built documentation

### Documentation Best Practices

- Use Mermaid diagrams for visualizing architecture and flows
- Include code examples with proper syntax highlighting
- Keep documentation concise but comprehensive
- Use consistent terminology across all documentation
- Update documentation in the same commit as code changes
- Review documentation for accuracy during code reviews

