# Video Editing Tools for AI Agent

## Overview

Enhanced the AI agent with professional video editing capabilities, enabling it to analyze content and produce videos like a human editor.

## New Tools Added

### 1. Visual Analysis Tools

#### `extract_video_frames`
- Extracts frames from videos at specific timestamps or evenly distributed
- Returns base64-encoded images for visual content analysis
- Allows the agent to "see" video content
- Parameters:
  - `video_id`: Video ID
  - `timestamps`: Optional array of specific timestamps (seconds)
  - `max_frames`: Maximum frames to extract (default: 10)

#### `get_video_metadata`
- Returns detailed video metadata
- Information includes: duration, resolution, codec, bitrate, FPS, file size
- Helps agent understand technical video properties

#### `get_archive_metadata`
- Gets metadata for archived live streams
- Includes file size, recording time, and video properties

### 2. Content Analysis & Clipping Decision Tools

#### `analyze_danmu_highlights`
- Analyzes comment (danmu) density to find highlight moments
- High comment density = high engagement = interesting content
- Parameters:
  - `time_window`: Time window in seconds for density analysis (e.g., 30)
  - `min_density`: Minimum comments per window to consider as highlight
- Returns: Time ranges with high engagement, sample comments

#### `search_danmu_keywords`
- Searches for specific keywords in comments
- Finds moments mentioned by viewers (e.g., "精彩", "666", "笑死")
- Returns timestamps with context window around matches
- Parameters:
  - `keywords`: Array of keywords to search
  - `context_seconds`: Seconds of context before/after match

### 3. Video Production Tools

#### `clip_range`
- Clips one or more time ranges from a stream into a single video
- Supports multiple ranges via the `ranges` array, concatenated into one output
- Optional transition effects between clips when multiple ranges are used
- Parameters:
  - `ranges`: Array of {start, end} time ranges
  - `title`: Title for the clip
  - `transition`: Optional transition effect (none, fade, dissolve, wipeleft, wiperight, slideup, slidedown)

#### `merge_videos`
- Merges multiple videos into a single video
- Supports optional transition effects between clips (fade, dissolve, wipe, slide)
- Videos concatenated in provided order
- Creates compilations or combines clips
- Parameters:
  - `video_ids`: Array of video IDs to merge
  - `output_title`, `output_note`: Metadata for merged video
  - `transition`: Optional transition effect (none, fade, dissolve, wipeleft, wiperight, slideup, slidedown)

#### `extract_video_audio`
- Extracts audio from video as separate MP3 file
- Useful for audio analysis or audio-only content

## Agent Workflow

The agent now follows a professional editing workflow:

1. **Understand Content**: Get archive/video metadata
2. **Find Highlights**: Analyze comment density for high-engagement moments
3. **Verify Content**: Search keywords to confirm interesting moments
4. **Visual Check**: Extract frames to see what's happening (optional)
5. **Make Decisions**: Determine clip time ranges based on analysis
6. **Execute**: Clip multiple ranges and generate JianYing draft
7. **Edit in JianYing**: User opens the draft in JianYing for final editing and export

## Updated Agent Prompt

The agent prompt has been enhanced with:
- Explanation of new video editing capabilities
- Guidance on when and how to use each tool
- Suggested workflow for video editing tasks
- Instructions to combine multiple analysis methods

## Implementation Status

### Frontend (TypeScript)
- ✅ Tool definitions added to `src/lib/agent/tools.ts`
- ✅ Agent prompt updated in `src/lib/agent/agent.ts`
- ✅ All 8 new tools integrated with LangChain
- ✅ `clip_multiple_ranges` updated to return draft path instead of video IDs

### Backend (Rust)
- ✅ Handler file created at `src-tauri/src/handlers/video_editing.rs`
- ✅ Module registered in `src-tauri/src/handlers/mod.rs`
- ✅ JianYing draft generator implemented at `src-tauri/src/jianying/mod.rs`
- ✅ `clip_multiple_ranges` generates JianYing draft instead of merging videos
- ✅ All commands registered in `src-tauri/src/main.rs`
- ✅ FFmpeg concat file path escaping fixed (only escape `'` and `\`, not `[]`)
- ✅ Audio concat filter added to `merge_videos` for proper multi-clip audio merging

## Recent Fixes

### 1. FFmpeg Concat File Path Escaping (✅ Fixed)
**Problem**: Files with square brackets `[]` in names failed to open with FFmpeg concat demuxer.

**Solution**:
- Only escape single quotes (`'` → `'\''`) and backslashes (`\` → `\\`)
- Do NOT escape square brackets (they work fine inside single quotes)
- Use `-safe 0` flag to disable path safety checks

### 2. LLM API Error Handling (✅ Fixed)
**Problem**: LLM API errors were not caught and displayed to users.

**Solution**:
- Added try-catch wrapper around `agent.stream()` in `src/page/AI.svelte`
- Display error messages with red warning icon and background
- Show helpful troubleshooting tips (check API endpoint, key, network, model name)

### 3. Subtitle Generator Configuration (✅ Fixed)
**Problem**:
- `generate_archive_subtitle` tool hardcoded `"whisper"`, ignoring user config
- Third-party services (powerlive) need opus audio but only mp4 was generated

**Solution**:
- Read `config.subtitle_generator_type` instead of hardcoding
- Extract opus audio for powerlive service: `ffmpeg -i tmp.mp4 -vn -acodec libopus -b:a 128k tmp.opus`

### 4. Multi-Clip Audio Loss (✅ Fixed)
**Problem**: When merging videos with transitions, only the first clip had audio.

**Solution**:
- Added audio concat filter: `[0:a][1:a]...[n:a]concat=n=N:v=0:a=1[outa]`
- Map merged audio: `-map [outa]`

### 5. clip_multiple_ranges Refactor (✅ Completed)
**Change**: Instead of merging clips into one video, now generates JianYing draft for user editing.

**Implementation**:
- Created `src-tauri/src/jianying/mod.rs` module
- Generates `draft_content.json` and `draft_info.json`
- Each clip becomes a segment on the timeline
- User can open draft in JianYing for further editing

## Next Steps

Implementation is complete. Suggested next steps:

1. **Test the JianYing draft generation**:
   - Use AI agent to clip multiple ranges from a live stream
   - Verify draft files are created correctly
   - Open the draft in JianYing to confirm it works

2. **Test video merging with transitions**:
   - Merge multiple videos with different transition effects
   - Verify audio is present in all clips

3. **Test error handling**:
   - Try invalid API configurations to see error messages
   - Verify error messages are helpful and actionable

4. **Consider future enhancements**:
   - Add transition effects to JianYing draft generation
   - Support more JianYing features (text, stickers, filters)
   - Add preview functionality before generating draft

## Usage Example

User: "帮我从最近的直播中剪辑精彩片段"

Agent workflow:
1. get_recent_record_all() - Find recent streams
2. analyze_danmu_highlights() - Find high-engagement moments
3. search_danmu_keywords(["精彩", "666"]) - Verify highlights
4. extract_video_frames() - Check visual content (optional)
5. clip_range() - Clip all highlights into a single video with optional transitions
6. Return clip result to user

## Benefits

- **Automated Content Discovery**: Agent finds highlights without manual review
- **Data-Driven Decisions**: Uses comment density and keywords for objective analysis
- **Visual Verification**: Can "see" content before clipping
- **Efficient Workflow**: Multiple clips in one operation
- **Professional Editing**: JianYing draft allows full editing capabilities (transitions, text, effects)
- **Flexible Output**: User has full control over final video in JianYing

## Technical Notes

- Frame extraction uses FFmpeg with quality setting (-q:v 2)
- Metadata extraction uses FFprobe with JSON output
- Video merging uses FFmpeg concat demuxer for simple concat, xfade filter for transitions
- Audio merging uses concat filter to combine all audio streams
- Audio extraction outputs MP3 with quality 2
- JianYing draft format follows version 3.0.0 specification
- Draft includes materials (videos), tracks (timeline), and canvas (resolution)
- All operations are async and report progress where applicable
- Temporary files are cleaned up automatically
