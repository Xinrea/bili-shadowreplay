import { defineConfig } from "vitepress";

// https://vitepress.dev/reference/site-config
export default defineConfig({
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
          { text: "安装准备", link: "/getting-started/installation" },
          { text: "配置使用", link: "/getting-started/configuration" },
        ],
      },
      {
        text: "说明文档",
        items: [
          { text: "功能说明", link: "/usage/features" },
          { text: "常见问题", link: "/usage/faq" },
        ],
      },
    ],

    socialLinks: [
      { icon: "github", link: "https://github.com/Xinrea/bili-shadowreplay" },
    ],
  },
});
