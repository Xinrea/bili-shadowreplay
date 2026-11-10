#!/usr/bin/env python3
"""Run catalog integration tests in a disposable loopback-only RustFS container.

No credentials are written to disk or printed. The application's configured
storage and any existing RustFS container are never accessed.
"""
import hashlib
import hmac
import os
from pathlib import Path
import secrets
import subprocess
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone

ROOT = Path(__file__).resolve().parent.parent


def create_bucket(endpoint, bucket, access, secret):
    """Sign one empty PutBucket request with SigV4 using the standard library."""
    now = datetime.now(timezone.utc)
    date, stamp = now.strftime("%Y%m%d"), now.strftime("%Y%m%dT%H%M%SZ")
    host = endpoint.removeprefix("http://")
    payload = hashlib.sha256(b"").hexdigest()
    headers = f"host:{host}\nx-amz-content-sha256:{payload}\nx-amz-date:{stamp}\n"
    names = "host;x-amz-content-sha256;x-amz-date"
    canonical = f"PUT\n/{bucket}\n\n{headers}\n{names}\n{payload}"
    scope = f"{date}/us-east-1/s3/aws4_request"
    signing = f"AWS4-HMAC-SHA256\n{stamp}\n{scope}\n{hashlib.sha256(canonical.encode()).hexdigest()}"
    key = ("AWS4" + secret).encode()
    for part in [date, "us-east-1", "s3", "aws4_request"]:
        key = hmac.new(key, part.encode(), hashlib.sha256).digest()
    signature = hmac.new(key, signing.encode(), hashlib.sha256).hexdigest()
    request = urllib.request.Request(endpoint + "/" + bucket, data=b"", method="PUT", headers={
        "x-amz-date": stamp,
        "x-amz-content-sha256": payload,
        "Authorization": f"AWS4-HMAC-SHA256 Credential={access}/{scope}, SignedHeaders={names}, Signature={signature}",
    })
    # Do not redirect signed requests or send local traffic through a proxy.
    class NoRedirect(urllib.request.HTTPRedirectHandler):
        def redirect_request(self, *args, **kwargs):
            return None
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    with opener.open(request, timeout=2) as response:
        assert response.status == 200


def main():
    name = "bsr-json-probe-" + secrets.token_hex(6)
    env = os.environ.copy()
    access, secret = secrets.token_hex(12), secrets.token_hex(24)
    env.update(RUSTFS_ACCESS_KEY=access, RUSTFS_SECRET_KEY=secret)
    # Use the already installed image; never silently pull another version.
    image = subprocess.check_output(
        ["docker", "image", "inspect", "rustfs/rustfs", "--format", "{{.Id}}"], text=True
    ).strip()
    started = False
    try:
        subprocess.run([
            "docker", "run", "--rm", "-d", "--name", name,
            "-p", "127.0.0.1::9000",
            "-e", "RUSTFS_ACCESS_KEY", "-e", "RUSTFS_SECRET_KEY",
            "-e", "RUSTFS_CONSOLE_ENABLE=false",
            "-e", "RUSTFS_OBS_LOGGER_LEVEL=error", image, "/data",
        ], env=env, check=True, stdout=subprocess.DEVNULL, timeout=30)
        started = True
        port = subprocess.check_output(["docker", "port", name, "9000/tcp"], text=True).strip()
        endpoint = "http://" + port
        deadline = time.monotonic() + 60
        while True:
            try:
                create_bucket(endpoint, "catalog-probe", access, secret)
                break
            except (urllib.error.URLError, TimeoutError):
                if time.monotonic() >= deadline:
                    raise RuntimeError("RustFS did not become ready") from None
                time.sleep(1)
        env.update(S3_TEST_ENDPOINT=endpoint, S3_TEST_BUCKET="catalog-probe",
                   S3_TEST_ACCESS_KEY_ID=access, S3_TEST_SECRET_ACCESS_KEY=secret)
        # Prevent ambient STS test credentials from contaminating these tests.
        env.pop("S3_TEST_SESSION_TOKEN", None)
        tests = [
            ["-p", "recording_catalog", "--test", "catalog", "rustfs_catalog_conformance"],
            ["-p", "recording_storage", "--test", "s3_rustfs", "rustfs_recording_lifecycle"],
            ["-p", "recorder", "--lib",
             "danmu::writer_tests::batching::rustfs_batches_events_and_resumes_existing_log"],
            ["-p", "bili-shadowreplay", "--no-default-features", "--features", "headless",
             "database::catalog::runtime::tests::rustfs_production_catalog_transfers_all_directions_and_remounts"],
        ]
        for args in tests:
            subprocess.run(
                ["cargo", "test", "--manifest-path", "src-tauri/Cargo.toml", "--locked"]
                + args + ["--", "--ignored", "--exact", "--nocapture"],
                cwd=ROOT, env=env, check=True, timeout=120,
            )
    finally:
        if started:
            subprocess.run(["docker", "stop", "--time", "5", name], check=True,
                           stdout=subprocess.DEVNULL, timeout=20)


if __name__ == "__main__":
    main()
