import flowbitePlugin from "flowbite/plugin";

const token = (name) => `rgb(var(--c-${name}) / <alpha-value>)`;
// A status color: solid, a tinted background, and text that reads on that tint.
const tone = (name) => ({
  DEFAULT: token(name),
  soft: token(`${name}-soft`),
  fg: token(`${name}-fg`),
});

export default {
  content: [
    "./src/**/*.{html,js,svelte,ts}",
    "./node_modules/flowbite-svelte/**/*.{html,js,svelte,ts}",
  ],
  darkMode: "class",
  theme: {
    extend: {
      colors: {
        // Theme tokens, defined in styles.css for light and `.dark`.
        canvas: token("canvas"),
        chrome: token("chrome"),
        surface: token("surface"),
        control: { DEFAULT: token("control"), hover: token("control-hover") },
        sunken: token("sunken"),
        selected: token("selected"),
        line: { DEFAULT: token("line"), strong: token("line-strong") },
        fg: {
          DEFAULT: token("fg"),
          secondary: token("fg-secondary"),
          muted: token("fg-muted"),
          faint: token("fg-faint"),
        },
        accent: tone("accent"),
        success: tone("success"),
        warning: tone("warning"),
        danger: tone("danger"),
        // flowbite-svelte
        primary: {
          50: "#FFF5F2",
          100: "#FFF1EE",
          200: "#FFE4DE",
          300: "#FFD5CC",
          400: "#FFBCAD",
          500: "#FE795D",
          600: "#EF562F",
          700: "#EB4F27",
          800: "#CC4522",
          900: "#A5371B",
        },
      },
    },
  },

  plugins: [flowbitePlugin],
};
