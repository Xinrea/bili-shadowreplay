import { invoke } from "../invoker";
import {
  default_profile,
  generateEventId,
  type ClipRangeParams,
} from "../interface";
import { normalizeToolArguments } from "./messages";

type ToolArguments = Record<string, unknown>;
type ToolExecutor = (args: ToolArguments) => Promise<unknown>;

interface ExtractedVideoFrame {
  timestamp: number;
  image_base64: string;
}

function executor<T extends ToolArguments>(
  implementation: (args: T) => Promise<unknown>,
): ToolExecutor {
  return (args) => implementation(args as T);
}

const confirmedToolExecutors: Record<string, ToolExecutor> = {
  remove_account: executor(
    async ({ platform, uid }: { platform: string; uid: number }) =>
      invoke("remove_account", { platform, uid }),
  ),

  add_recorder: executor(
    async ({ platform, room_id, extra }: {
      platform: string;
      room_id: string;
      extra: string;
    }) => invoke("add_recorder", { platform, roomId: room_id, extra }),
  ),

  remove_recorder: executor(
    async ({ platform, room_id }: { platform: string; room_id: string }) =>
      invoke("remove_recorder", { platform, roomId: room_id }),
  ),

  delete_archive: executor(
    async ({ platform, room_id, live_id }: {
      platform: string;
      room_id: string;
      live_id: string;
    }) => invoke("delete_archive", {
      platform,
      roomId: room_id,
      liveId: live_id,
    }),
  ),

  delete_archives: executor(
    async ({ platform, room_id, live_ids }: {
      platform: string;
      room_id: string;
      live_ids: string[];
    }) => invoke("delete_archives", {
      platform,
      roomId: room_id,
      liveIds: live_ids,
    }),
  ),

  delete_background_task: executor(
    async ({ id }: { id: string }) => invoke("delete_task", { id }),
  ),

  get_video_cover: executor(async ({ id }: { id: number }) => ({
    cover: await invoke("get_video_cover", { id }),
  })),

  delete_video: executor(
    async ({ id }: { id: number }) => invoke("delete_video", { id }),
  ),

  get_video_typelist: async () => invoke("get_video_typelist"),

  get_video_subtitle: executor(
    async ({ id }: { id: number }) => invoke("get_video_subtitle", { id }),
  ),

  generate_video_subtitle: executor(
    async ({ id }: { id: number }) => invoke("generate_video_subtitle", { id }),
  ),

  encode_video_subtitle: executor(
    async ({ id, srt_style }: { id: number; srt_style: string }) =>
      invoke("encode_video_subtitle", { id, srtStyle: srt_style }),
  ),

  post_video_to_bilibili: executor(
    async ({ uid, room_id, video_id, title, desc, tag, tid }: {
      uid: number;
      room_id: string;
      video_id: number;
      title: string;
      desc: string;
      tag: string;
      tid: number;
    }) => {
      const profile = default_profile();
      profile.title = title;
      profile.desc = desc;
      profile.tag = tag;
      profile.tid = tid;
      return invoke("upload_procedure", {
        uid,
        eventId: generateEventId(),
        roomId: room_id,
        videoId: video_id,
        profile,
      });
    },
  ),

  clip_range: executor(
    async ({ clip_range_params }: {
      reason: string;
      clip_range_params: Omit<ClipRangeParams, "ranges"> & {
        ranges: Array<{ start: number; end: number }>;
      };
    }) => invoke("clip_range", {
      eventId: generateEventId(),
      params: { ...clip_range_params } as ClipRangeParams,
    }),
  ),

  generic_ffmpeg_command: executor(
    async ({ args }: { args: string[] }) =>
      invoke("generic_ffmpeg_command", { args }),
  ),

  open_clip: executor(
    async ({ video_id }: { video_id: number }) =>
      invoke("open_clip", { videoId: video_id }),
  ),

  list_folder: executor(
    async ({ path }: { path: string }) => invoke("list_folder", { path }),
  ),

  generate_archive_subtitle: executor(
    async ({ platform, room_id, live_id }: {
      platform: string;
      room_id: string;
      live_id: string;
    }) => invoke("generate_archive_subtitle", {
      platform,
      roomId: room_id,
      liveId: live_id,
    }),
  ),

  extract_video_frames: executor(
    async ({ video_id, timestamps, max_frames }: {
      video_id: number;
      timestamps?: number[];
      max_frames?: number;
    }) => {
      const frames = await invoke<ExtractedVideoFrame[]>(
        "extract_video_frames",
        {
          videoId: video_id,
          timestamps: timestamps ?? [],
          maxFrames: max_frames ?? 10,
        },
      );

      // Rig recognizes this response/parts shape as a multimodal tool result.
      // The Rust bridge keeps the response as the tool result required by the
      // provider protocol and sends the parts as a following image message.
      return {
        response: {
          tool: "extract_video_frames",
          video_id,
          frame_count: frames.length,
          frames: frames.map(({ timestamp }, index) => ({ index, timestamp })),
        },
        parts: frames.map(({ image_base64 }) => ({
          type: "image",
          data: image_base64,
          mimeType: "image/jpeg",
        })),
      };
    },
  ),

  get_video_metadata: executor(
    async ({ video_id }: { video_id: number }) =>
      invoke("get_video_metadata", { videoId: video_id }),
  ),

  merge_videos: executor(
    async ({ video_ids, output_title, output_note, transition }: {
      video_ids: number[];
      output_title: string;
      output_note: string;
      transition?: string;
    }) => invoke("merge_videos", {
      videoIds: video_ids,
      outputTitle: output_title,
      outputNote: output_note,
      transition: transition ?? "none",
    }),
  ),

  extract_video_audio: executor(
    async ({ video_id }: { video_id: number }) =>
      invoke("extract_video_audio", { videoId: video_id }),
  ),

  get_archive_metadata: executor(
    async ({ platform, room_id, live_id }: {
      platform: string;
      room_id: string;
      live_id: string;
    }) => invoke("get_archive_metadata", {
      platform,
      roomId: room_id,
      liveId: live_id,
    }),
  ),
};

export async function invokeToolByName(
  name: string,
  args: unknown,
): Promise<unknown> {
  const selectedTool = confirmedToolExecutors[name];
  if (!selectedTool) {
    throw new Error(`Tool ${name} is not available for frontend confirmation`);
  }
  return selectedTool(normalizeToolArguments(args));
}
