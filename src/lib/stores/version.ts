import { writable } from "svelte/store";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { TAURI_ENV } from "../invoker";

export const hasNewVersion = writable(false);
export const latestVersion = writable("");

type UpdatePhase = "idle" | "checking" | "check-error" | "available" | "downloading" | "installing" | "restarting" | "restart-required" | "error";
interface UpdateStatus {
  phase: UpdatePhase;
  downloaded: number;
  total?: number;
  error?: string;
}

export const updateStatus = writable<UpdateStatus>({ phase: "idle", downloaded: 0 });
let pendingUpdate: Update | null = null;
let checking: Promise<void> | null = null;
let installing = false;
let installed = false;

// GitHub release tags use stable SemVer versions; a different tag is not
// necessarily newer (e.g. a development build ahead of the latest release).
export function isNewerVersion(candidate: string, current: string): boolean {
  const parse = (value: string) => /^v?(\d+)\.(\d+)\.(\d+)(-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/.exec(value);
  const next = parse(candidate);
  const previous = parse(current);
  if (!next || !previous || next[4]) return false;
  for (let index = 1; index <= 3; index++) {
    const difference = Number(next[index]) - Number(previous[index]);
    if (difference !== 0) return difference > 0;
  }
  return Boolean(previous[4]);
}

export function checkForUpdates(): Promise<void> {
  if (!TAURI_ENV || installing || installed) return Promise.resolve();
  if (checking) return checking;
  updateStatus.set({ phase: "checking", downloaded: 0 });
  checking = (async () => {
    try {
      const update = await check({ timeout: 30_000 });
      const previous = pendingUpdate;
      pendingUpdate = update;
      if (previous) await previous.close().catch(() => {});
      hasNewVersion.set(Boolean(update));
      if (update) latestVersion.set(`v${update.version.replace(/^v/, "")}`);
      updateStatus.set({ phase: update ? "available" : "idle", downloaded: 0 });
    } catch (error) {
      updateStatus.set({ phase: "check-error", downloaded: 0, error: String(error) });
    } finally {
      checking = null;
    }
  })();
  return checking;
}

export async function installUpdate(): Promise<void> {
  if (!TAURI_ENV || installing) return;
  if (checking) await checking;
  if (installing) return;
  if (!pendingUpdate && !installed) await checkForUpdates();
  if ((!pendingUpdate && !installed) || installing) return;
  installing = true;
  let downloaded = 0;
  let total: number | undefined;
  try {
    if (!installed) {
      updateStatus.set({ phase: "downloading", downloaded });
      await pendingUpdate!.downloadAndInstall((event) => {
        if (event.event === "Started") {
          downloaded = 0;
          total = event.data.contentLength;
        } else if (event.event === "Progress") {
          downloaded += event.data.chunkLength;
        }
        updateStatus.set({
          phase: event.event === "Finished" ? "installing" : "downloading",
          downloaded,
          total,
        });
      });
      installed = true;
    }
    updateStatus.set({ phase: "restarting", downloaded, total });
    await relaunch();
  } catch (error) {
    updateStatus.set({
      phase: installed ? "restart-required" : "error",
      downloaded,
      total,
      error: String(error),
    });
  } finally {
    installing = false;
  }
}
