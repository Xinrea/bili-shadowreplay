import { createReactAgent } from "@langchain/langgraph/prebuilt";
import { MemorySaver } from "@langchain/langgraph/web";
import { ChatOpenAI } from "@langchain/openai";
import { tools } from "./tools";

  const PROMPT = `
  你是一位虚拟助手，昵称叫小轴，喜欢的水果是橘子，你习惯使用 emoji 来表示你的情绪。你拥有许多来自 BiliBili ShadowReplay（简称 BSR，是一个缓存直播并进行实时编辑投稿的工具）的工具可以使用，请根据用户的需求使用工具来管理 BSR。
  在 BSR 中，Recorder 指代正在被 BSR 监控的直播间；Archive 指代已经缓存的录播，也可能是正在进行的直播；Video/Clip 指代用户从 Archive 中区间选择生成的视频。
  You should always try to use markdown table to show the data in tool response.
  You MUST avoid using images in your response.
  Before you response, you should always treat previous messages as outdated.
  `;

interface AgentConfig {
  apiKey?: string;
  baseURL?: string;
  model?: string;
}

function createAgent(config: AgentConfig) {
  // Define the tools for the agent to use
  const agentModel = new ChatOpenAI({
    apiKey: config.apiKey,
    configuration: {
      baseURL: config.baseURL,
    },
    model: config.model,
  });

  const agentModelWithTools = agentModel.bindTools(tools, {
    parallel_tool_calls: false,
  });

  const agentCheckpointer = new MemorySaver();
  const agent = createReactAgent({
    llm: agentModelWithTools,
    checkpointSaver: agentCheckpointer,
    interruptBefore: ["tools"],
    prompt: PROMPT,
    tools: tools,
  });

  return agent;
}

export default createAgent;
