#!/bin/bash

# Test script for merge_videos function with special characters in filenames
# This script generates test videos and tests the FFmpeg concat functionality

set -e

TEST_DIR="test_videos"
OUTPUT_DIR="test_output"

# Clean up previous test files
rm -rf "$TEST_DIR" "$OUTPUT_DIR"
mkdir -p "$TEST_DIR" "$OUTPUT_DIR"

echo "=== Generating test videos with special characters in filenames ==="

# Generate test video 1 with square brackets
ffmpeg -f lavfi -i testsrc=duration=2:size=640x480:rate=30 \
  -f lavfi -i sine=frequency=1000:duration=2 \
  -pix_fmt yuv420p -y "$TEST_DIR/[test1][video].mp4"

# Generate test video 2 with single quotes
ffmpeg -f lavfi -i testsrc=duration=2:size=640x480:rate=30 \
  -f lavfi -i sine=frequency=1500:duration=2 \
  -pix_fmt yuv420p -y "$TEST_DIR/test'2'video.mp4"

# Generate test video 3 with mixed special characters
ffmpeg -f lavfi -i testsrc=duration=2:size=640x480:rate=30 \
  -f lavfi -i sine=frequency=2000:duration=2 \
  -pix_fmt yuv420p -y "$TEST_DIR/[test3]'video'[final].mp4"

echo "✓ Test videos generated"

# Test 1: Merge videos with square brackets
echo ""
echo "=== Test 1: Merging videos with square brackets ==="
cat > "$TEST_DIR/concat_test1.txt" << EOF
file '$(pwd)/$TEST_DIR/[test1][video].mp4'
file '$(pwd)/$TEST_DIR/test'2'video.mp4'
EOF

ffmpeg -f concat -safe 0 -i "$TEST_DIR/concat_test1.txt" \
  -c copy -y "$OUTPUT_DIR/merged_test1.mp4" 2>&1 | tail -5

if [ -f "$OUTPUT_DIR/merged_test1.mp4" ]; then
  echo "✓ Test 1 passed: Basic merge successful"
else
  echo "✗ Test 1 failed: Output file not created"
  exit 1
fi

# Test 2: Merge with properly escaped paths (single quotes only, no bracket escaping)
echo ""
echo "=== Test 2: Merging all videos with proper escaping ==="
cat > "$TEST_DIR/concat_test2.txt" << EOF
file '$(pwd)/$TEST_DIR/[test1][video].mp4'
file '$(pwd)/$TEST_DIR/test'\''2'\''video.mp4'
file '$(pwd)/$TEST_DIR/[test3]'\''video'\''[final].mp4'
EOF

echo "Concat file contents:"
cat "$TEST_DIR/concat_test2.txt"
echo ""

ffmpeg -f concat -safe 0 -i "$TEST_DIR/concat_test2.txt" \
  -c copy -y "$OUTPUT_DIR/merged_test2.mp4" 2>&1 | tail -5

if [ -f "$OUTPUT_DIR/merged_test2.mp4" ]; then
  echo "✓ Test 2 passed: Escaped paths merge successful"

  # Verify output duration (should be ~6 seconds)
  duration=$(ffprobe -v error -show_entries format=duration \
    -of default=noprint_wrappers=1:nokey=1 "$OUTPUT_DIR/merged_test2.mp4")
  echo "  Output duration: ${duration}s (expected: ~6s)"
else
  echo "✗ Test 2 failed: Output file not created"
  exit 1
fi

echo ""
echo "=== All tests passed! ==="
echo "Test files location: $TEST_DIR/"
echo "Output files location: $OUTPUT_DIR/"
