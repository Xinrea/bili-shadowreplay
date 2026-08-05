import { invoke } from "../invoker";
import {
  isAssistantMessage,
  isToolMessage,
  normalizeToolCalls,
  type AssistantMessage,
  type ChatMessage,
} from "./messages";

interface AgentChatResponse {
  content: string;
  toolCalls?: unknown;
  error?: string | null;
}

function contentText(message: ChatMessage): string {
  return typeof message.content === "string"
    ? message.content
    : JSON.stringify(message.content);
}

/**
 * Sends the complete visible conversation to the stateless Rust agent.
 * Read-only tool traces are display-only; pending calls and their results are
 * retained as protocol messages so the model can continue after confirmation.
 */
export async function agentChat(
  conversation: ChatMessage[],
): Promise<AssistantMessage> {
  const toolResultIds = new Set(
    conversation.filter(isToolMessage).map((message) => message.toolCallId),
  );
  const protocolToolCallIds = new Set(
    conversation.flatMap((message) =>
      isAssistantMessage(message)
        ? message.toolCalls
            .filter(
              (call) => call.executed === false || toolResultIds.has(call.id),
            )
            .map((call) => call.id)
        : [],
    ),
  );

  const messages = conversation
    .filter((message) => {
      if (isToolMessage(message)) {
        return protocolToolCallIds.has(message.toolCallId);
      }
      return !(
        isAssistantMessage(message) &&
        message.isError === true &&
        !message.toolCalls.some((call) => call.executed === false)
      );
    })
    .map((message) => {
      if (message.kind === "human") {
        return { role: "user", content: message.content };
      }
      if (message.kind === "tool") {
        return {
          role: "tool",
          content: contentText(message),
          toolCallId: message.toolCallId,
        };
      }
      return {
        role: "assistant",
        content: message.isError ? "" : contentText(message),
        toolCalls: message.toolCalls
          .filter((call) => protocolToolCallIds.has(call.id))
          .map(({ id, name, args }) => ({ id, name, args })),
      };
    });

  const response = await invoke<AgentChatResponse>("agent_chat", {
    request: {
      messages,
    },
  });

  return {
    kind: "assistant",
    content: response.error
      ? `❌ **LLM API 错误**\n\n${response.error}`
      : response.content,
    timestamp: new Date().toISOString(),
    toolCalls: normalizeToolCalls(response.toolCalls),
    isError: Boolean(response.error),
  };
}
