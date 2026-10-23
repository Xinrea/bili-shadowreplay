import DefaultTheme from "vitepress/theme";
import type { Theme } from "vitepress";
import HomePage from "./home/HomePage.vue";

export default {
  extends: DefaultTheme,
  enhanceApp({ app }) {
    app.component("HomePage", HomePage);
  },
} satisfies Theme;
