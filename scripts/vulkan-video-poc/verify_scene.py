#!/usr/bin/env python3
"""Decode the finished artifact independently; never run inside GPU capture.

Check actual Skia content, not just an encoder-produced black video: background
colour, the moving translucent red sprite, and subtitle appearance/disappearance.
CPU pixels here belong to a separate output verifier, outside the GPU producer.
"""
import argparse
import json
import subprocess
from pathlib import Path


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sample(rgb, width, height, rect):
    left, top, right, bottom = rect
    left, right = int(left * width / 1920), int(right * width / 1920)
    top, bottom = int(top * height / 1080), int(bottom * height / 1080)
    require(0 <= left < right <= width and 0 <= top < bottom <= height, "Invalid sample rectangle")
    sums, bright, count = [0, 0, 0], 0, 0
    for y in range(top, bottom):
        for x in range(left, right):
            pixel = rgb[(y * width + x) * 3:(y * width + x + 1) * 3]
            for channel, value in enumerate(pixel):
                sums[channel] += value
            bright += all(value > 180 for value in pixel)
            count += 1
    return [total / count for total in sums], bright


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--video", type=Path, required=True)
    parser.add_argument("--ffmpeg", default="ffmpeg")
    parser.add_argument("--width", type=int, required=True)
    parser.add_argument("--height", type=int, required=True)
    parser.add_argument("--fps", type=int, required=True)
    parser.add_argument("--frames", type=int, required=True)
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()
    require(args.frames >= 12 and min(args.width, args.height, args.fps) > 0,
            "Scene verification requires at least 12 frames and positive dimensions/rate")
    # Compare SDR code values with the generated scene. An independent CPU RGBA
    # input through this pinned SDK's libplacebo conversion yielded [23,46,93]
    # and [195,12,22]. Allow conversion/4:2:0/codec rounding, while still rejecting
    # swapped channels, missing alpha, blank frames and misplaced sprites.
    # A BT.709 OETF formula is not an oracle for libplacebo's display-referred
    # transfer mapping; colour tags are checked separately by verify.py.
    expected_background = [24, 48, 96]
    expected_sprite = [197, 12, 24]
    results = []
    for index in [0, args.frames // 2, args.frames - 2]:
        result = subprocess.run([
            args.ffmpeg, "-v", "error", "-nostdin", "-i", str(args.video),
            "-vf", f"select=eq(n\\,{index})", "-fps_mode", "passthrough",
            "-frames:v", "1", "-pix_fmt", "rgb24", "-f", "rawvideo", "-",
        ], capture_output=True, check=False)
        require(result.returncode == 0, f"Frame decode failed: {result.stderr.decode(errors='replace')}")
        require(len(result.stdout) == args.width * args.height * 3, "Decoded frame has wrong size")
        background, _ = sample(result.stdout, args.width, args.height, (40, 900, 180, 1030))
        x = 240 + 1200 * index / args.frames
        sprite, _ = sample(result.stdout, args.width, args.height, (x + 40, 200, x + 120, 280))
        _, bright = sample(result.stdout, args.width, args.height, (500, 440, 1420, 640))
        require(max(abs(a - b) for a, b in zip(background, expected_background)) <= 6,
                f"Frame {index}: background colour {background}, expected {expected_background}")
        require(max(abs(a - b) for a, b in zip(sprite, expected_sprite)) <= 8,
                f"Frame {index}: moving translucent sprite {sprite}, expected {expected_sprite}")
        if index == args.frames // 2:
            require(bright > max(8, args.width * args.height // 5000),
                    f"Frame {index}: generated subtitle was not drawn ({bright} white pixels)")
        else:
            require(bright == 0, f"Frame {index}: subtitle leaked outside its cue ({bright} white pixels)")
        results.append({"frame": index, "background_rgb": background,
                        "sprite_rgb": sprite, "subtitle_white_pixels": bright})
    receipt = {"scope": "independent post-encode CPU decode, outside the captured GPU process",
               "expected_background_rgb": expected_background,
               "expected_sprite_rgb": expected_sprite, "samples": results}
    output = json.dumps(receipt, indent=2)
    if args.json:
        args.json.write_text(output + "\n", encoding="utf-8")
    print(output)


if __name__ == "__main__":
    main()
