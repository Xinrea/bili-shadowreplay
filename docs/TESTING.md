# Testing Guide for bili-shadowreplay

## Overview

This guide explains how to run tests for the bili-shadowreplay project and understand the test infrastructure.

## Test Structure

```
src-tauri/
├── crates/
│   ├── recorder/
│   │   ├── src/           # Library code with inline tests
│   │   ├── tests/         # Integration tests
│   │   │   ├── fixtures/  # Test fixtures (JSON, M3U8, etc.)
│   │   │   ├── integration_test.rs
│   │   │   ├── playlist_test.rs
│   │   │   └── helpers_test.rs
│   │   └── Cargo.toml
│   ├── danmu_stream/      # Danmu provider tests
│   └── whisper-cpp-rs/    # (Excluded from CI - requires CMake)
```

## Running Tests Locally

### Prerequisites

1. **System Dependencies** (Ubuntu/Debian):
   ```bash
   sudo apt-get update
   sudo apt-get install -y libssl-dev pkg-config
   ```

2. **Rust Toolchain**:
   ```bash
   rustup update stable
   ```

3. **Git Submodules** (for full build):
   ```bash
   git submodule update --init --recursive
   ```

### Quick Test Commands

```bash
cd src-tauri

# Run all workspace tests (excluding whisper-cpp-rs)
export PKG_CONFIG_PATH=/usr/lib/x86_64-linux-gnu/pkgconfig
cargo test --workspace --lib --exclude whisper-cpp-rs

# Run recorder package tests only
cargo test --package recorder

# Run specific test file
cargo test --package recorder --test integration_test

# Run with verbose output
cargo test --package recorder -- --nocapture

# Run a single test
cargo test --package recorder test_platform_type_string_conversion
```

### Running All Tests (including integration)

```bash
cd src-tauri
export PKG_CONFIG_PATH=/usr/lib/x86_64-linux-gnu/pkgconfig

# Library tests
cargo test --workspace --lib --exclude whisper-cpp-rs

# Integration tests
cargo test --package recorder --test '*'
```

## Test Coverage

### Current Coverage (as of 2026-09-18)

| Component | Status | Notes |
|-----------|--------|-------|
| Platform adapters | ⚠️ Partial | Basic parsing tests, no full mock integration yet |
| Recorder core | ✅ Good | Playlist, HLS recorder internals |
| Pure utilities | ✅ Excellent | Platform type, sanitize filename, etc. |
| Danmu providers | ⚠️ Limited | Basic tests, mostly inline |
| FFmpeg integration | ❌ None | Requires further work |
| End-to-end flows | ❌ None | Future work |

**Total**: ~91 library tests passing (live-network smoke tests are `#[ignore]`)

## Test Infrastructure

### HTTP Mocking

We use [wiremock](https://docs.rs/wiremock/) to mock HTTP calls for platform APIs:

```rust
use wiremock::{MockServer, Mock, ResponseTemplate};
use wiremock::matchers::{method, path};

#[tokio::test]
async fn test_with_mock_server() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/endpoint"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
        .mount(&mock_server)
        .await;

    // Point adapter helpers like get_room_info_with_base(..., &mock_server.uri()) here
}
```

### Test Fixtures

Fixtures are stored in `src-tauri/crates/recorder/tests/fixtures/`.
They are **real API/page responses** captured with the same request shapes as
`recorder::platforms::*::api` (short-lived query tokens are redacted).

Refresh them locally:

```bash
export TEST_BILIBILI_COOKIE='...'
export TEST_DOUYIN_COOKIE='...'
python3 scripts/fetch_test_fixtures.py
```

Included fixtures:

- `bilibili_room_info.json` / `bilibili_play_url.json` / `bilibili_user_info.json`
- `douyin_room_info.json` / `douyin_room_offline.json` (H5 reflow API)
- `huya_room_page.html` (`m.huya.com` + `HNF_GLOBAL_INIT`)
- `kuaishou_initial_state.json` / `kuaishou_room_*.json`
- `test_playlist.m3u8` (real Bilibili fmp4 HLS snippet)

Load fixtures in tests:

```rust
fn load_fixture(name: &str) -> String {
    let fixture_path = format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), name);
    std::fs::read_to_string(fixture_path).unwrap()
}
```

**macOS note:** if linking fails with `unknown architecture arm64e.x1-macos`,
point Cargo at an older SDK, e.g.
`export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX26.sdk`.
## Continuous Integration

Tests run automatically on GitHub Actions for:
- Push to `main` branch
- Pull requests to `main`

See `.github/workflows/test.yml` for the CI configuration.

### CI Workflow

1. Checkout code with submodules
2. Install system dependencies (OpenSSL, pkg-config)
3. Setup Rust toolchain
4. Cache dependencies
5. Run workspace library tests (`cargo test --workspace --lib --exclude whisper-cpp-rs`)
6. Run recorder integration tests (`cargo test --package recorder --tests`)

**Note**: Tests that require CUDA, Whisper models, or long FFmpeg encodes are excluded from CI.
Live-network platform smoke tests are marked `#[ignore]` and must be run manually with
`cargo test -- --ignored`.

## Known Ignored Live Tests

These are optional smoke tests against real platforms (room availability / anti-bot vary):

1. **`platforms::huya::api::tests::test_get_room_info_live`** / **`test_get_user_info`**
2. **`platforms::douyin::api::tests::test_get_room_owner_sec_uid_live`**

CI uses fixture-backed or parse-only coverage instead.

## Writing New Tests

### Unit Tests (Inline)

```rust
// In src-tauri/crates/recorder/src/my_module.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_my_function() {
        assert_eq!(my_function(5), 10);
    }
}
```

### Integration Tests

```rust
// In src-tauri/crates/recorder/tests/my_test.rs
use recorder::core::playlist::HlsPlaylist;

#[tokio::test]
async fn test_integration_scenario() {
    // Test code here
}
```

### Test Guidelines

1. **Use fixtures** for external API responses (never hit production APIs in tests)
2. **Mock HTTP calls** with wiremock for platform adapter tests
3. **Clean up** temp files in tests (`tokio::fs::remove_file` after test)
4. **Test error paths** not just happy paths
5. **Keep tests fast** - CI should complete in <5 minutes
6. **No flaky tests** - tests must be deterministic

## What's NOT Tested Yet

The following areas need test coverage in future work:

1. **Platform adapter full integration**
   - Complete mock-based tests for all 5 platforms
   - Status transition logic (not live → live → recording → ended)
   - Reconnection and error recovery

2. **FFmpeg operations**
   - Command construction (without actually running FFmpeg)
   - Argument validation
   - Error handling for FFmpeg failures

3. **End-to-end recording flows**
   - Would require long-running integration tests
   - Could use small video fixtures + local HTTP server

4. **Frontend tests**
   - Svelte component tests
   - invoker.ts integration tests

## Troubleshooting

### "Could not find openssl"

```bash
export PKG_CONFIG_PATH=/usr/lib/x86_64-linux-gnu/pkgconfig
# Or install: sudo apt-get install libssl-dev pkg-config
```

### "Whisper.cpp CMakeLists.txt not found"

```bash
git submodule update --init --recursive
# Or exclude whisper-cpp-rs: cargo test --exclude whisper-cpp-rs
```

### Test Times Out

Some tests create tokio runtimes - ensure you're using `#[tokio::test]` for async tests.

## Resources

- [Rust Testing Book](https://doc.rust-lang.org/book/ch11-00-testing.html)
- [Wiremock Docs](https://docs.rs/wiremock/)
- [Tokio Testing](https://tokio.rs/tokio/topics/testing)

## Contact

For test infrastructure questions, open an issue.
