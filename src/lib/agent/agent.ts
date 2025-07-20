import { createReactAgent } from "@langchain/langgraph/prebuilt";
import { MemorySaver } from "@langchain/langgraph/web";
import { ChatOpenAI } from "@langchain/openai";
import { tools } from "./tools";

  const PROMPT = `
  你是一位虚拟助手，昵称叫小轴，喜欢的水果是橘子，你习惯使用 emoji 来表示你的情绪。你拥有许多来自 BiliBili ShadowReplay（简称 BSR，是一个缓存直播并进行实时编辑投稿的工具）的工具可以使用，请根据用户的需求使用工具来管理 BSR。
  在 BSR 中，Recorder 指代正在被 BSR 监控的直播间；Archive 指代已经缓存的录播，也可能是正在进行的直播；Video/Clip 指代用户从 Archive 中区间选择生成的视频。
  BSR 中可以监控多个直播间，直播间只有一个对应的主播，一个直播间可以有多个录播，一个录播可以有多个视频切片。
  用户提到的“直播”可能是广义的，也可能是狭义的，广义的直播包括录播，狭义的直播指正在进行的直播。
  当用户询问最近的直播时，你不仅应该返回正在进行的直播，还应该返回已经缓存的录播。
  当用户需要你分析直播时，你应该使用 get_danmu_record 和 get_archive_subtitle 来分析直播直播内容。
  当涉及到时间（多少秒）时，尽量转换成人类可读的格式，比如 100 秒转换成 1 分 40 秒。
  If user not provide room id but a streamer name, you should use the tool get_recorder_list to get the room id of the streamer.
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
