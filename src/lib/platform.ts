const labels = new Map<string, string>([
  ["bilibili", "B站"],
  ["douyin", "抖音"],
  ["douyu", "斗鱼"],
  ["huya", "虎牙"],
  ["kuaishou", "快手"],
  ["tiktok", "TikTok"],
  ["twitch", "Twitch"],
  ["youtube", "YouTube"],
]);

/** Display name of a live platform, or the id itself for unknown platforms. */
export function platformLabel(platform: string | undefined): string {
  if (!platform) return "未知";
  return labels.get(platform.toLowerCase()) ?? platform;
}

/**
 * Web page of a live room, or null when the platform has none. Room ids that
 * are already URLs are returned as they are.
 */
export function liveRoomUrl(platform: string | undefined, roomId: string): string | null {
  if (!platform) return null;
  if (roomId.startsWith("http")) return roomId;
  switch (platform.toLowerCase()) {
    case "bilibili":
      return `https://live.bilibili.com/${roomId}`;
    case "douyin":
      return `https://live.douyin.com/${roomId}`;
    case "douyu":
      return `https://www.douyu.com/${roomId}`;
    case "huya":
      return `https://www.huya.com/${roomId}`;
    case "kuaishou":
      return `https://live.kuaishou.com/u/${roomId}`;
    case "tiktok":
      return `https://www.tiktok.com/${roomId}/live`;
    case "twitch":
      return `https://www.twitch.tv/${roomId}`;
    case "youtube": {
      if (roomId.startsWith("legacy-c-")) {
        return `https://www.youtube.com/c/${roomId.slice(9)}/live`;
      }
      if (roomId.startsWith("legacy-user-")) {
        return `https://www.youtube.com/user/${roomId.slice(12)}/live`;
      }
      if (/^[A-Za-z0-9_-]{11}$/.test(roomId)) {
        return `https://www.youtube.com/watch?v=${roomId}`;
      }
      if (roomId.startsWith("UC")) {
        return `https://www.youtube.com/channel/${roomId}/live`;
      }
      const handle = roomId.startsWith("@") ? roomId : `@${roomId}`;
      return `https://www.youtube.com/${handle}/live`;
    }
    default:
      return null;
  }
}
