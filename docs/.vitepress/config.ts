import { defineConfig } from "vitepress";
import { withMermaid } from "vitepress-plugin-mermaid";

// https://vitepress.dev/reference/site-config
export default withMermaid({
  title: "BiliBili ShadowReplay",
  description: "直播录制/实时回放/剪辑/投稿工具",
  themeConfig: {
    // https://vitepress.dev/reference/default-theme-config
    nav: [
      { text: "Home", link: "/" },
      {
        text: "Releases",
        link: "https://github.com/Xinrea/bili-shadowreplay/releases",
      },
    ],

    sidebar: [
      {
        text: "开始使用",
        items: [
          {
            text: "安装准备",
            items: [
              {
                text: "桌面端安装",
                link: "/getting-started/installation/desktop",
              },
              {
                text: "Docker 安装",
                link: "/getting-started/installation/docker",
              },
            ],
          },
          {
            text: "配置使用",
            items: [
              { text: "账号配置", link: "/getting-started/config/account" },
              { text: "FFmpeg 配置", link: "/getting-started/config/ffmpeg" },
              { text: "Whisper 配置", link: "/getting-started/config/whisper" },
              { text: "LLM 配置", link: "/getting-started/config/llm" },
            ],
          },
        ],
      },
      {
        text: "说明文档",
        items: [
          {
            text: "功能说明",
            items: [
              { text: "工作流程", link: "/usage/features/workflow" },
              { text: "直播间管理", link: "/usage/features/room" },
              { text: "切片功能", link: "/usage/features/clip" },
              { text: "字幕功能", link: "/usage/features/subtitle" },
              { text: "弹幕功能", link: "/usage/features/danmaku" },
              { text: "Webhook", link: "/usage/features/webhook" },
            ],
          },
          { text: "常见问题", link: "/usage/faq" },
        ],
      },
      {
        text: "开发文档",
        items: [
          {
            text: "DeepWiki",
            link: "https://deepwiki.com/Xinrea/bili-shadowreplay",
          },
        ],
      },
    ],

    socialLinks: [
      { icon: "github", link: "https://github.com/Xinrea/bili-shadowreplay" },
    ],
  },
});
