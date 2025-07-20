import { tool } from "@langchain/core/tools";
import { z } from "zod";
import { invoke } from "../invoker";
import { default_profile, generateEventId, type ClipRangeParams, type Profile } from "../interface";

const platform_list = ["bilibili", "douyin"];

// @ts-ignore
const get_accounts = tool(
  async () => {
    const result = (await invoke("get_accounts")) as any;
    // hide cookies in result
    return {
      accounts: result.accounts.map((a: any) => {
        return {
          ...a,
          cookies: "********",
        };
      }),
    };
  },
  {
    name: "get_accounts",
    description: "Get all available accounts",
    schema: z.object({}),
  }
);

// @ts-ignore
const remove_account = tool(
  async ({ platform, uid }: { platform: string; uid: number }) => {
    const result = await invoke("remove_account", {
      platform,
      uid,
    });
    return result;
  },
  {
    name: "remove_account",
    description: "Remove an account",
    schema: z.object({
      platform: z
        .string()
        .describe(
          `The platform of the account. Can be ${platform_list.join(", ")}`
        ),
      uid: z.number().describe("The uid of the account"),
    }),
  }
);

// @ts-ignore
const add_recorder = tool(
  async ({ platform, room_id }: { platform: string; room_id: number }) => {
    const result = await invoke("add_recorder", {
      platform,
      roomId: room_id,
    });
    return result;
  },
  {
    name: "add_recorder",
    description: "Add a recorder",
    schema: z.object({
      platform: z
        .string()
        .describe(
          `The platform of the recorder. Can be ${platform_list.join(", ")}`
        ),
      room_id: z.number().describe("The room id of the recorder"),
    }),
  }
);

// @ts-ignore
const remove_recorder = tool(
  async ({ platform, room_id }: { platform: string; room_id: number }) => {
    const result = await invoke("remove_recorder", {
      platform,
      roomId: room_id,
    });
    return result;
  },
  {
    name: "remove_recorder",
    description: "Remove a recorder",
    schema: z.object({
      platform: z
        .string()
        .describe(
          `The platform of the recorder. Can be ${platform_list.join(", ")}`
        ),
      room_id: z.number().describe("The room id of the recorder"),
    }),
  }
);

// @ts-ignore
const get_recorder_list = tool(
  async () => {
    const result = await invoke("get_recorder_list");
    return result;
  },
  {
    name: "get_recorder_list",
    description: "Get the list of all available recorders",
    schema: z.object({}),
  }
);

// @ts-ignore
const get_recorder_info = tool(
  async ({ platform, room_id }: { platform: string; room_id: number }) => {
    const result = await invoke("get_room_info", { platform, roomId: room_id });
    return result;
  },
  {
    name: "get_recorder_info",
    description: "Get the info of a recorder",
    schema: z.object({
      platform: z.string().describe("The platform of the room"),
      room_id: z.number().describe("The room id of the room"),
    }),
  }
);

// @ts-ignore
const get_archives = tool(
  async ({ room_id }: { room_id: number }) => {
    const archives = (await invoke("get_archives", {
      roomId: room_id,
    })) as any[];
    // hide cover in result
    return {
      archives: archives.map((a: any) => {
        return {
          ...a,
          cover: null,
        };
      }),
    };
  },
  {
    name: "get_archives",
    description: "Get the list of all archives of a recorder",
    schema: z.object({
      room_id: z.number().describe("The room id of the recorder"),
    }),
  }
);

// @ts-ignore
const get_archive = tool(
  async ({ room_id, live_id }: { room_id: number; live_id: string }) => {
    const result = (await invoke("get_archive", {
      roomId: room_id,
      liveId: live_id,
    })) as any;
    // hide cover in result
    return {
      ...result,
      cover: null,
    };
  },
  {
    name: "get_archive",
    description: "Get the info of a archive",
    schema: z.object({
      room_id: z.number().describe("The room id of the recorder"),
      live_id: z.string().describe("The live id of the archive"),
    }),
  }
);

// @ts-ignore
const delete_archive = tool(
  async ({
    platform,
    room_id,
    live_id,
  }: {
    platform: string;
    room_id: number;
    live_id: string;
  }) => {
    const result = await invoke("delete_archive", {
      platform,
      roomId: room_id,
      liveId: live_id,
    });
    return result;
  },
  {
    name: "delete_archive",
    description: "Delete an archive",
    schema: z.object({
      platform: z
        .string()
        .describe(
          `The platform of the recorder. Can be ${platform_list.join(", ")}`
        ),
      room_id: z.number().describe("The room id of the recorder"),
      live_id: z.string().describe("The live id of the archive"),
    }),
  }
);

// @ts-ignore
const get_background_tasks = tool(
  async () => {
    const result = await invoke("get_tasks");
    return {
      tasks: result,
    };
  },
  {
    name: "get_background_tasks",
    description: "Get the list of all background tasks",
    schema: z.object({}),
  }
);

// @ts-ignore
const delete_background_task = tool(
  async ({ id }: { id: string }) => {
    const result = await invoke("delete_task", { id });
    return result;
  },
  {
    name: "delete_background_task",
    description: "Delete a background task",
    schema: z.object({
      id: z.string().describe("The id of the task"),
    }),
  }
);

// @ts-ignore
const get_videos = tool(
  async ({ room_id }: { room_id: number }) => {
    const result = await invoke("get_videos", { roomId: room_id });
    return {
      videos: result,
    };
  },
  {
    name: "get_videos",
    description: "Get the list of all videos of a room",
    schema: z.object({
      room_id: z.number().describe("The room id of the room"),
    }),
  }
);

// @ts-ignore
const get_all_videos = tool(
  async () => {
    const result = await invoke("get_all_videos");
    return result;
  },
  {
    name: "get_all_videos",
    description: "Get the list of all videos from all rooms",
    schema: z.object({}),
  }
);

// @ts-ignore
const get_video = tool(
  async ({ id }: { id: number }) => {
    const result = await invoke("get_video", { id });
    return {
      video: result,
    };
  },
  {
    name: "get_video",
    description: "Get the info of a video",
    schema: z.object({
      id: z.number().describe("The id of the video"),
    }),
  }
);

// @ts-ignore
const get_video_cover = tool(
  async ({ id }: { id: number }) => {
    const result = await invoke("get_video_cover", { id });
    return {
      cover: result,
    };
  },
  {
    name: "get_video_cover",
    description: "Get the cover of a video in base64 format",
    schema: z.object({
      id: z.number().describe("The id of the video"),
    }),
  }
);

// @ts-ignore
const delete_video = tool(
  async ({ id }: { id: number }) => {
    const result = await invoke("delete_video", { id });
    return result;
  },
  {
    name: "delete_video",
    description: "Delete a video",
    schema: z.object({
      id: z.number().describe("The id of the video"),
    }),
  }
);

// @ts-ignore
const get_video_typelist = tool(
  async () => {
    const result = await invoke("get_video_typelist");
    return result;
  },
  {
    name: "get_video_typelist",
    description:
      "Get the list of all video types that can be selected on bilibili platform",
    schema: z.object({}),
  }
);

// @ts-ignore
const get_video_subtitle = tool(
  async ({ id }: { id: number }) => {
    const result = await invoke("get_video_subtitle", { id });
    return result;
  },
  {
    name: "get_video_subtitle",
    description: "Get the subtitle of a video, if empty, you can use generate_video_subtitle to generate the subtitle",
    schema: z.object({
      id: z.number().describe("The id of the video"),
    }),
  }
);

// @ts-ignore
const generate_video_subtitle = tool(
  async ({ id }: { id: number }) => {
    const result = await invoke("generate_video_subtitle", { id });
    return result;
  },
  {
    name: "generate_video_subtitle",
    description: "Generate the subtitle of a video",
    schema: z.object({
      id: z.number().describe("The id of the video"),
    }),
  }
);

// @ts-ignore
const encode_video_subtitle = tool(
  async ({ id, srt_style }: { id: number; srt_style: string }) => {
    const result = await invoke("encode_video_subtitle", {
      id,
      srtStyle: srt_style,
    });
    return result;
  },
  {
    name: "encode_video_subtitle",
    description: "Encode the subtitle of a video",
    schema: z.object({
      id: z.number().describe("The id of the video"),
      srt_style: z
        .string()
        .describe(
          "The style of the subtitle, it is used for ffmpeg -vf force_style, it must be a valid srt style"
        ),
    }),
  }
);

// @ts-ignore
const post_video_to_bilibili = tool(
  async ({ uid, room_id, video_id, title, desc, tag }: { uid: number; room_id: number; video_id: number; title: string; desc: string; tag: string }) => {
    // invoke("upload_procedure", {
    //   uid: uid_selected,
    //   eventId: event_id,
    //   roomId: roomId,
    //   videoId: video.id,
    //   cover: video.cover,
    //   profile: profile,
    // })
    const event_id = generateEventId();
    const cover = await invoke("get_video_cover", { id: video_id });
    let profile = default_profile();
    profile.title = title;
    profile.desc = desc;
    profile.tag = tag;
    const result = await invoke("upload_procedure", { uid, eventId: event_id, roomId: room_id, videoId: video_id, cover, profile });
    return result;
  },
  {
    name: "post_video_to_bilibili",
    description: "Post a video to bilibili",
    schema: z.object({
      uid: z.number().describe("The uid of the user, it should be one of the uid in the bilibili accounts"),
      room_id: z.number().describe("The room id of the room"),
      video_id: z.number().describe("The id of the video"),
      title: z.string().describe("The title of the video"),
      desc: z.string().describe("The description of the video"),
      tag: z.string().describe("The tag of the video, multiple tags should be separated by comma"),
    }),
  }
);

// @ts-ignore
const get_danmu_record = tool(
  async ({ platform, room_id, live_id }: { platform: string; room_id: number; live_id: string }) => {
    const result = (await invoke("get_danmu_record", { platform, roomId: room_id, liveId: live_id })) as any[];
    // remove ts from result
    return {
      danmu_record: result.map((r: any) => {
        return {
          ...r,
          ts: (r.ts / 1000).toFixed(1),
        };
      }),
    };
  },
  {
    name: "get_danmu_record",
    description: "Get the danmu record of a live, entry ts is relative to the live start time in seconds",
    schema: z.object({
      platform: z.string().describe("The platform of the room"),
      room_id: z.number().describe("The room id of the room"),
      live_id: z.string().describe("The live id of the live"),
    }),
  }
);

// @ts-ignore
const clip_range = tool(
  async ({ reason, clip_range_params }: { reason: string; clip_range_params: ClipRangeParams }) => {
    const event_id = generateEventId();
    const result = await invoke("clip_range", { eventId: event_id, params: clip_range_params });
    return result;
  },
  {
    name: "clip_range",
    description: "Clip a range of a live, it will be used to generate a video. If user want you to decide the clip range, you can use get_danmu_record to get the danmu record of the live or get_video_subtitle to get the subtitle of the video, analyze the danmu record or subtitle and decide the clip range. You must provide a reason for your decision on params",
    schema: z.object({
      reason: z.string().describe("The reason for the clip range, it will be shown to the user. You must offer a summary of the clip range content and why you choose this clip range."),
      clip_range_params: z.object({
        room_id: z.number().describe("The room id of the room"),
        live_id: z.string().describe("The live id of the live"),
        x: z.number().describe("The start time in SECONDS of the clip, relative to the live start time, must be less than y"),
        y: z.number().describe("The end time in SECONDS of the clip, relative to the live start time, must be greater than x"),
        danmu: z.boolean().describe("Whether to encode danmu, encode danmu will take a lot of time, so it is recommended to set it to false"),
        offset: z.number().describe("Must be 0"),
        local_offset: z.number().describe("The offset for danmu timestamp, it is used to correct the timestamp of danmu"),
        title: z.string().describe("The title of the clip"),
        cover: z.string().describe("Must be empty"),
        platform: z.string().describe("The platform of the clip"),
      }),
    }),
  }
);

// @ts-ignore
const get_recent_record = tool(
  async ({ room_id, offset, limit }: { room_id: number; offset: number; limit: number }) => {
    const records = (await invoke("get_recent_record", {
      roomId: room_id,
      offset,
      limit,
    })) as any[];
    return {
      records: records.map((r: any) => {
        return { ...r, cover: null };
      }),
    };
  },
  {
    name: "get_recent_record",
    description: "Get the list of recent records that bsr has recorded",
    schema: z.object({
      room_id: z.number().describe("The room id of the room"),
      offset: z.number().describe("The offset of the records"),
      limit: z.number().describe("The limit of the records"),
    }),
  }
);

// @ts-ignore
const generic_ffmpeg_command = tool(
  async ({ args }: { args: string[] }) => {
    const result = await invoke("generic_ffmpeg_command", { args });
    return result;
  },
  {
    name: "generic_ffmpeg_command",
    description: "Run a generic ffmpeg command",
    schema: z.object({
      args: z.array(z.string()).describe("The arguments of the ffmpeg command"),
    }),
  }
);

// @ts-ignore
const open_clip = tool(
  async ({ video_id }: { video_id: number }) => {
    const result = await invoke("open_clip", { videoId: video_id });
    return result;
  },
  {
    name: "open_clip",
    description: "Open a video preview window",
    schema: z.object({
      video_id: z.number().describe("The id of the video"),
    }),
  }
);

// @ts-ignore
const list_folder = tool(
  async ({ path }: { path: string }) => {
    const result = await invoke("list_folder", { path });
    return result;
  },
  {
    name: "list_folder",
    description: "List the files in a folder",
    schema: z.object({
      path: z.string().describe("The path of the folder"),
    }),
  }
);

const tools = [
  get_accounts,
  remove_account,
  add_recorder,
  remove_recorder,
  get_recorder_list,
  get_recorder_info,
  get_archives,
  get_archive,
  delete_archive,
  get_background_tasks,
  delete_background_task,
  get_videos,
  get_all_videos,
  get_video,
  get_video_cover,
  delete_video,
  get_video_typelist,
  get_video_subtitle,
  generate_video_subtitle,
  encode_video_subtitle,
  post_video_to_bilibili,
  clip_range,
  get_danmu_record,
  get_recent_record,
  generic_ffmpeg_command,
  open_clip,
  list_folder,
];

export { tools };
