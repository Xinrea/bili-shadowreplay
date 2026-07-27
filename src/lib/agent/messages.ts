export type MessageContent = string | unknown[];

export interface ToolCall {
  id: string;
  name: string;
  args: Record<string, unknown>;
  executed: boolean;
  error?: string | null;
}

export interface HumanMessage {
  kind: "human";
  content: string;
  timestamp: string;
}

export interface AssistantMessage {
  kind: "assistant";
  content: MessageContent;
  timestamp: string;
  toolCalls: ToolCall[];
  isError?: boolean;
}

export interface ToolMessage {
  kind: "tool";
  content: MessageContent;
  timestamp: string;
  name: string;
  toolCallId: string;
  status: "success" | "error";
  resolution: "confirmed" | "rejected";
}

export type ChatMessage = HumanMessage | AssistantMessage | ToolMessage;

export function isAssistantMessage(
  message: ChatMessage,
): message is AssistantMessage {
  return message.kind === "assistant";
}

export function isToolMessage(message: ChatMessage): message is ToolMessage {
  return message.kind === "tool";
}

function isRecord(value: unknown): value is Record<string, any> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

export function normalizeToolArguments(value: unknown): Record<string, unknown> {
  let parsed = value;
  if (typeof value === "string") {
    try {
      parsed = JSON.parse(value);
    } catch {
      return {};
    }
  }
  if (!isRecord(parsed)) return {};

  return normalizeArgumentValue(parsed) as Record<string, unknown>;
}

function snakeCaseKey(key: string): string {
  return key.replace(/([a-z0-9])([A-Z])/g, "$1_$2").toLowerCase();
}

function normalizeArgumentValue(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(normalizeArgumentValue);
  if (!isRecord(value)) return value;

  const normalized: Record<string, unknown> = {};
  for (const [key, item] of Object.entries(value)) {
    const canonical = snakeCaseKey(key);
    if (canonical === key) normalized[canonical] = normalizeArgumentValue(item);
  }
  for (const [key, item] of Object.entries(value)) {
    const canonical = snakeCaseKey(key);
    if (!(canonical in normalized)) {
      normalized[canonical] = normalizeArgumentValue(item);
    }
  }
  return normalized;
}

function messageContent(value: unknown): MessageContent {
  if (typeof value === "string" || Array.isArray(value)) return value;
  return value == null ? "" : JSON.stringify(value);
}

function timestamp(value: unknown): string {
  if (typeof value === "string" || typeof value === "number") {
    const date = new Date(value);
    if (!Number.isNaN(date.getTime())) return date.toISOString();
  }
  return new Date().toISOString();
}

export function normalizeToolCalls(value: unknown): ToolCall[] {
  if (!Array.isArray(value)) return [];

  return value.flatMap((raw): ToolCall[] => {
    if (!isRecord(raw) || !raw.id || !raw.name) return [];
    return [{
      id: String(raw.id),
      name: String(raw.name),
      args: normalizeToolArguments(raw.args),
      executed: raw.executed !== false,
      error: raw.error == null ? null : String(raw.error),
    }];
  });
}

function wasRejected(content: MessageContent): boolean {
  if (typeof content !== "string") return false;
  try {
    const value = JSON.parse(content);
    return isRecord(value) && value.rejected === true;
  } catch {
    return false;
  }
}

/**
 * Reads both the current message shape and the former persisted format.
 * Legacy fields are normalized here and never leak into the application.
 */
export function deserializeMessages(stored: unknown): ChatMessage[] {
  if (!Array.isArray(stored)) return [];

  return stored.flatMap((raw): ChatMessage[] => {
    if (!isRecord(raw)) return [];

    const fields = isRecord(raw.kwargs) ? raw.kwargs : raw;
    const legacyId = Array.isArray(raw.id) ? raw.id.join("/") : String(raw.id ?? "");
    const kind = raw.kind ??
      (legacyId.includes("HumanMessage")
        ? "human"
        : legacyId.includes("AIMessage")
          ? "assistant"
          : legacyId.includes("ToolMessage")
            ? "tool"
            : undefined);
    const additional = isRecord(fields.additional_kwargs)
      ? fields.additional_kwargs
      : isRecord(raw.additional_kwargs)
        ? raw.additional_kwargs
        : {};
    const savedTimestamp = fields.timestamp ?? raw.timestamp ?? additional.timestamp;

    if (kind === "human") {
      const content = messageContent(fields.content ?? raw.content);
      return [{
        kind: "human",
        content: typeof content === "string" ? content : JSON.stringify(content),
        timestamp: timestamp(savedTimestamp),
      }];
    }

    if (kind === "assistant") {
      return [{
        kind: "assistant",
        content: messageContent(fields.content ?? raw.content),
        timestamp: timestamp(savedTimestamp),
        toolCalls: normalizeToolCalls(
          fields.toolCalls ?? raw.toolCalls ?? fields.tool_calls ?? raw.tool_calls,
        ),
        isError: fields.isError === true || raw.isError === true || additional.isError === true,
      }];
    }

    if (kind === "tool") {
      const toolCallId = fields.toolCallId ?? raw.toolCallId ??
        fields.tool_call_id ?? raw.tool_call_id;
      if (!toolCallId) return [];
      const content = messageContent(fields.content ?? raw.content);
      const resolution = fields.resolution ?? raw.resolution;
      return [{
        kind: "tool",
        content,
        timestamp: timestamp(savedTimestamp),
        name: String(fields.name ?? raw.name ?? "未知工具"),
        toolCallId: String(toolCallId),
        status: fields.status === "error" || raw.status === "error" ? "error" : "success",
        resolution: resolution === "rejected" || wasRejected(content)
          ? "rejected"
          : "confirmed",
      }];
    }

    return [];
  });
}
