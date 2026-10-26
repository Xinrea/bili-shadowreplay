"""Fast checks for fixture/report helpers; no Cargo or FFmpeg required."""
import hashlib
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch
import zlib

import run


class HarnessTests(unittest.TestCase):
    def test_fixtures_are_deterministic_distinct_rgba_pngs(self):
        with tempfile.TemporaryDirectory(dir=run.HERE) as temporary:
            with patch.object(run, "OUT", Path(temporary)):
                assets = run.fixtures()
                paths = sorted(assets.glob("*.png"))
                self.assertEqual(len(paths), 100)
                hashes = [hashlib.sha256(path.read_bytes()).digest() for path in paths]
                self.assertEqual(len(set(hashes)), 100)
                run.fixtures()
                self.assertEqual(hashes, [hashlib.sha256(path.read_bytes()).digest() for path in paths])
                data = paths[0].read_bytes()
                self.assertEqual(data[:8], b"\x89PNG\r\n\x1a\n")
                offset, compressed = 8, b""
                while offset < len(data):
                    length = struct.unpack(">I", data[offset:offset + 4])[0]
                    kind = data[offset + 4:offset + 8]
                    payload = data[offset + 8:offset + 8 + length]
                    crc = struct.unpack(">I", data[offset + 8 + length:offset + 12 + length])[0]
                    self.assertEqual(crc, zlib.crc32(kind + payload))
                    if kind == b"IHDR":
                        self.assertEqual(struct.unpack(">IIBBBBB", payload), (64, 64, 8, 6, 0, 0, 0))
                    if kind == b"IDAT":
                        compressed += payload
                    offset += length + 12
                pixels = zlib.decompress(compressed)
                self.assertEqual(len(pixels), 64 * (1 + 64 * 4))
                alphas = [pixels[y * 257 + 1 + x * 4 + 3] for y in range(64) for x in range(64)]
                self.assertIn(0, alphas)
                self.assertTrue(any(0 < alpha < 255 for alpha in alphas))

    def test_medians_preserve_nested_metrics_not_labels_or_booleans(self):
        rows = [{"wall_ms": n, "render": {"frames": 360, "image_cache_misses": n},
                 "scene": "repeated", "profiled": True, "ffmpeg_drain_ms": None}
                for n in (9, 1, 5)]
        self.assertEqual(run.numeric_medians(rows), {
            "wall_ms": 5, "render": {"frames": 360, "image_cache_misses": 5},
        })


if __name__ == "__main__":
    unittest.main()
