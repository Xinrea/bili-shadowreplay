import {
  Brain,
  FileVideo,
  History,
  Info,
  LayoutDashboard,
  List,
  Settings,
  Users,
  Video,
} from "lucide-svelte";

/** Main window pages in sidebar order; the label doubles as the page id. */
export const pages = [
  { label: "总览", icon: LayoutDashboard, keywords: ["overview", "dashboard", "summary"] },
  { label: "直播间", icon: Video, keywords: ["rooms", "live", "streamer"] },
  { label: "录播", icon: History, keywords: ["archives", "records", "replay"] },
  { label: "切片", icon: FileVideo, keywords: ["clips", "videos"] },
  { label: "任务", icon: List, keywords: ["tasks", "jobs"] },
  { label: "助手", icon: Brain, keywords: ["assistant", "ai", "agent", "chat"] },
  { label: "账号", icon: Users, keywords: ["accounts", "login", "cookie"] },
  { label: "设置", icon: Settings, keywords: ["settings", "preferences", "config"] },
  { label: "关于", icon: Info, keywords: ["about", "version", "update"] },
];
