/**
 * 虎牙直播播放器URL构建函数
 * 基于对虎牙直播JavaScript代码的深入分析
 * 谢谢 Claude 4.5 Sonnet
 */

/**
 * 构建播放器URL
 * @param {Object} info - 播放器配置信息
 * @param {string} info.url - 解码后的基础URL
 * @param {string} info.sStreamName - 流名称
 * @param {string} info.presenterUid - 主播UID
 * @param {string} info.sFlvAntiCode - FLV防码参数
 * @param {string} info.sHlsAntiCode - HLS防码参数
 * @param {string} info.sP2pAntiCode - P2P防码参数
 * @param {number} info.uid - 用户ID
 * @param {string} info.sGuid - 设备GUID
 * @param {number} info.appid - 应用ID
 * @param {string} info.type - 播放器类型 (P2PFLV/HLS)
 * @param {number} info.playTimeout - 播放超时时间
 * @param {string} info.h5Root - H5根路径
 * @returns {string} 完整的播放URL
 */
function buildPlayerUrl(info) {
  // 验证必需参数
  if (!info.url) {
    throw new Error("URL is required");
  }

  let baseUrl = info.url;

  // 确保URL以?开头，如果没有则添加
  if (!baseUrl.includes("?")) {
    baseUrl += "?";
  } else if (!baseUrl.endsWith("&") && !baseUrl.endsWith("?")) {
    baseUrl += "&";
  }

  // 根据播放器类型添加防码参数
  if (info.type === "P2PFLV" && info.sFlvAntiCode) {
    baseUrl += info.sFlvAntiCode;
  } else if (info.type === "HLS" && info.sHlsAntiCode) {
    baseUrl += info.sHlsAntiCode;
  } else if (info.type === "P2P" && info.sP2pAntiCode) {
    baseUrl += info.sP2pAntiCode;
  }

  // 添加用户身份参数
  if (info.uid !== undefined) {
    baseUrl += "&uid=" + encodeURIComponent(info.uid);
  }

  if (info.sGuid) {
    baseUrl += "&sGuid=" + encodeURIComponent(info.sGuid);
  }

  if (info.appid !== undefined) {
    baseUrl += "&appid=" + encodeURIComponent(info.appid);
  }

  // 添加流信息参数
  if (info.sStreamName) {
    baseUrl += "&sStreamName=" + encodeURIComponent(info.sStreamName);
  }

  if (info.presenterUid) {
    baseUrl += "&presenterUid=" + encodeURIComponent(info.presenterUid);
  }

  // 添加播放配置参数
  if (info.playTimeout) {
    baseUrl += "&playTimeout=" + encodeURIComponent(info.playTimeout);
  }

  if (info.h5Root) {
    baseUrl += "&h5Root=" + encodeURIComponent(info.h5Root);
  }

  // 添加动态参数
  const timestamp = Date.now();
  baseUrl += "&t=" + timestamp;

  // 生成序列ID（模拟播放器内部逻辑）
  const seqId = generateSeqId();
  baseUrl += "&seqId=" + seqId;

  // 添加其他必要参数
  baseUrl += "&ver=1";
  baseUrl += "&sv=" + getVersion();

  return baseUrl;
}

/**
 * 生成序列ID
 * 模拟播放器内部的getAnticodeSeqid()方法
 * @returns {string} 序列ID
 */
function generateSeqId() {
  // 模拟播放器内部序列ID生成逻辑
  const timestamp = Date.now();
  const random = Math.floor(Math.random() * 1000000);
  return timestamp + "_" + random;
}

/**
 * 获取版本号
 * 模拟播放器内部的版本获取逻辑
 * @returns {string} 版本号
 */
function getVersion() {
  // 模拟虎牙直播的版本号格式
  const now = new Date();
  const year = now.getFullYear();
  const month = String(now.getMonth() + 1).padStart(2, "0");
  const day = String(now.getDate()).padStart(2, "0");
  const hour = String(now.getHours()).padStart(2, "0");
  const minute = String(now.getMinutes()).padStart(2, "0");

  return `${year}${month}${day}${hour}${minute}`;
}

/**
 * 从liveLineUrl构建完整播放URL
 * @param {string} liveLineUrl - Base64编码的liveLineUrl
 * @param {Object} streamInfo - 流信息对象
 * @param {Object} userInfo - 用户信息对象
 * @returns {string} 完整的播放URL
 */
function buildUrlFromLiveLineUrl(liveLineUrl, streamInfo, userInfo) {
  // 解码liveLineUrl
  const decodedUrl = atob(liveLineUrl);

  // 构建播放器配置
  const playerInfo = {
    url: decodedUrl,
    sStreamName: streamInfo.sStreamName,
    presenterUid: streamInfo.presenterUid,
    sFlvAntiCode: streamInfo.sFlvAntiCode,
    sHlsAntiCode: streamInfo.sHlsAntiCode,
    sP2pAntiCode: streamInfo.sP2pAntiCode,
    uid: userInfo.uid || 0,
    sGuid: userInfo.sGuid || "",
    appid: userInfo.appid || 66,
    type: streamInfo.type || "P2PFLV",
    playTimeout: streamInfo.playTimeout || 5000,
    h5Root: "https://hd.huya.com/cdn_libs/mobile/",
  };

  return buildPlayerUrl(playerInfo);
}

/**
 * 解析虎牙直播URL参数
 * @param {string} url - 完整的播放URL
 * @returns {Object} 解析后的参数对象
 */
function parsePlayerUrl(url) {
  const urlObj = new URL(url);
  const params = {};

  for (const [key, value] of urlObj.searchParams) {
    params[key] = value;
  }

  return {
    baseUrl: urlObj.origin + urlObj.pathname,
    params: params,
  };
}

/**
 * 验证播放URL是否有效
 * @param {string} url - 播放URL
 * @returns {boolean} 是否有效
 */
function validatePlayerUrl(url) {
  try {
    const urlObj = new URL(url);
    const params = urlObj.searchParams;

    // 检查必需参数
    const requiredParams = ["uid", "sGuid", "appid", "seqId", "t"];
    for (const param of requiredParams) {
      if (!params.has(param)) {
        return false;
      }
    }

    return true;
  } catch (e) {
    return false;
  }
}

console.log("虎牙直播播放器URL构建函数已加载");

// 示例用法
const exampleInfo = {
  url: "https://tx.hls.huya.com/src/431653844-431653844-1853939143172685824-863431144-10057-A-0-1-imgplus.m3u8?ratio=2000&wsSecret=725304fc2867cbe6254f12b264055136&wsTime=68fb9aa9&fm=RFdxOEJjSjNoNkRKdDZUWV8kMF8kMV8kMl8kMw%3D%3D&ctype=tars_mobile&fs=bgct&t=103",
  sStreamName:
    "431653844-431653844-1853939143172685824-863431144-10057-A-0-1-imgplus",
  presenterUid: 431653844,
  sFlvAntiCode:
    "wsSecret=820369d885b161baa5a7a82170881d78&wsTime=68fb97be&fm=RFdxOEJjSjNoNkRKdDZUWV8kMF8kMV8kMl8kMw%3D%3D&ctype=tars_mobile&fs=bgct&t=103",
  uid: 2246697169,
  sGuid: "0af264cd4955d5688902472c482cb47c",
  appid: 66,
  type: "HLS",
  playTimeout: 5000,
  h5Root: "https://hd.huya.com/cdn_libs/mobile/",
};

const playerUrl = buildPlayerUrl(exampleInfo);
console.log("构建的播放URL:", playerUrl);
