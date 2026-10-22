import { writable } from "svelte/store";
import {
  default_profile,
  type Profile,
  type ProjectItem,
  type SubtitleStyle,
  type UploadItem,
  type VideoItem,
} from "./interface";
import { parseTimeline, projectCues, visibleSubtitles } from "./project-timeline";
import { buildSubtitleAss, loadSubtitleStyle } from "./subtitle-ass";

/** A material or project another page asked to post. */
export interface UploadRequest {
  source: "material" | "project";
  id: number;
}

/** Set by other pages; the 投稿 page opens a draft for it and clears it. */
export const uploadRequest = writable<UploadRequest | null>(null);

export function requestUpload(source: UploadRequest["source"], id: number) {
  uploadRequest.set({ source, id });
}

/** What a new upload posts. */
export type UploadPick =
  | { source: "material"; item: VideoItem }
  | { source: "project"; item: ProjectItem }
  | { source: "file"; file: string };

/** An upload's stored form, completed with the defaults it may lack. */
export function parseProfile(json: string): Profile {
  try {
    return { ...default_profile(), ...(json ? JSON.parse(json) : {}), videos: [] };
  } catch {
    return default_profile();
  }
}

/** Keep the form as the room's template for its next upload. */
export function saveRoomTemplate(roomId: string, profile: Profile) {
  if (roomId) localStorage.setItem("profile-" + roomId, JSON.stringify(profile));
}

/**
 * The form of a new upload of `title`: the form saved with its project, or a
 * copy of the room's last one.
 */
export function draftProfile(roomId: string, title: string, saved = ""): Profile {
  if (saved) {
    const profile = parseProfile(saved);
    return { ...profile, title: profile.title || title };
  }
  const template = roomId ? localStorage.getItem("profile-" + roomId) : null;
  const profile = template?.includes("videos") ? parseProfile(template) : default_profile();
  return { ...profile, title, videos: [] };
}

/**
 * The subtitles of a project as its editor renders them at the project's frame
 * rate `fps`, or null without any.
 */
export function projectSubtitleAss(project: ProjectItem, fps: number): string | null {
  const timeline = parseTimeline(project.timeline);
  if (visibleSubtitles(timeline).length === 0) return null;
  const style =
    (timeline.subtitle_style as SubtitleStyle | null) ?? loadSubtitleStyle(project.room_id);
  return buildSubtitleAss(projectCues(timeline, fps), style);
}

export function uploadTitle(upload: UploadItem) {
  return parseProfile(upload.profile).title || "未命名投稿";
}

export function isUploadActive(upload: UploadItem) {
  return upload.status === "exporting" || upload.status === "posting";
}

export const sourceLabels: Record<UploadItem["source"], string> = {
  material: "素材",
  project: "切片",
  file: "视频文件",
};

/** How the video to upload is found, for the source's current state. */
export function sourceDescription(
  upload: Pick<UploadItem, "source" | "source_state" | "danmu" | "danmu_embedded">,
) {
  if (upload.source_state === "missing") {
    return upload.source === "file" ? "视频文件已不存在，无法投稿。" : "来源已被删除，无法投稿。";
  }
  switch (upload.source) {
    case "material":
      return upload.source_state === "render"
        ? "投稿时为素材压制弹幕后生成视频。"
        : upload.danmu_embedded
          ? "直接使用素材的视频文件投稿，弹幕已压制在画面中。"
          : "直接使用素材的视频文件投稿。";
    case "file":
      return "直接使用选定的视频文件投稿。";
    default:
      if (upload.danmu_embedded) {
        return upload.source_state === "render"
          ? "投稿时按切片的时间轴生成视频，素材画面中已压制弹幕。"
          : "切片只包含一个完整素材，直接使用素材文件投稿，弹幕已压制在画面中。";
      }
      return upload.source_state === "render"
        ? `投稿时按切片的时间轴生成视频，${upload.danmu ? "压制弹幕" : "不压制弹幕"}。`
        : "切片只包含一个完整素材，直接使用素材文件投稿。";
  }
}
