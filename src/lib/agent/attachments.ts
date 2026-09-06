import type { MessageAttachment } from "./messages";

export const MAX_ATTACHMENTS = 8;
export const MAX_IMAGE_BYTES = 8 * 1024 * 1024;
export const MAX_TEXT_BYTES = 256 * 1024;
export const FILE_ACCEPT =
  "image/*,.txt,.md,.markdown,.json,.jsonl,.csv,.log,.srt,.ass,.ssa,.xml,.yaml,.yml,.toml,.html,.htm,.css,.js,.ts,.vtt,.lrc";

const IMAGE_EXTENSIONS = new Set([
  "png",
  "jpg",
  "jpeg",
  "gif",
  "webp",
  "heic",
  "heif",
  "bmp",
  "svg",
]);

const TEXT_EXTENSIONS = new Set([
  "txt",
  "md",
  "markdown",
  "json",
  "jsonl",
  "csv",
  "log",
  "srt",
  "ass",
  "ssa",
  "xml",
  "yaml",
  "yml",
  "toml",
  "html",
  "htm",
  "css",
  "js",
  "ts",
  "tsx",
  "jsx",
  "ini",
  "conf",
  "cfg",
  "env",
  "vtt",
  "lrc",
  "sub",
]);

const MIME_BY_EXTENSION: Record<string, string> = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
  heic: "image/heic",
  heif: "image/heif",
  bmp: "image/bmp",
  svg: "image/svg+xml",
  txt: "text/plain",
  md: "text/markdown",
  markdown: "text/markdown",
  json: "application/json",
  jsonl: "application/jsonl",
  csv: "text/csv",
  log: "text/plain",
  srt: "application/x-subrip",
  ass: "text/plain",
  ssa: "text/plain",
  xml: "application/xml",
  yaml: "text/yaml",
  yml: "text/yaml",
  toml: "text/plain",
  html: "text/html",
  htm: "text/html",
  css: "text/css",
  js: "text/javascript",
  ts: "text/plain",
  vtt: "text/vtt",
  lrc: "text/plain",
};

export interface ProtocolContentPart {
  type: "image" | "text";
  name?: string;
  mimeType?: string;
  data?: string;
  text?: string;
}

export interface AttachmentLoadError {
  name: string;
  message: string;
}

type FileKind = "image" | "text";

export function fileNameFromPath(path: string): string {
  return path.split(/[/\\]/).pop() || path;
}

export function protocolPartsFromAttachments(
  attachments: MessageAttachment[],
): ProtocolContentPart[] {
  return attachments.map((attachment) =>
    attachment.kind === "image"
      ? {
        type: "image" as const,
        name: attachment.name,
        mimeType: attachment.mimeType,
        data: attachment.data,
      }
      : {
        type: "text" as const,
        name: attachment.name,
        mimeType: attachment.mimeType,
        text: attachment.text,
      },
  );
}

type LoadResult =
  | { ok: true; attachment: MessageAttachment }
  | { ok: false; error: AttachmentLoadError };

export async function loadAttachmentsFromFiles(
  files: Iterable<File>,
): Promise<{ attachments: MessageAttachment[]; errors: AttachmentLoadError[] }> {
  return collectResults(
    [...files].map((file) => loadAttachmentFromFile(file, file.name, file.type, file.size)),
  );
}

export async function loadAttachmentsFromPaths(
  paths: string[],
): Promise<{ attachments: MessageAttachment[]; errors: AttachmentLoadError[] }> {
  return collectResults(paths.map((path) => loadAttachmentFromPath(path)));
}

export function mergeAttachments(
  current: MessageAttachment[],
  incoming: MessageAttachment[],
): { attachments: MessageAttachment[]; skipped: number } {
  const room = Math.max(0, MAX_ATTACHMENTS - current.length);
  return {
    attachments: [...current, ...incoming.slice(0, room)],
    skipped: Math.max(0, incoming.length - room),
  };
}

async function loadAttachmentFromPath(path: string): Promise<LoadResult> {
  const name = fileNameFromPath(path);
  try {
    const { readFile } = await import("@tauri-apps/plugin-fs");
    const bytes = await readFile(path);
    return loadAttachmentFromBytes(name, "", bytes);
  } catch (error) {
    return {
      ok: false,
      error: {
        name,
        message: error instanceof Error ? error.message : "无法读取文件",
      },
    };
  }
}

async function loadAttachmentFromFile(
  file: Blob,
  name: string,
  mimeType: string,
  size: number,
): Promise<LoadResult> {
  const kind = classifyFile(name, mimeType);
  if (!kind) {
    return unsupported(name);
  }
  if (kind === "image" && size > MAX_IMAGE_BYTES) {
    return tooLarge(name, "图片", MAX_IMAGE_BYTES);
  }
  if (kind === "text" && size > MAX_TEXT_BYTES) {
    return tooLarge(name, "文本", MAX_TEXT_BYTES);
  }

  try {
    if (kind === "image") {
      return {
        ok: true,
        attachment: await imageAttachmentFromBlob(file, name, mimeType),
      };
    }

    const text = await file.text();
    if (text.includes("\0")) {
      return {
        ok: false,
        error: { name, message: "该文件不是可读取的文本" },
      };
    }
    return {
      ok: true,
      attachment: {
        kind: "text",
        name,
        mimeType: mimeType || mimeFromName(name) || "text/plain",
        text,
      },
    };
  } catch (error) {
    return {
      ok: false,
      error: {
        name,
        message: error instanceof Error ? error.message : "无法读取文件",
      },
    };
  }
}

async function loadAttachmentFromBytes(
  name: string,
  mimeType: string,
  bytes: Uint8Array,
): Promise<LoadResult> {
  const inferredMime = mimeType || mimeFromName(name);
  const kind = classifyFile(name, inferredMime);
  if (!kind) {
    return unsupported(name);
  }
  if (kind === "image" && bytes.byteLength > MAX_IMAGE_BYTES) {
    return tooLarge(name, "图片", MAX_IMAGE_BYTES);
  }
  if (kind === "text" && bytes.byteLength > MAX_TEXT_BYTES) {
    return tooLarge(name, "文本", MAX_TEXT_BYTES);
  }

  const copy = new Uint8Array(bytes.byteLength);
  copy.set(bytes);
  const blob = new Blob([copy], { type: inferredMime });
  return loadAttachmentFromFile(blob, name, inferredMime, bytes.byteLength);
}

async function imageAttachmentFromBlob(
  file: Blob,
  name: string,
  mimeType: string,
): Promise<MessageAttachment> {
  const type = mimeType || mimeFromName(name) || "image/jpeg";
  if (
    type === "image/gif" ||
    type === "image/svg+xml" ||
    type === "image/heic" ||
    type === "image/heif" ||
    file.size <= 512 * 1024
  ) {
    return {
      kind: "image",
      name,
      mimeType: type === "image/jpg" ? "image/jpeg" : type,
      data: await blobToBase64(file),
    };
  }

  try {
    return await compressImage(file, name, type);
  } catch {
    return {
      kind: "image",
      name,
      mimeType: type === "image/jpg" ? "image/jpeg" : type,
      data: await blobToBase64(file),
    };
  }
}

async function compressImage(
  file: Blob,
  name: string,
  mimeType: string,
): Promise<MessageAttachment> {
  const objectUrl = URL.createObjectURL(file);
  try {
    const image = await loadHtmlImage(objectUrl);
    const maxEdge = 1920;
    const scale = Math.min(1, maxEdge / Math.max(image.width, image.height));
    const width = Math.max(1, Math.round(image.width * scale));
    const height = Math.max(1, Math.round(image.height * scale));
    const canvas = document.createElement("canvas");
    canvas.width = width;
    canvas.height = height;
    const context = canvas.getContext("2d");
    if (!context) {
      throw new Error("无法压缩图片");
    }
    context.drawImage(image, 0, 0, width, height);
    const keepPng = mimeType === "image/png" || mimeType === "image/webp";
    const outputType = keepPng ? mimeType : "image/jpeg";
    const dataUrl = canvas.toDataURL(outputType, 0.85);
    const data = dataUrl.includes("base64,")
      ? dataUrl.slice(dataUrl.indexOf("base64,") + 7)
      : dataUrl;
    return {
      kind: "image",
      name,
      mimeType: outputType,
      data,
    };
  } finally {
    URL.revokeObjectURL(objectUrl);
  }
}

function loadHtmlImage(src: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => reject(new Error("无法解析图片"));
    image.src = src;
  });
}

function blobToBase64(file: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const result = typeof reader.result === "string" ? reader.result : "";
      resolve(
        result.includes("base64,")
          ? result.slice(result.indexOf("base64,") + 7)
          : result,
      );
    };
    reader.onerror = () => reject(new Error("无法读取文件"));
    reader.readAsDataURL(file);
  });
}

function classifyFile(name: string, mimeType: string): FileKind | null {
  const mime = mimeType.toLowerCase();
  if (mime.startsWith("image/")) return "image";
  if (mime.startsWith("text/") || mime === "application/json" || mime === "application/xml") {
    return "text";
  }
  const ext = extension(name);
  if (IMAGE_EXTENSIONS.has(ext)) return "image";
  if (TEXT_EXTENSIONS.has(ext)) return "text";
  return null;
}

function mimeFromName(name: string): string {
  return MIME_BY_EXTENSION[extension(name)] ?? "";
}

function extension(name: string): string {
  const base = fileNameFromPath(name);
  const index = base.lastIndexOf(".");
  return index >= 0 ? base.slice(index + 1).toLowerCase() : "";
}

function unsupported(name: string) {
  return {
    ok: false as const,
    error: {
      name,
      message: "仅支持图片和文本文件",
    },
  };
}

function tooLarge(name: string, kind: string, maxBytes: number) {
  const maxMb = Math.round(maxBytes / (1024 * 1024));
  return {
    ok: false as const,
    error: {
      name,
      message: `${kind}不能超过 ${maxMb}MB`,
    },
  };
}

async function collectResults(
  jobs: Array<Promise<LoadResult>>,
): Promise<{ attachments: MessageAttachment[]; errors: AttachmentLoadError[] }> {
  const results = await Promise.all(jobs);
  const attachments: MessageAttachment[] = [];
  const errors: AttachmentLoadError[] = [];
  for (const result of results) {
    if (result.ok === true) attachments.push(result.attachment);
    else errors.push(result.error);
  }
  return { attachments, errors };
}
