import { writable } from "svelte/store";

export const hasNewVersion = writable(false);
export const currentVersion = writable("");
export const latestVersion = writable("");
