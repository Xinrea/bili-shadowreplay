#!/usr/bin/env python3
"""Fetch real platform API fixtures for recorder tests.

Mirrors the HTTP request shapes used by:
  - recorder::platforms::bilibili::api
  - recorder::platforms::douyin::api (H5 reflow fallback)
  - recorder::platforms::huya::api (m.huya.com page)
  - recorder::platforms::kuaishou::api (__INITIAL_STATE__)

Required env:
  TEST_BILIBILI_COOKIE
  TEST_DOUYIN_COOKIE

Usage:
  python3 scripts/fetch_test_fixtures.py
"""

from __future__ import annotations

import json
import os
import re
import sys
import urllib.parse
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "src-tauri/crates/recorder/tests/fixtures"
UA = (
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) "
    "AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36"
)

SENSITIVE_QUERY_KEYS = {
    "token",
    "sign",
    "signature",
    "wssecret",
    "wstime",
    "txsecret",
    "txtime",
    "auth_key",
    "key",
    "expires",
    "expire",
    "us",
    "t",
    "fm",
    "u",
    "r",
    "domain",
}


def dump(name: str, obj=None, text: str | None = None) -> None:
    FIXTURES.mkdir(parents=True, exist_ok=True)
    path = FIXTURES / name
    if obj is not None:
        path.write_text(json.dumps(obj, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    else:
        path.write_text(text or "", encoding="utf-8")
    print(f"wrote {path.relative_to(ROOT)} ({path.stat().st_size} bytes)")


def http_get(url: str, cookie: str = "", referer: str = "", ua: str = UA) -> str:
    headers = {"User-Agent": ua}
    if cookie:
        headers["Cookie"] = cookie
    if referer:
        headers["Referer"] = referer
    req = urllib.request.Request(url, headers=headers)
    with urllib.request.urlopen(req, timeout=30) as resp:
        return resp.read().decode("utf-8", errors="replace")


def redact_url(url: str) -> str:
    if not isinstance(url, str) or not url.startswith("http"):
        return url
    try:
        parsed = urllib.parse.urlparse(url)
        qs = urllib.parse.parse_qs(parsed.query, keep_blank_values=True)
        changed = False
        for key in list(qs):
            lower = key.lower()
            if lower in SENSITIVE_QUERY_KEYS or "secret" in lower or "sign" in lower:
                qs[key] = ["REDACTED"]
                changed = True
        if not changed:
            return url
        new_q = urllib.parse.urlencode({k: v[0] for k, v in qs.items()})
        return urllib.parse.urlunparse(
            (parsed.scheme, parsed.netloc, parsed.path, parsed.params, new_q, parsed.fragment)
        )
    except Exception:
        return url


def walk_redact(obj):
    if isinstance(obj, dict):
        return {k: walk_redact(v) for k, v in obj.items()}
    if isinstance(obj, list):
        return [walk_redact(x) for x in obj]
    if isinstance(obj, str):
        return redact_url(obj)
    return obj


def fetch_bilibili(cookie: str) -> None:
    candidates = ["545068", "7734200", "6", "3", "21452505"]
    chosen = None
    for rid in candidates:
        raw = http_get(
            f"https://api.live.bilibili.com/room/v1/Room/get_info?room_id={rid}",
            cookie=cookie,
            referer="https://live.bilibili.com/",
        )
        data = json.loads(raw)
        if data.get("code") != 0:
            continue
        live = data["data"]["live_status"]
        print(
            f"bilibili room {rid}: live={live} title={data['data']['title']!r} "
            f"room_id={data['data']['room_id']}"
        )
        if live == 1 or chosen is None:
            chosen = data
            if live == 1:
                break
    if chosen is None:
        raise RuntimeError("failed to fetch bilibili room info")

    room_id = chosen["data"]["room_id"]
    uid = chosen["data"]["uid"]
    dump("bilibili_room_info.json", obj=walk_redact(chosen))

    # Same shape as get_stream_info(..., Protocol::HttpHls, Format::FMP4, [Avc], Qn::Q10000)
    play_raw = http_get(
        "https://api.live.bilibili.com/xlive/web-room/v2/index/getRoomPlayInfo"
        f"?room_id={room_id}&protocol=1&format=2&codec=0&qn=10000&platform=h5",
        cookie=cookie,
        referer="https://live.bilibili.com/",
    )
    play = json.loads(play_raw)
    print(f"bilibili playurl code={play.get('code')}")
    dump("bilibili_play_url.json", obj=walk_redact(play))

    # Prefer card API (stable without WBI) then shape like space/acc/info fields used in tests.
    card = json.loads(
        http_get(
            f"https://api.bilibili.com/x/web-interface/card?mid={uid}",
            cookie=cookie,
            referer="https://www.bilibili.com/",
        )
    )
    if card.get("code") != 0:
        raise RuntimeError(f"bilibili card API failed: {card}")
    c = card["data"]["card"]
    user_fixture = {
        "code": 0,
        "message": "0",
        "ttl": 1,
        "data": {
            "mid": int(c["mid"]),
            "name": c["name"],
            "face": c["face"],
            "sign": c.get("sign", ""),
        },
    }
    dump("bilibili_user_info.json", obj=user_fixture)

    # Real playlist snippet from the live play URL (using unredacted play response).
    try:
        stream = play["data"]["playurl_info"]["playurl"]["stream"][0]
        fmt = stream["format"][0]
        codec = fmt["codec"][0]
        url = f"{codec['url_info'][0]['host']}{codec['base_url']}{codec['url_info'][0]['extra']}"
        body = http_get(url, referer="https://live.bilibili.com/")
        lines = []
        for line in body.splitlines():
            if line.startswith("http"):
                lines.append(redact_url(line))
            elif line and not line.startswith("#") and "?" in line:
                lines.append(line.split("?", 1)[0])
            else:
                lines.append(line)
            if len(lines) >= 40:
                break
        dump("test_playlist.m3u8", text="\n".join(lines) + "\n")
    except Exception as exc:
        print(f"warn: could not fetch m3u8 snippet: {exc}")


def fetch_douyin(cookie: str) -> None:
    web_rid = "200525029536"
    page = http_get(
        f"https://live.douyin.com/{web_rid}",
        cookie=cookie,
        referer="https://live.douyin.com/",
    )
    # Exact regex used by get_room_owner_sec_uid
    m = re.search(r'\\"sec_uid\\":\\"(.*?)\\"', page)
    if not m:
        raise RuntimeError("failed to extract sec_uid from douyin room page")
    sec_uid = m.group(1)
    m_room = re.search(r'\\"roomId\\":\\"(\d+)\\"', page)
    if not m_room:
        raise RuntimeError("failed to extract roomId from douyin room page")
    room_id = m_room.group(1)
    print(f"douyin web_rid={web_rid} room_id={room_id} sec_uid={sec_uid}")

    # H5 reflow API — same query as get_room_info_h5
    params = (
        "type_id=0&live_id=1&version_code=99.99.99&app_id=1128"
        f"&room_id={room_id}&sec_user_id={urllib.parse.quote(sec_uid)}"
        "&aid=6383&device_platform=web"
    )
    h5 = json.loads(
        http_get(
            f"https://webcast.amemv.com/webcast/room/reflow/info/?{params}",
            cookie=cookie,
            referer="https://live.douyin.com/",
        )
    )
    if h5.get("status_code") != 0:
        raise RuntimeError(f"douyin H5 API failed: {h5.get('status_code')}")
    dump("douyin_room_info.json", obj=walk_redact(h5))

    offline = json.loads(json.dumps(h5))
    room = offline.setdefault("data", {}).setdefault("room", {})
    room["status"] = 4
    room.pop("stream_url", None)
    dump("douyin_room_offline.json", obj=walk_redact(offline))
    dump(
        "douyin_meta.txt",
        text=(
            f"web_rid={web_rid}\n"
            f"room_id={room_id}\n"
            f"sec_user_id={sec_uid}\n"
            "source=webcast.amemv.com/webcast/room/reflow/info\n"
        ),
    )


def fetch_huya() -> None:
    html = http_get(
        "https://m.huya.com/599934",
        referer="https://m.huya.com/",
        ua=(
            "Mozilla/5.0 (iPhone; CPU iPhone OS 16_0 like Mac OS X) "
            "AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.0 "
            "Mobile/15E148 Safari/604.1"
        ),
    )
    if "HNF_GLOBAL_INIT" not in html:
        raise RuntimeError("huya page missing HNF_GLOBAL_INIT")
    scripts = []
    rest = html
    while True:
        start = rest.find("<script")
        if start < 0:
            break
        rest = rest[start:]
        end = rest.find("</script>")
        if end < 0:
            break
        end += len("</script>")
        block = rest[:end]
        if "HNF_GLOBAL_INIT" in block:
            scripts.append(block)
        rest = rest[end:]
    out = (
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\">"
        "<title>huya fixture</title></head><body>\n"
        + "\n".join(scripts)
        + "\n</body></html>\n"
    )
    out = re.sub(r"https?://[^\s\"'<>]+", lambda m: redact_url(m.group(0)), out)
    dump("huya_room_page.html", text=out)


def extract_initial_state(html: str):
    marker = "window.__INITIAL_STATE__"
    idx = html.find(marker)
    if idx < 0:
        return None
    after = html[idx + len(marker) :]
    eq = after.find("=")
    rest = after[eq + 1 :].lstrip()
    if not rest.startswith("{"):
        return None
    depth = 0
    in_str = False
    escape = False
    end = None
    for i, ch in enumerate(rest):
        if in_str:
            if escape:
                escape = False
            elif ch == "\\":
                escape = True
            elif ch == '"':
                in_str = False
            continue
        if ch == '"':
            in_str = True
        elif ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                end = i + 1
                break
    if end is None:
        return None
    return json.loads(rest[:end])


def fetch_kuaishou() -> None:
    for url in [
        "https://live.kuaishou.com/u/3x4tw52egs9k2mc",
        "https://www.kuaishou.com/u/3x4tw52egs9k2mc",
    ]:
        try:
            html = http_get(url, referer="https://live.kuaishou.com/")
        except Exception as exc:
            print(f"kuaishou {url} failed: {exc}")
            continue
        state = extract_initial_state(html)
        if not state:
            print(f"kuaishou {url}: no __INITIAL_STATE__")
            continue
        dump("kuaishou_initial_state.json", obj=walk_redact(state))
        item = ((state.get("liveroom") or {}).get("playList") or [{}])[0]
        dump(
            "kuaishou_room_info.json",
            obj=walk_redact(
                {
                    "result": 1,
                    "data": {
                        "liveStream": item.get("liveStream") or {},
                        "author": item.get("author") or {},
                        "isLiving": item.get("isLiving", False),
                        "errorType": item.get("errorType"),
                    },
                }
            ),
        )
        dump(
            "kuaishou_room_offline.json",
            obj={
                "result": 1,
                "data": {
                    "liveStream": {"living": False},
                    "author": item.get("author") or {},
                    "isLiving": False,
                },
            },
        )
        return
    raise RuntimeError("failed to fetch kuaishou fixtures")


def main() -> int:
    bili = os.environ.get("TEST_BILIBILI_COOKIE", "").strip()
    douyin = os.environ.get("TEST_DOUYIN_COOKIE", "").strip()
    if not bili or not douyin:
        print(
            "ERROR: set TEST_BILIBILI_COOKIE and TEST_DOUYIN_COOKIE",
            file=sys.stderr,
        )
        return 1

    print("== bilibili ==")
    fetch_bilibili(bili)
    print("== douyin ==")
    fetch_douyin(douyin)
    print("== huya ==")
    fetch_huya()
    print("== kuaishou ==")
    try:
        fetch_kuaishou()
    except Exception as exc:
        print(f"kuaishou skipped: {exc}")
    print("done")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
