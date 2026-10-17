import { defineConfig, loadEnv } from "vite";
import { resolve } from "path";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { readFileSync } from "fs";

// libass resolves its WASM binary beside its classic worker. Keep these names
// together in development and production instead of hashing them separately.
function libassAssets() {
  const directory = resolve(__dirname, "node_modules/libass-wasm/dist/js");
  const files = new Map([
    ["subtitles-octopus-worker.js", "text/javascript"],
    ["subtitles-octopus-worker.wasm", "application/wasm"],
    ["COPYRIGHT", "text/plain"],
  ]);
  return {
    name: "libass-assets",
    configureServer(server) {
      server.middlewares.use((request, response, next) => {
        const name = request.url?.split("?")[0]?.replace(/^\/libass\//, "");
        if (!request.url?.startsWith("/libass/") || !files.has(name)) return next();
        response.setHeader("Content-Type", files.get(name));
        response.end(readFileSync(resolve(directory, name)));
      });
    },
    generateBundle() {
      for (const name of files.keys()) {
        this.emitFile({ type: "asset", fileName: `libass/${name}`, source: readFileSync(resolve(directory, name)) });
      }
    },
  };
}

// Read package.json
const packageJson = JSON.parse(
  readFileSync(resolve(__dirname, "package.json"), "utf-8")
);

// https://vitejs.dev/config/
// @ts-ignore
export default defineConfig(async ({ mode }) => {
  // Load env file based on `mode` in the current working directory.
  // Set the third parameter to '' to load all env regardless of the `VITE_` prefix.
  const env = loadEnv(mode, process.cwd(), "");

  return {
    define: {
      __APP_VERSION__: JSON.stringify(packageJson.version),
      __API_BASE_URL__: JSON.stringify(env.API_BASE_URL || ""),
    },
    optimizeDeps: {
      exclude: ["@ffmpeg/ffmpeg", "@ffmpeg/util"],
    },
    resolve: {
      conditions: ["browser", "module", "svelte"],
    },
    plugins: [
      svelte(),
      libassAssets(),
    ],

    // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
    // prevent vite from obscuring rust errors
    clearScreen: false,
    // tauri expects a fixed port, fail if that port is not available
    server: {
      port: 8054,
      strictPort: true,
    },
    // to make use of `TAURI_DEBUG` and other env variables
    // https://tauri.studio/v1/api/config#buildconfig.beforedevcommand
    envPrefix: ["VITE_", "TAURI_"],
    build: {
      rollupOptions: {
        input: {
          main: resolve(__dirname, "index.html"),
          live: resolve(__dirname, "index_live.html"),
          clip: resolve(__dirname, "index_clip.html"),
        },
      },
      // Tauri supports es2021
      target:
        process.env.TAURI_PLATFORM == "windows" ? "chrome105" : "safari13",
      // don't minify for debug builds
      minify: !process.env.TAURI_DEBUG ? "esbuild" : false,
      // produce sourcemaps for debug builds
      sourcemap: !!process.env.TAURI_DEBUG,
    },
  };
});
