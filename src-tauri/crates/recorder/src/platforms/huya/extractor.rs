use rand::seq::IndexedRandom;
use regex::Regex;
use reqwest::Url;
use serde_json::Value;

use crate::platforms::huya::url_builder::PlayerInfo;
use crate::platforms::huya::url_builder::UrlBuilder;
use crate::platforms::PlatformType;
use crate::RoomInfo;
use crate::UserInfo;

#[derive(Clone, Debug, Default)]
pub struct StreamInfo {
    pub hls_url: String,
}

impl StreamInfo {
    // https://hs.hls.huya.com/huyalive/156976698-156976698-674209784144068608-314076852-10057-A-0-1.m3u8?ratio=200
    // 156976698-156976698-674209784144068608-314076852-10057-A-0-1
    pub fn id(&self) -> String {
        let url = Url::parse(&self.hls_url).unwrap();
        let path = url.path();
        let segments = path.split('/').collect::<Vec<&str>>();
        let filename = segments[segments.len() - 1];
        // 去掉 .m3u8 后缀
        filename
            .strip_suffix(".m3u8")
            .unwrap_or(filename)
            .to_string()
    }
}

pub struct LiveStreamExtractor;

impl LiveStreamExtractor {
    /// 从 JavaScript 代码中提取指定的变量
    pub fn extract_infos(
        js_content: &str,
    ) -> Result<(UserInfo, RoomInfo, StreamInfo), super::errors::HuyaClientError> {
        let global_init = Self::extract_variable(js_content, "window.HNF_GLOBAL_INIT")?;
        let room_info_obj = global_init
            .get("roomInfo")
            .and_then(|v| v.as_object())
            .unwrap();
        // roomInfo.tProfileInfo.lUid
        let uid = room_info_obj
            .get("tProfileInfo")
            .and_then(|v| v.as_object())
            .unwrap()
            .get("lUid")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            .to_string();
        // roomInfo.tProfileInfo.sNick
        let nick = room_info_obj
            .get("tProfileInfo")
            .and_then(|v| v.as_object())
            .unwrap()
            .get("sNick")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        // roomInfo.tProfileInfo.sAvatar180
        let avatar = room_info_obj
            .get("tProfileInfo")
            .and_then(|v| v.as_object())
            .unwrap()
            .get("sAvatar180")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // roomInfo.tLiveInfo.sScreenshot
        let cover = room_info_obj
            .get("tLiveInfo")
            .and_then(|v| v.as_object())
            .unwrap()
            .get("sScreenshot")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // roomInfo.tLiveInfo.sIntroduction
        let title = room_info_obj
            .get("tLiveInfo")
            .and_then(|v| v.as_object())
            .unwrap()
            .get("sIntroduction")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // roomInfo.tLiveInfo.lProfileRoom
        let room_id = room_info_obj
            .get("tLiveInfo")
            .and_then(|v| v.as_object())
            .unwrap()
            .get("lProfileRoom")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            .to_string();

        // roomInfo.eLiveStatus
        let status = room_info_obj
            .get("eLiveStatus")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        let user_info = UserInfo {
            user_id: uid,
            user_name: nick,
            user_avatar: avatar,
        };
        let room_info = RoomInfo {
            platform: PlatformType::Huya.as_str().to_string(),
            room_id,
            room_title: title,
            room_cover: cover,
            status: status == 2,
        };

        if !room_info.status {
            let stream_info = StreamInfo {
                hls_url: String::new(),
            };
            return Ok((user_info, room_info, stream_info));
        }

        // roomInfo.tLiveInfo.tLiveStreamInfo.vStreamInfo
        let live_stream_info = room_info_obj
            .get("tLiveInfo")
            .and_then(|v| v.as_object())
            .unwrap()
            .get("tLiveStreamInfo")
            .and_then(|v| v.as_object())
            .unwrap()
            .get("vStreamInfo")
            .and_then(|v| v.as_object())
            .unwrap()
            .get("value")
            .and_then(|v| v.as_array())
            .unwrap();

        // random one from vStreamInfo
        let stream_info = live_stream_info.choose(&mut rand::rng()).unwrap();

        let hls_url = stream_info
            .get("sHlsUrl")
            .and_then(|v| v.as_str())
            .unwrap()
            .to_string();
        let hls_anti_code = stream_info
            .get("sHlsAntiCode")
            .and_then(|v| v.as_str())
            .unwrap()
            .to_string();
        let presenter_uid = stream_info
            .get("lPresenterUid")
            .and_then(|v| v.as_i64())
            .unwrap()
            .to_string();
        let stream_name = stream_info
            .get("sStreamName")
            .and_then(|v| v.as_str())
            .unwrap()
            .to_string();

        let url = format!(
            "{}/{}.{}?{}",
            hls_url.replace("http://", "https://"),
            stream_name,
            "m3u8",
            hls_anti_code
        );

        let player_info = PlayerInfo {
            url,
            s_stream_name: Some(stream_name),
            presenter_uid: Some(presenter_uid),
            s_hls_anti_code: Some(hls_anti_code),
        };
        let result = UrlBuilder::build_player_url(&player_info).unwrap();
        let stream_info = StreamInfo { hls_url: result };
        Ok((user_info, room_info, stream_info))
    }
    /// 更健壮的提取方法，处理嵌套的大括号
    fn extract_variable(
        js_content: &str,
        var_name: &str,
    ) -> Result<Value, super::errors::HuyaClientError> {
        let var_pattern = format!(r"{}\s*=\s*", regex::escape(var_name));
        let re = Regex::new(&var_pattern)
            .map_err(|e| super::errors::HuyaClientError::ExtractorError(e.to_string()))?;

        if let Some(mat) = re.find(js_content) {
            let start_pos = mat.end();
            let remaining = &js_content[start_pos..];

            // 找到 JSON 对象的结束位置
            if let Some(json_end) = Self::find_json_end(remaining) {
                let js_obj_str = &remaining[..json_end];

                // 将 JavaScript 对象转换为 JSON
                let json_str = Self::js_to_json(js_obj_str)?;

                let value: Value = serde_json::from_str(&json_str)
                    .map_err(|e| super::errors::HuyaClientError::ExtractorError(e.to_string()))?;
                Ok(value)
            } else {
                Err(super::errors::HuyaClientError::ExtractorError(format!(
                    "Could not find end of JSON for variable {}",
                    var_name
                )))
            }
        } else {
            Err(super::errors::HuyaClientError::ExtractorError(format!(
                "Variable {} not found",
                var_name
            )))
        }
    }

    /// 找到 JSON 对象的结束位置（处理嵌套大括号）
    fn find_json_end(s: &str) -> Option<usize> {
        let mut brace_count = 0;
        let mut in_string = false;
        let mut escape_next = false;
        let mut start_found = false;

        for (i, ch) in s.char_indices() {
            if escape_next {
                escape_next = false;
                continue;
            }

            match ch {
                '\\' if in_string => escape_next = true,
                '"' => in_string = !in_string,
                '{' if !in_string => {
                    brace_count += 1;
                    start_found = true;
                }
                '}' if !in_string => {
                    brace_count -= 1;
                    if start_found && brace_count == 0 {
                        return Some(i + 1);
                    }
                }
                ';' if !in_string && brace_count == 0 && start_found => {
                    return Some(i);
                }
                _ => {}
            }
        }

        None
    }

    /// 将 JavaScript 对象转换为 JSON 字符串
    fn js_to_json(js_obj: &str) -> Result<String, super::errors::HuyaClientError> {
        let mut result = String::new();
        let mut chars = js_obj.chars().peekable();
        let mut in_string = false;
        let mut string_char = '"';
        let mut escape_next = false;
        let mut in_key = true; // 跟踪是否在键的位置
        let mut _brace_count = 0;
        let mut _bracket_count = 0;

        while let Some(ch) = chars.next() {
            if escape_next {
                result.push(ch);
                escape_next = false;
                continue;
            }

            match ch {
                '\\' if in_string => {
                    result.push(ch);
                    escape_next = true;
                }
                '\'' | '"' if !in_string => {
                    in_string = true;
                    string_char = ch;
                    result.push('"'); // 统一使用双引号
                }
                ch if in_string && ch == string_char => {
                    in_string = false;
                    result.push('"'); // 统一使用双引号
                }
                ':' if !in_string && in_key => {
                    result.push(':');
                    in_key = false;
                }
                ',' if !in_string && !in_key => {
                    result.push(',');
                    in_key = true;
                }
                '{' if !in_string => {
                    _brace_count += 1;
                    result.push(ch);
                }
                '}' if !in_string => {
                    _brace_count -= 1;
                    result.push(ch);
                }
                '[' if !in_string => {
                    _bracket_count += 1;
                    result.push(ch);
                }
                ']' if !in_string => {
                    _bracket_count -= 1;
                    result.push(ch);
                }
                ch if in_string => {
                    result.push(ch);
                }
                ch if !in_string && in_key && ch.is_alphanumeric() || ch == '_' => {
                    // 这是键名，需要加引号
                    result.push('"');
                    result.push(ch);
                    // 继续读取完整的键名
                    while let Some(&next_ch) = chars.peek() {
                        if next_ch.is_alphanumeric() || next_ch == '_' {
                            result.push(chars.next().unwrap());
                        } else {
                            break;
                        }
                    }
                    result.push('"');
                }
                ch if !in_string
                    && !in_key
                    && (ch.is_whitespace() || ch == '\n' || ch == '\r' || ch == '\t') =>
                {
                    // 跳过空白字符
                }
                ch => {
                    result.push(ch);
                }
            }
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_id() {
        let stream_info = StreamInfo { hls_url: "https://hs.hls.huya.com/huyalive/156976698-156976698-674209784144068608-314076852-10057-A-0-1.m3u8?ratio=200".to_string() };
        assert_eq!(
            stream_info.id(),
            "156976698-156976698-674209784144068608-314076852-10057-A-0-1"
        );
    }

    #[test]
    fn test_extract_variables() {
        // set log level to debug
        std::env::set_var("RUST_LOG", "debug");
        let _ = env_logger::try_init();
        let js_content = r#"
            window.HNF_GLOBAL_INIT = {
                "roomProfile": {
                    "tCacheInfo": {
                        "iSourceType": 1,
                        "iUpdateTime": 1761144871,
                        "_classname": "LiveRoom.CacheInfo"
                    },
                    "lUid": 156976698,
                    "iIsProfile": 1,
                    "iIsFreeze": 0,
                    "iIsMatchRoom": 0,
                    "iFreezeLevel": 0,
                    "_classname": "LiveRoom.GetLiveProfileResp",
                    "liveLineUrl": "Ly9ocy5obHMuaHV5YS5jb20vaHV5YWxpdmUvMTU2OTc2Njk4LTE1Njk3NjY5OC02NzQyMDk3ODQxNDQwNjg2MDgtMzE0MDc2ODUyLTEwMDU3LUEtMC0xLm0zdTg\u002FcmF0aW89MjAwMCZ3c1NlY3JldD03YWJjN2RlYzg4MDkxNDZmMzFmOTIwNDZlYjA0NGUzYiZ3c1RpbWU9NjhmYTQxYmEmZm09UkZkeE9FSmpTak5vTmtSS2REWlVXVjhrTUY4a01WOGtNbDhrTXclM0QlM0QmY3R5cGU9dGFyc19tb2JpbGUmZnM9YmdjdCZ0PTEwMw==",
                    "isFace": false
                },
                "roomInfo": {
                    "tCacheInfo": {
                        "iSourceType": 2,
                        "iUpdateTime": 1761144860,
                        "_classname": "LiveRoom.CacheInfo"
                    },
                    "eLiveStatus": 2,
                    "tProfileInfo": {
                        "lUid": 156976698,
                        "lYyid": 144136666,
                        "sNick": "三导-赵迪克【熬鹰小队主鹰】",
                        "iSex": 1,
                        "iLevel": 32,
                        "sAvatar180": "https:\u002F\u002Fhuyaimg.msstatic.com\u002Favatar\u002F1003\u002F23\u002F3be5ff7cff0f6d08fee796ac537ef0_180_135.jpg?1525686175",
                        "lProfileRoom": 857824,
                        "sPrivateHost": "hycoc",
                        "lActivityId": 0,
                        "lActivityCount": 164515,
                        "_classname": "LiveRoom.LiveProfileInfo"
                    },
                    "tLiveInfo": {
                        "lUid": 156976698,
                        "lYyid": 144136666,
                        "sNick": "三导-赵迪克【熬鹰小队主鹰】",
                        "iSex": 1,
                        "iLevel": 32,
                        "sAvatar180": "https:\u002F\u002Fhuyaimg.msstatic.com\u002Favatar\u002F1003\u002F23\u002F3be5ff7cff0f6d08fee796ac537ef0_180_135.jpg?1525686175",
                        "lProfileRoom": 857824,
                        "sPrivateHost": "hycoc",
                        "sProfileHomeHost": "hycoc",
                        "iIsPlatinum": 0,
                        "lActivityId": 0,
                        "lActivityCount": 164515,
                        "iGid": 8,
                        "iGameId": 0,
                        "sGameFullName": "魔兽世界",
                        "sGameHostName": "wow",
                        "iBussType": 1,
                        "lLiveId": 7563988042591917000,
                        "lChannel": 156976698,
                        "lLiveChannel": 156976698,
                        "lUserCount": 193160,
                        "lTotalCount": 193160,
                        "iStartTime": 1761128205,
                        "iEndTime": 0,
                        "iTime": 277,
                        "sRoomName": "",
                        "sIntroduction": "NAXX很难吗？",
                        "sPreviewUrl": "",
                        "iLiveSourceType": 0,
                        "iScreenType": 1,
                        "sScreenshot": "https:\u002F\u002Fanchorpost.msstatic.com\u002Fcdnimage\u002Fanchorpost\u002F1003\u002F23\u002F3be5ff7cff0f6d08fee796ac537ef0_2_8_1675412871.jpg?spformat=png,webp",
                        "iRecommendStatus": 0,
                        "sRecommendTagName": "",
                        "iIsSecret": 0,
                        "iCameraOpen": 0,
                        "iCodecType": 0,
                        "iIsBluRay": 1,
                        "sBluRayMBitRate": "20M",
                        "iBitRate": 20000,
                        "lLiveCompatibleFlag": 0,
                        "iUpdateCacheTime": 1761144890,
                        "lMultiStreamFlag": 66057,
                        "tLiveStreamInfo": {
                            "vStreamInfo": {
                                "_bValue": 0,
                                "value": [{
                                    "sCdnType": "AL",
                                    "iIsMaster": 0,
                                    "lChannelId": 156976698,
                                    "lSubChannelId": 156976698,
                                    "lPresenterUid": 156976698,
                                    "sStreamName": "156976698-156976698-674209784144068608-314076852-10057-A-0-1",
                                    "sFlvUrl": "http:\u002F\u002Fal.flv.huya.com\u002Fsrc",
                                    "sFlvUrlSuffix": "flv",
                                    "sFlvAntiCode": "wsSecret=7abc7dec8809146f31f92046eb044e3b&wsTime=68fa41ba&fm=RFdxOEJjSjNoNkRKdDZUWV8kMF8kMV8kMl8kMw%3D%3D&ctype=tars_mobile&fs=bgct&t=103",
                                    "sHlsUrl": "http:\u002F\u002Fal.hls.huya.com\u002Fsrc",
                                    "sHlsUrlSuffix": "m3u8",
                                    "sHlsAntiCode": "wsSecret=7abc7dec8809146f31f92046eb044e3b&wsTime=68fa41ba&fm=RFdxOEJjSjNoNkRKdDZUWV8kMF8kMV8kMl8kMw%3D%3D&ctype=tars_mobile&fs=bgct&t=103",
                                    "iLineIndex": 3,
                                    "iIsMultiStream": 1,
                                    "iPCPriorityRate": -1,
                                    "iWebPriorityRate": -1,
                                    "iMobilePriorityRate": -1,
                                    "vFlvIPList": {
                                        "_proto": {
                                            "_classname": "string"
                                        },
                                        "_bValue": 0,
                                        "value": [],
                                        "_classname": "list\u003Cstring\u003E"
                                    },
                                    "iIsP2PSupport": 2,
                                    "sP2pUrl": "http:\u002F\u002Fal.p2p.huya.com\u002Fhuyalive",
                                    "sP2pUrlSuffix": "slice",
                                    "sP2pAntiCode": "wsSecret=7abc7dec8809146f31f92046eb044e3b&wsTime=68fa41ba&fm=RFdxOEJjSjNoNkRKdDZUWV8kMF8kMV8kMl8kMw%3D%3D&ctype=tars_mobile&fs=bgct&t=103",
                                    "lFreeFlag": 2,
                                    "iIsHEVCSupport": 0,
                                    "vP2pIPList": {
                                        "_proto": {
                                            "_classname": "string"
                                        },
                                        "_bValue": 0,
                                        "value": [],
                                        "_classname": "list\u003Cstring\u003E"
                                    },
                                    "_classname": "HUYA.StreamInfo"
                                }, {
                                    "sCdnType": "TX",
                                    "iIsMaster": 0,
                                    "lChannelId": 156976698,
                                    "lSubChannelId": 156976698,
                                    "lPresenterUid": 156976698,
                                    "sStreamName": "156976698-156976698-674209784144068608-314076852-10057-A-0-1",
                                    "sFlvUrl": "http:\u002F\u002Ftx.flv.huya.com\u002Fsrc",
                                    "sFlvUrlSuffix": "flv",
                                    "sFlvAntiCode": "wsSecret=7abc7dec8809146f31f92046eb044e3b&wsTime=68fa41ba&fm=RFdxOEJjSjNoNkRKdDZUWV8kMF8kMV8kMl8kMw%3D%3D&ctype=tars_mobile&fs=bgct&t=103",
                                    "sHlsUrl": "http:\u002F\u002Ftx.hls.huya.com\u002Fsrc",
                                    "sHlsUrlSuffix": "m3u8",
                                    "sHlsAntiCode": "wsSecret=7abc7dec8809146f31f92046eb044e3b&wsTime=68fa41ba&fm=RFdxOEJjSjNoNkRKdDZUWV8kMF8kMV8kMl8kMw%3D%3D&ctype=tars_mobile&fs=bgct&t=103",
                                    "iLineIndex": 5,
                                    "iIsMultiStream": 1,
                                    "iPCPriorityRate": 0,
                                    "iWebPriorityRate": 0,
                                    "iMobilePriorityRate": 0,
                                    "vFlvIPList": {
                                        "_proto": {
                                            "_classname": "string"
                                        },
                                        "_bValue": 0,
                                        "value": [],
                                        "_classname": "list\u003Cstring\u003E"
                                    },
                                    "iIsP2PSupport": 2,
                                    "sP2pUrl": "http:\u002F\u002Ftx.p2p.huya.com\u002Fhuyalive",
                                    "sP2pUrlSuffix": "slice",
                                    "sP2pAntiCode": "wsSecret=7abc7dec8809146f31f92046eb044e3b&wsTime=68fa41ba&fm=RFdxOEJjSjNoNkRKdDZUWV8kMF8kMV8kMl8kMw%3D%3D&ctype=tars_mobile&fs=bgct&t=103",
                                    "lFreeFlag": 1,
                                    "iIsHEVCSupport": 0,
                                    "vP2pIPList": {
                                        "_proto": {
                                            "_classname": "string"
                                        },
                                        "_bValue": 0,
                                        "value": [],
                                        "_classname": "list\u003Cstring\u003E"
                                    },
                                    "_classname": "HUYA.StreamInfo"
                                }, {
                                    "sCdnType": "HS",
                                    "iIsMaster": 0,
                                    "lChannelId": 156976698,
                                    "lSubChannelId": 156976698,
                                    "lPresenterUid": 156976698,
                                    "sStreamName": "156976698-156976698-674209784144068608-314076852-10057-A-0-1",
                                    "sFlvUrl": "http:\u002F\u002Fhs.flv.huya.com\u002Fhuyalive",
                                    "sFlvUrlSuffix": "flv",
                                    "sFlvAntiCode": "wsSecret=7abc7dec8809146f31f92046eb044e3b&wsTime=68fa41ba&fm=RFdxOEJjSjNoNkRKdDZUWV8kMF8kMV8kMl8kMw%3D%3D&ctype=tars_mobile&fs=bgct&t=103",
                                    "sHlsUrl": "http:\u002F\u002Fhs.hls.huya.com\u002Fhuyalive",
                                    "sHlsUrlSuffix": "m3u8",
                                    "sHlsAntiCode": "wsSecret=7abc7dec8809146f31f92046eb044e3b&wsTime=68fa41ba&fm=RFdxOEJjSjNoNkRKdDZUWV8kMF8kMV8kMl8kMw%3D%3D&ctype=tars_mobile&fs=bgct&t=103",
                                    "iLineIndex": 14,
                                    "iIsMultiStream": 1,
                                    "iPCPriorityRate": 100,
                                    "iWebPriorityRate": 100,
                                    "iMobilePriorityRate": 100,
                                    "vFlvIPList": {
                                        "_proto": {
                                            "_classname": "string"
                                        },
                                        "_bValue": 0,
                                        "value": [],
                                        "_classname": "list\u003Cstring\u003E"
                                    },
                                    "iIsP2PSupport": 2,
                                    "sP2pUrl": "http:\u002F\u002Fhs.p2p.huya.com\u002Fhuyalive",
                                    "sP2pUrlSuffix": "slice",
                                    "sP2pAntiCode": "wsSecret=7abc7dec8809146f31f92046eb044e3b&wsTime=68fa41ba&fm=RFdxOEJjSjNoNkRKdDZUWV8kMF8kMV8kMl8kMw%3D%3D&ctype=tars_mobile&fs=bgct&t=103",
                                    "lFreeFlag": 0,
                                    "iIsHEVCSupport": 0,
                                    "vP2pIPList": {
                                        "_proto": {
                                            "_classname": "string"
                                        },
                                        "_bValue": 0,
                                        "value": [],
                                        "_classname": "list\u003Cstring\u003E"
                                    },
                                    "_classname": "HUYA.StreamInfo"
                                }],
                                "_classname": "list\u003CHUYA.StreamInfo\u003E"
                            },
                            "mStreamRatio": {
                                "_kproto": {
                                    "_classname": "string"
                                },
                                "_vproto": {
                                    "_classname": "int32"
                                },
                                "_bKey": 0,
                                "_bValue": 0,
                                "value": {
                                    "TX": 0,
                                    "AL": -1,
                                    "HS": 100
                                },
                                "_classname": "map\u003Cstring,int32\u003E"
                            },
                            "vBitRateInfo": {
                                "_bValue": 0,
                                "value": [{
                                    "sDisplayName": "蓝光20M",
                                    "iBitRate": 0,
                                    "iCodecType": 0,
                                    "iCompatibleFlag": 0,
                                    "iHEVCBitRate": -1,
                                    "_classname": "LiveRoom.LiveBitRateInfo"
                                }, {
                                    "sDisplayName": "蓝光8M",
                                    "iBitRate": 8000,
                                    "iCodecType": 0,
                                    "iCompatibleFlag": 0,
                                    "iHEVCBitRate": 8000,
                                    "_classname": "LiveRoom.LiveBitRateInfo"
                                }, {
                                    "sDisplayName": "蓝光4M",
                                    "iBitRate": 4000,
                                    "iCodecType": 0,
                                    "iCompatibleFlag": 0,
                                    "iHEVCBitRate": 4000,
                                    "_classname": "LiveRoom.LiveBitRateInfo"
                                }, {
                                    "sDisplayName": "超清",
                                    "iBitRate": 2000,
                                    "iCodecType": 0,
                                    "iCompatibleFlag": 0,
                                    "iHEVCBitRate": 2000,
                                    "_classname": "LiveRoom.LiveBitRateInfo"
                                }, {
                                    "sDisplayName": "流畅",
                                    "iBitRate": 500,
                                    "iCodecType": 0,
                                    "iCompatibleFlag": 0,
                                    "iHEVCBitRate": 500,
                                    "_classname": "LiveRoom.LiveBitRateInfo"
                                }],
                                "_classname": "list\u003CLiveRoom.LiveBitRateInfo\u003E"
                            },
                            "iDefaultLiveStreamBitRate": 2000,
                            "sDefaultLiveStreamLine": "HS",
                            "sDefaultLiveStreamSuffix": "m3u8",
                            "sDefaultLiveStreamUrl": "http:\u002F\u002Fhs.hls.huya.com\u002Fhuyalive\u002F156976698-156976698-674209784144068608-314076852-10057-A-0-1.m3u8?ratio=2000&wsSecret=7abc7dec8809146f31f92046eb044e3b&wsTime=68fa41ba&fm=RFdxOEJjSjNoNkRKdDZUWV8kMF8kMV8kMl8kMw%3D%3D&ctype=tars_mobile&fs=bgct&t=103",
                            "_classname": "LiveRoom.LiveStreamInfo"
                        },
                        "iIsRoomPay": 0,
                        "iIsWatchTogetherVip": 0,
                        "tRoomPayInfo": {
                            "lUid": 0,
                            "iIsRoomPay": 0,
                            "iIsShowTag": 0,
                            "sRoomPayTag": "",
                            "sRoomPayPassword": "",
                            "_classname": "LiveRoom.RoomPayInfo"
                        },
                        "_classname": "LiveRoom.LiveInfo"
                    },
                    "tRecentLive": {
                        "lUid": 0,
                        "lYyid": 0,
                        "sNick": "",
                        "iSex": 0,
                        "iLevel": 0,
                        "sAvatar180": "",
                        "lProfileRoom": 0,
                        "sPrivateHost": "",
                        "sProfileHomeHost": "",
                        "iIsPlatinum": 0,
                        "lActivityId": 0,
                        "lActivityCount": 0,
                        "iGid": 0,
                        "iGameId": 0,
                        "sGameFullName": "",
                        "sGameHostName": "",
                        "iBussType": 0,
                        "lLiveId": 0,
                        "lChannel": 0,
                        "lLiveChannel": 0,
                        "lUserCount": 0,
                        "lTotalCount": 0,
                        "iStartTime": 0,
                        "iEndTime": 0,
                        "iTime": 0,
                        "sRoomName": "",
                        "sIntroduction": "",
                        "sPreviewUrl": "",
                        "iLiveSourceType": 0,
                        "iScreenType": 0,
                        "sScreenshot": "",
                        "iRecommendStatus": 0,
                        "sRecommendTagName": "",
                        "iIsSecret": 0,
                        "iCameraOpen": 0,
                        "iCodecType": 0,
                        "iIsBluRay": 0,
                        "sBluRayMBitRate": "",
                        "iBitRate": 0,
                        "lLiveCompatibleFlag": 0,
                        "iUpdateCacheTime": 0,
                        "lMultiStreamFlag": 0,
                        "_classname": "LiveRoom.RecentLiveInfo"
                    },
                    "tReplayInfo": {
                        "lUid": 0,
                        "lYyid": 0,
                        "sNick": "",
                        "iSex": 0,
                        "iLevel": 0,
                        "sAvatar180": "",
                        "lProfileRoom": 0,
                        "sPrivateHost": "",
                        "iIsPlatinum": 0,
                        "lActivityId": 0,
                        "lActivityCount": 0,
                        "iGid": 0,
                        "iGameId": 0,
                        "sGameFullName": "",
                        "sGameHostName": "",
                        "iBussType": 0,
                        "lLiveId": 0,
                        "lChannel": 0,
                        "lLiveChannel": 0,
                        "lUserCount": 0,
                        "lTotalCount": 0,
                        "iStartTime": 0,
                        "iEndTime": 0,
                        "iTime": 0,
                        "sRoomName": "",
                        "sIntroduction": "",
                        "sPreviewUrl": "",
                        "iLiveSourceType": 0,
                        "iScreenType": 0,
                        "sScreenshot": "",
                        "iIsSecret": 0,
                        "iCameraOpen": 0,
                        "iCodecType": 0,
                        "iIsBluRay": 0,
                        "sBluRayMBitRate": "",
                        "iBitRate": 0,
                        "lLiveCompatibleFlag": 0,
                        "lMultiStreamFlag": 0,
                        "tReplayVideoInfo": {
                            "sUrl": "",
                            "sHlsUrl": "",
                            "iVideoSyncTime": 0,
                            "_classname": "LiveRoom.ReplayVideoInfo"
                        },
                        "_classname": "LiveRoom.ReplayInfo"
                    },
                    "_classname": "LiveRoom.GetLiveRoomResp"
                },
                "roomRecommendLiveList": [{
                    "iIdx": 1,
                    "sTagMark": "gameRecommendLives",
                    "sTagName": "相关推荐",
                    "vList": {
                        "_bValue": 0,
                        "value": [{
                            "lUid": 1423805942,
                            "lYyid": 1584989003,
                            "sNick": "暴雪游戏频道",
                            "iSex": 2,
                            "iLevel": 18,
                            "sAvatar180": "http:\u002F\u002Fhuyaimg.msstatic.com\u002Favatar\u002F1091\u002F8f\u002Ff72631f796b336a21670335b67d785_180_135.jpg?1472726154",
                            "lProfileRoom": 660104,
                            "sPrivateHost": "blizzardgame1",
                            "sProfileHomeHost": "blizzardgame1",
                            "iIsPlatinum": 1,
                            "lActivityId": 0,
                            "lActivityCount": 938376,
                            "iGid": 8,
                            "iGameId": 820,
                            "sGameFullName": "魔兽世界",
                            "sGameHostName": "wow",
                            "iBussType": 1,
                            "lLiveId": 7563993133377692000,
                            "lChannel": 1423805942,
                            "lLiveChannel": 1423805942,
                            "lUserCount": 55549,
                            "lTotalCount": 55549,
                            "sRoomName": "【重播】炉石传说黄金公开赛西安站",
                            "sIntroduction": "【重播】网易大神魔兽世界娱乐竞速赛",
                            "sPreviewUrl": "",
                            "iLiveSourceType": 12,
                            "iScreenType": 0,
                            "sScreenshot": "https:\u002F\u002Fanchorpost.msstatic.com\u002Fcdnimage\u002Fanchorpost\u002F1091\u002F8f\u002Ff72631f796b336a21670335b67d785_8_1761131325.jpg?spformat=png,webp",
                            "iIsSecret": 0,
                            "iCameraOpen": 0,
                            "iIsBluRay": 1,
                            "sBluRayMBitRate": "8M",
                            "iBitRate": 8000,
                            "lLiveCompatibleFlag": 16,
                            "iRecommendStatus": 6041,
                            "sRecommendTagName": "虎牙官方",
                            "iIsRoomPay": 0,
                            "sRoomPayTag": "",
                            "iIsWatchTogetherVip": 0,
                            "iStartTime": 1761129390,
                            "iTime": 15478,
                            "iUpdateCacheTime": 1761144868,
                            "mpCorner": {
                                "_kproto": {
                                    "_classname": "string"
                                },
                                "_bKey": 0,
                                "_bValue": 0,
                                "value": {
                                    "ListPos1": {
                                        "sContent": "有更新",
                                        "sIcon": "",
                                        "_classname": "LiveList.CornerInfo"
                                    }
                                },
                                "_classname": "map\u003Cstring,LiveList.CornerInfo\u003E"
                            },
                            "tImgRecInfo": {
                                "sType": "",
                                "sValue": "",
                                "sTypeDesc": "",
                                "_classname": "LiveList.ImgRecInfo"
                            },
                            "_classname": "LiveList.LiveListInfo"
                        }, {
                            "lUid": 1199518259112,
                            "lYyid": 35184390053227,
                            "sNick": "AH丶小团宝",
                            "iSex": 2,
                            "iLevel": 31,
                            "sAvatar180": "https:\u002F\u002Fhuyaimg.msstatic.com\u002Favatar\u002F1083\u002F33\u002F2799fbb085a3546991f952a13def44_180_135.jpg?1758247511",
                            "lProfileRoom": 137629,
                            "sPrivateHost": "35184390053227",
                            "sProfileHomeHost": "35184390053227",
                            "iIsPlatinum": 1,
                            "lActivityId": 0,
                            "lActivityCount": 120564,
                            "iGid": 8,
                            "iGameId": 0,
                            "sGameFullName": "魔兽世界",
                            "sGameHostName": "wow",
                            "iBussType": 1,
                            "lLiveId": 7564021497140413000,
                            "lChannel": 1199518259112,
                            "lLiveChannel": 1199518259112,
                            "lUserCount": 264917,
                            "lTotalCount": 264917,
                            "sRoomName": "",
                            "sIntroduction": "臭游戏+~~",
                            "sPreviewUrl": "",
                            "iLiveSourceType": 0,
                            "iScreenType": 1,
                            "sScreenshot": "https:\u002F\u002Fanchorpost.msstatic.com\u002Fcdnimage\u002Fanchorpost\u002F1083\u002F33\u002F2799fbb085a3546991f952a13def44_8_1755265051.jpg?spformat=png,webp",
                            "iIsSecret": 0,
                            "iCameraOpen": 0,
                            "iIsBluRay": 1,
                            "sBluRayMBitRate": "8M",
                            "iBitRate": 8000,
                            "lLiveCompatibleFlag": 0,
                            "iRecommendStatus": 0,
                            "sRecommendTagName": "",
                            "iIsRoomPay": 0,
                            "sRoomPayTag": "",
                            "iIsWatchTogetherVip": 0,
                            "iStartTime": 1761135994,
                            "iTime": 8874,
                            "iUpdateCacheTime": 1761144868,
                            "mpCorner": {
                                "_kproto": {
                                    "_classname": "string"
                                },
                                "_bKey": 0,
                                "_bValue": 0,
                                "value": {
                                    "ListPos2": {
                                        "sContent": "摸了个鱼",
                                        "sIcon": "",
                                        "_classname": "LiveList.CornerInfo"
                                    }
                                },
                                "_classname": "map\u003Cstring,LiveList.CornerInfo\u003E"
                            },
                            "tImgRecInfo": {
                                "sType": "",
                                "sValue": "",
                                "sTypeDesc": "",
                                "_classname": "LiveList.ImgRecInfo"
                            },
                            "_classname": "LiveList.LiveListInfo"
                        }, {
                            "lUid": 8313500,
                            "lYyid": 138713814,
                            "sNick": "爱玩WOW的茜宝",
                            "iSex": 2,
                            "iLevel": 35,
                            "sAvatar180": "https:\u002F\u002Fhuyaimg.msstatic.com\u002Favatar\u002F1089\u002Fa0\u002F9d09de8d812e415fa53460063c4c02_180_135.jpg?1754813630",
                            "lProfileRoom": 124395,
                            "sPrivateHost": "lovexier",
                            "sProfileHomeHost": "lovexier",
                            "iIsPlatinum": 1,
                            "lActivityId": 0,
                            "lActivityCount": 57555,
                            "iGid": 8,
                            "iGameId": 0,
                            "sGameFullName": "魔兽世界",
                            "sGameHostName": "wow",
                            "iBussType": 1,
                            "lLiveId": 7563993857824377000,
                            "lChannel": 8313500,
                            "lLiveChannel": 8313500,
                            "lUserCount": 210202,
                            "lTotalCount": 210202,
                            "sRoomName": "",
                            "sIntroduction": "老天奶~给我出个凤凰吧叭~",
                            "sPreviewUrl": "",
                            "iLiveSourceType": 0,
                            "iScreenType": 1,
                            "sScreenshot": "https:\u002F\u002Fanchorpost.msstatic.com\u002Fcdnimage\u002Fanchorpost\u002F1089\u002Fa0\u002F9d09de8d812e415fa53460063c4c02_2_8_1662360246.jpg?spformat=png,webp",
                            "iIsSecret": 0,
                            "iCameraOpen": 0,
                            "iIsBluRay": 1,
                            "sBluRayMBitRate": "8M",
                            "iBitRate": 8000,
                            "lLiveCompatibleFlag": 0,
                            "iRecommendStatus": 496,
                            "sRecommendTagName": "魅力新星",
                            "iIsRoomPay": 0,
                            "sRoomPayTag": "",
                            "iIsWatchTogetherVip": 0,
                            "iStartTime": 1761129559,
                            "iTime": 15309,
                            "iUpdateCacheTime": 1761144868,
                            "mpCorner": {
                                "_kproto": {
                                    "_classname": "string"
                                },
                                "_bKey": 0,
                                "_bValue": 0,
                                "value": {
                                    "ListPos1": {
                                        "sContent": "魅力新星",
                                        "sIcon": "",
                                        "_classname": "LiveList.CornerInfo"
                                    }
                                },
                                "_classname": "map\u003Cstring,LiveList.CornerInfo\u003E"
                            },
                            "tImgRecInfo": {
                                "sType": "",
                                "sValue": "",
                                "sTypeDesc": "",
                                "_classname": "LiveList.ImgRecInfo"
                            },
                            "_classname": "LiveList.LiveListInfo"
                        }, {
                            "lUid": 17981059,
                            "lYyid": 10841615,
                            "sNick": "牛会长",
                            "iSex": 1,
                            "iLevel": 29,
                            "sAvatar180": "https:\u002F\u002Fhuyaimg.msstatic.com\u002Favatar\u002F1056\u002F57\u002F5a06880da605e4a2d4811266a26727_180_135.jpg?1708064520",
                            "lProfileRoom": 252450,
                            "sPrivateHost": "shangdi",
                            "sProfileHomeHost": "shangdi",
                            "iIsPlatinum": 1,
                            "lActivityId": 0,
                            "lActivityCount": 135893,
                            "iGid": 8,
                            "iGameId": 0,
                            "sGameFullName": "魔兽世界",
                            "sGameHostName": "wow",
                            "iBussType": 1,
                            "lLiveId": 7563983640111566000,
                            "lChannel": 17981059,
                            "lLiveChannel": 17981059,
                            "lUserCount": 187251,
                            "lTotalCount": 187251,
                            "sRoomName": "",
                            "sIntroduction": "正式服-每天都有团",
                            "sPreviewUrl": "",
                            "iLiveSourceType": 0,
                            "iScreenType": 1,
                            "sScreenshot": "https:\u002F\u002Ftx-live-cover.msstatic.com\u002Fhuyalive\u002F17981059-17981059-77228060352446464-36085574-10057-A-0-1\u002F20251022225314.jpg?sign=bwqHPOzOut1aRtvZa0HU5sNrXtRhPTEyNTM0OTg3MDEmYj1odXlhLXNjcmVlbnNob3RzLXJldmlldy0xMjUzNDk4NzAxJms9QUtJRFFpcTNSbEJtV0p6ZUxKTVZrMklWdVEybm1pY2RkRWdEJmU9MTc3NjY5Njc5NSZ0PTE3NjExNDQ3OTUmcj0xMjM0NTY3OCZmPS9odXlhbGl2ZS8xNzk4MTA1OS0xNzk4MTA1OS03NzIyODA2MDM1MjQ0NjQ2NC0zNjA4NTU3NC0xMDA1Ny1BLTAtMS8yMDI1MTAyMjIyNTMxNC5qcGc=",
                            "iIsSecret": 0,
                            "iCameraOpen": 0,
                            "iIsBluRay": 1,
                            "sBluRayMBitRate": "20M",
                            "iBitRate": 20000,
                            "lLiveCompatibleFlag": 0,
                            "iRecommendStatus": 0,
                            "sRecommendTagName": "",
                            "iIsRoomPay": 0,
                            "sRoomPayTag": "",
                            "iIsWatchTogetherVip": 0,
                            "iStartTime": 1761127180,
                            "iTime": 17688,
                            "iUpdateCacheTime": 1761144868,
                            "mpCorner": {
                                "_kproto": {
                                    "_classname": "string"
                                },
                                "_bKey": 0,
                                "_bValue": 0,
                                "value": {},
                                "_classname": "map\u003Cstring,LiveList.CornerInfo\u003E"
                            },
                            "tImgRecInfo": {
                                "sType": "",
                                "sValue": "",
                                "sTypeDesc": "",
                                "_classname": "LiveList.ImgRecInfo"
                            },
                            "_classname": "LiveList.LiveListInfo"
                        }],
                        "_classname": "list\u003CLiveList.LiveListInfo\u003E"
                    },
                    "_classname": "LiveList.RoomRecommendLiveTag"
                }, {
                    "iIdx": 2,
                    "sTagMark": "allRecommendLives",
                    "sTagName": "热游直播",
                    "vList": {
                        "_bValue": 0,
                        "value": [{
                            "lUid": 367138632,
                            "lYyid": 280114420,
                            "sNick": "卡尔",
                            "iSex": 1,
                            "iLevel": 44,
                            "sAvatar180": "http:\u002F\u002Fhuyaimg.msstatic.com\u002Favatar\u002F1084\u002Fb7\u002F896bc815db9560eabbcb4a227f62ba_180_135.jpg?1416387967",
                            "lProfileRoom": 521000,
                            "sPrivateHost": "kaerlol",
                            "sProfileHomeHost": "kaerlol",
                            "iIsPlatinum": 1,
                            "lActivityId": 0,
                            "lActivityCount": 11571972,
                            "iGid": 1,
                            "iGameId": 0,
                            "sGameFullName": "英雄联盟",
                            "sGameHostName": "lol",
                            "iBussType": 1,
                            "lLiveId": 7564029154208187000,
                            "lChannel": 367138632,
                            "lLiveChannel": 367138632,
                            "lUserCount": 8535291,
                            "lTotalCount": 8535291,
                            "sRoomName": "",
                            "sIntroduction": "【第六届百里挑一】56进54",
                            "sPreviewUrl": "",
                            "iLiveSourceType": 0,
                            "iScreenType": 1,
                            "sScreenshot": "http:\u002F\u002Flive-cover.msstatic.com\u002Fhuyalive\u002F367138632-367138632-1576848417538179072-734400720-10057-A-0-1-imgplus\u002F20251022225310.jpg",
                            "iIsSecret": 0,
                            "iCameraOpen": 0,
                            "iIsBluRay": 1,
                            "sBluRayMBitRate": "10M",
                            "iBitRate": 10000,
                            "lLiveCompatibleFlag": 0,
                            "iRecommendStatus": 5363,
                            "sRecommendTagName": "超级明星",
                            "iIsRoomPay": 0,
                            "sRoomPayTag": "",
                            "iIsWatchTogetherVip": 0,
                            "iStartTime": 1761137777,
                            "iTime": 7064,
                            "iUpdateCacheTime": 1761144841,
                            "mpCorner": {
                                "_kproto": {
                                    "_classname": "string"
                                },
                                "_bKey": 0,
                                "_bValue": 0,
                                "value": {
                                    "ListPos1": {
                                        "sContent": "",
                                        "sIcon": "",
                                        "_classname": "LiveList.CornerInfo"
                                    }
                                },
                                "_classname": "map\u003Cstring,LiveList.CornerInfo\u003E"
                            },
                            "tImgRecInfo": {
                                "sType": "",
                                "sValue": "",
                                "sTypeDesc": "",
                                "_classname": "LiveList.ImgRecInfo"
                            },
                            "_classname": "LiveList.LiveListInfo"
                        }, {
                            "lUid": 1354740567,
                            "lYyid": 1496831143,
                            "sNick": "华创-吕德华",
                            "iSex": 1,
                            "iLevel": 38,
                            "sAvatar180": "https:\u002F\u002Fhuyaimg.msstatic.com\u002Favatar\u002F1034\u002F77\u002Fa4286776aa02881faa959bbb2a94d5_180_135.jpg?1598180690",
                            "lProfileRoom": 243547,
                            "sPrivateHost": "1496831143",
                            "sProfileHomeHost": "1496831143",
                            "iIsPlatinum": 1,
                            "lActivityId": 0,
                            "lActivityCount": 9663585,
                            "iGid": 2336,
                            "iGameId": 0,
                            "sGameFullName": "王者荣耀",
                            "sGameHostName": "wzry",
                            "iBussType": 3,
                            "lLiveId": 7564000821295882000,
                            "lChannel": 1354740567,
                            "lLiveChannel": 1354740567,
                            "lUserCount": 6733192,
                            "lTotalCount": 6733192,
                            "sRoomName": "",
                            "sIntroduction": "已国服吕布 本月必拿双国服",
                            "sPreviewUrl": "",
                            "iLiveSourceType": 0,
                            "iScreenType": 1,
                            "sScreenshot": "https:\u002F\u002Fanchorpost.msstatic.com\u002Fcdnimage\u002Fanchorpost\u002F1034\u002F77\u002Fa4286776aa02881faa959bbb2a94d5_2336_1760698674.jpg?spformat=png,webp",
                            "iIsSecret": 0,
                            "iCameraOpen": 0,
                            "iIsBluRay": 1,
                            "sBluRayMBitRate": "12M",
                            "iBitRate": 12000,
                            "lLiveCompatibleFlag": 0,
                            "iRecommendStatus": 5363,
                            "sRecommendTagName": "超级明星",
                            "iIsRoomPay": 0,
                            "sRoomPayTag": "",
                            "iIsWatchTogetherVip": 0,
                            "iStartTime": 1761131180,
                            "iTime": 13661,
                            "iUpdateCacheTime": 1761144841,
                            "mpCorner": {
                                "_kproto": {
                                    "_classname": "string"
                                },
                                "_bKey": 0,
                                "_bValue": 0,
                                "value": {
                                    "ListPos1": {
                                        "sContent": "小时最佳",
                                        "sIcon": "https:\u002F\u002Fhuyaimg.msstatic.com\u002Fcdnimage\u002Fcornermark\u002FphpuTxfcK1723113210.png",
                                        "_classname": "LiveList.CornerInfo"
                                    }
                                },
                                "_classname": "map\u003Cstring,LiveList.CornerInfo\u003E"
                            },
                            "tImgRecInfo": {
                                "sType": "",
                                "sValue": "",
                                "sTypeDesc": "",
                                "_classname": "LiveList.ImgRecInfo"
                            },
                            "_classname": "LiveList.LiveListInfo"
                        }, {
                            "lUid": 294636272,
                            "lYyid": 229813522,
                            "sNick": "狂鸟丶楚河-90327",
                            "iSex": 1,
                            "iLevel": 48,
                            "sAvatar180": "https:\u002F\u002Fhuyaimg.msstatic.com\u002Favatar\u002F1086\u002Fbf\u002Ffd6f69d69c0015eaface1f6024869e_180_135.jpg?1619540458",
                            "lProfileRoom": 998,
                            "sPrivateHost": "chuhe",
                            "sProfileHomeHost": "chuhe",
                            "iIsPlatinum": 1,
                            "lActivityId": 0,
                            "lActivityCount": 9788131,
                            "iGid": 1964,
                            "iGameId": 0,
                            "sGameFullName": "主机游戏",
                            "sGameHostName": "ZJGAME",
                            "iBussType": 2,
                            "lLiveId": 7563966842753702000,
                            "lChannel": 294636272,
                            "lLiveChannel": 294636272,
                            "lUserCount": 5493761,
                            "lTotalCount": 5493761,
                            "sRoomName": "",
                            "sIntroduction": "新主播，第一天直播很紧张!",
                            "sPreviewUrl": "",
                            "iLiveSourceType": 0,
                            "iScreenType": 1,
                            "sScreenshot": "http:\u002F\u002Flive-cover.msstatic.com\u002Fhuyalive\u002F294636272-294636272-1265453152455360512-589396000-10057-A-0-1-imgplus\u002F20251022225322.jpg",
                            "iIsSecret": 0,
                            "iCameraOpen": 0,
                            "iIsBluRay": 1,
                            "sBluRayMBitRate": "30M",
                            "iBitRate": 30000,
                            "lLiveCompatibleFlag": 0,
                            "iRecommendStatus": 5363,
                            "sRecommendTagName": "超级明星",
                            "iIsRoomPay": 0,
                            "sRoomPayTag": "",
                            "iIsWatchTogetherVip": 0,
                            "iStartTime": 1761123269,
                            "iTime": 21572,
                            "iUpdateCacheTime": 1761144841,
                            "mpCorner": {
                                "_kproto": {
                                    "_classname": "string"
                                },
                                "_bKey": 0,
                                "_bValue": 0,
                                "value": {
                                    "ListPos1": {
                                        "sContent": "",
                                        "sIcon": "",
                                        "_classname": "LiveList.CornerInfo"
                                    }
                                },
                                "_classname": "map\u003Cstring,LiveList.CornerInfo\u003E"
                            },
                            "tImgRecInfo": {
                                "sType": "",
                                "sValue": "",
                                "sTypeDesc": "",
                                "_classname": "LiveList.ImgRecInfo"
                            },
                            "_classname": "LiveList.LiveListInfo"
                        }, {
                            "lUid": 2272316519,
                            "lYyid": 2272348727,
                            "sNick": "小小小酷哥-组织",
                            "iSex": 1,
                            "iLevel": 49,
                            "sAvatar180": "https:\u002F\u002Fhuyaimg.msstatic.com\u002Favatar\u002F1056\u002Fcc\u002F2ae247f33b3d03046c0b41eddf8ac9_180_135.jpg?1756555664",
                            "lProfileRoom": 1995,
                            "sPrivateHost": "2272348727",
                            "sProfileHomeHost": "2272348727",
                            "iIsPlatinum": 1,
                            "lActivityId": 0,
                            "lActivityCount": 3481422,
                            "iGid": 2165,
                            "iGameId": 0,
                            "sGameFullName": "户外",
                            "sGameHostName": "huwai",
                            "iBussType": 8,
                            "lLiveId": 7564028387263898000,
                            "lChannel": 2272316519,
                            "lLiveChannel": 2272316519,
                            "lUserCount": 4317414,
                            "lTotalCount": 4317414,
                            "sRoomName": "",
                            "sIntroduction": "三亚",
                            "sPreviewUrl": "",
                            "iLiveSourceType": 6,
                            "iScreenType": 1,
                            "sScreenshot": "http:\u002F\u002Flive-cover.msstatic.com\u002Fhuyalive\u002F2272316519-2272316519-9759525135265562624-4544756494-10057-A-0-1-imgplus\u002F20251022225314.jpg",
                            "iIsSecret": 0,
                            "iCameraOpen": 0,
                            "iIsBluRay": 0,
                            "sBluRayMBitRate": "",
                            "iBitRate": 3000,
                            "lLiveCompatibleFlag": 0,
                            "iRecommendStatus": 5363,
                            "sRecommendTagName": "超级明星",
                            "iIsRoomPay": 0,
                            "sRoomPayTag": "",
                            "iIsWatchTogetherVip": 0,
                            "iStartTime": 1761137599,
                            "iTime": 7242,
                            "iUpdateCacheTime": 1761144841,
                            "mpCorner": {
                                "_kproto": {
                                    "_classname": "string"
                                },
                                "_bKey": 0,
                                "_bValue": 0,
                                "value": {
                                    "ListPos1": {
                                        "sContent": "上电视",
                                        "sIcon": "https:\u002F\u002Fhuyaimg.msstatic.com\u002Fcdnimage\u002Fcornermark\u002FphpC2AIU11706668197.png",
                                        "_classname": "LiveList.CornerInfo"
                                    }
                                },
                                "_classname": "map\u003Cstring,LiveList.CornerInfo\u003E"
                            },
                            "tImgRecInfo": {
                                "sType": "",
                                "sValue": "",
                                "sTypeDesc": "",
                                "_classname": "LiveList.ImgRecInfo"
                            },
                            "_classname": "LiveList.LiveListInfo"
                        }],
                        "_classname": "list\u003CLiveList.LiveListInfo\u003E"
                    },
                    "_classname": "LiveList.RoomRecommendLiveTag"
                }, {
                    "iIdx": 3,
                    "sTagMark": "musicRecommendLives",
                    "sTagName": "综艺娱乐",
                    "vList": {
                        "_bValue": 0,
                        "value": [{
                            "lUid": 2272316519,
                            "lYyid": 2272348727,
                            "sNick": "小小小酷哥-组织",
                            "iSex": 1,
                            "iLevel": 49,
                            "sAvatar180": "https:\u002F\u002Fhuyaimg.msstatic.com\u002Favatar\u002F1056\u002Fcc\u002F2ae247f33b3d03046c0b41eddf8ac9_180_135.jpg?1756555664",
                            "lProfileRoom": 1995,
                            "sPrivateHost": "2272348727",
                            "sProfileHomeHost": "2272348727",
                            "iIsPlatinum": 1,
                            "lActivityId": 0,
                            "lActivityCount": 3481415,
                            "iGid": 2165,
                            "iGameId": 0,
                            "sGameFullName": "户外",
                            "sGameHostName": "huwai",
                            "iBussType": 8,
                            "lLiveId": 7564028387263898000,
                            "lChannel": 2272316519,
                            "lLiveChannel": 2272316519,
                            "lUserCount": 4317414,
                            "lTotalCount": 4317414,
                            "sRoomName": "",
                            "sIntroduction": "三亚",
                            "sPreviewUrl": "",
                            "iLiveSourceType": 6,
                            "iScreenType": 1,
                            "sScreenshot": "http:\u002F\u002Flive-cover.msstatic.com\u002Fhuyalive\u002F2272316519-2272316519-9759525135265562624-4544756494-10057-A-0-1-imgplus\u002F20251022225250.jpg",
                            "iIsSecret": 0,
                            "iCameraOpen": 0,
                            "iIsBluRay": 0,
                            "sBluRayMBitRate": "",
                            "iBitRate": 3000,
                            "lLiveCompatibleFlag": 0,
                            "iRecommendStatus": 5363,
                            "sRecommendTagName": "超级明星",
                            "iIsRoomPay": 0,
                            "sRoomPayTag": "",
                            "iIsWatchTogetherVip": 0,
                            "iStartTime": 1761137599,
                            "iTime": 7241,
                            "iUpdateCacheTime": 1761144840,
                            "mpCorner": {
                                "_kproto": {
                                    "_classname": "string"
                                },
                                "_bKey": 0,
                                "_bValue": 0,
                                "value": {
                                    "ListPos1": {
                                        "sContent": "上电视",
                                        "sIcon": "https:\u002F\u002Fhuyaimg.msstatic.com\u002Fcdnimage\u002Fcornermark\u002FphpC2AIU11706668197.png",
                                        "_classname": "LiveList.CornerInfo"
                                    }
                                },
                                "_classname": "map\u003Cstring,LiveList.CornerInfo\u003E"
                            },
                            "tImgRecInfo": {
                                "sType": "",
                                "sValue": "",
                                "sTypeDesc": "",
                                "_classname": "LiveList.ImgRecInfo"
                            },
                            "_classname": "LiveList.LiveListInfo"
                        }, {
                            "lUid": 1199585096447,
                            "lYyid": 35184584818842,
                            "sNick": "高冷男神钱小佳",
                            "iSex": 1,
                            "iLevel": 42,
                            "sAvatar180": "https:\u002F\u002Fhuyaimg.msstatic.com\u002Favatar\u002F1033\u002F3a\u002F23a697f568984460e1dc4d058b5c85_180_135.jpg?1643699658",
                            "lProfileRoom": 229346,
                            "sPrivateHost": "qianjia2022",
                            "sProfileHomeHost": "qianjia2022",
                            "iIsPlatinum": 1,
                            "lActivityId": 0,
                            "lActivityCount": 269157,
                            "iGid": 2165,
                            "iGameId": 0,
                            "sGameFullName": "户外",
                            "sGameHostName": "huwai",
                            "iBussType": 8,
                            "lLiveId": 7563843037021004000,
                            "lChannel": 1199585096447,
                            "lLiveChannel": 1199585096447,
                            "lUserCount": 3730425,
                            "lTotalCount": 3730425,
                            "sRoomName": "",
                            "sIntroduction": "72小时爬五岳挑战",
                            "sPreviewUrl": "",
                            "iLiveSourceType": 6,
                            "iScreenType": 1,
                            "sScreenshot": "http:\u002F\u002Flive-cover.msstatic.com\u002Fhuyalive\u002F1199585096447-1199585096447-5537161443905896448-2399170316350-10057-A-0-1-imgplus\u002F20251022225252.jpg",
                            "iIsSecret": 0,
                            "iCameraOpen": 0,
                            "iIsBluRay": 0,
                            "sBluRayMBitRate": "",
                            "iBitRate": 1200,
                            "lLiveCompatibleFlag": 0,
                            "iRecommendStatus": 2381,
                            "sRecommendTagName": "白金偶像",
                            "iIsRoomPay": 0,
                            "sRoomPayTag": "",
                            "iIsWatchTogetherVip": 0,
                            "iStartTime": 1761094443,
                            "iTime": 50397,
                            "iUpdateCacheTime": 1761144840,
                            "mpCorner": {
                                "_kproto": {
                                    "_classname": "string"
                                },
                                "_bKey": 0,
                                "_bValue": 0,
                                "value": {
                                    "ListPos1": {
                                        "sContent": "白金偶像",
                                        "sIcon": "",
                                        "_classname": "LiveList.CornerInfo"
                                    }
                                },
                                "_classname": "map\u003Cstring,LiveList.CornerInfo\u003E"
                            },
                            "tImgRecInfo": {
                                "sType": "",
                                "sValue": "",
                                "sTypeDesc": "",
                                "_classname": "LiveList.ImgRecInfo"
                            },
                            "_classname": "LiveList.LiveListInfo"
                        }, {
                            "lUid": 1677942333,
                            "lYyid": 1909591507,
                            "sNick": "集梦会长【-超跑116-】",
                            "iSex": 1,
                            "iLevel": 50,
                            "sAvatar180": "https:\u002F\u002Fhuyaimg.msstatic.com\u002Favatar\u002F1079\u002F9e\u002Fc008b3d87b701d64bbfc7485cf1f8d_180_135.jpg?1683805119",
                            "lProfileRoom": 116,
                            "sPrivateHost": "1909591507",
                            "sProfileHomeHost": "1909591507",
                            "iIsPlatinum": 1,
                            "lActivityId": 0,
                            "lActivityCount": 5309552,
                            "iGid": 2165,
                            "iGameId": 0,
                            "sGameFullName": "户外",
                            "sGameHostName": "huwai",
                            "iBussType": 8,
                            "lLiveId": 7564002500972793000,
                            "lChannel": 1677942333,
                            "lLiveChannel": 1677942333,
                            "lUserCount": 3485694,
                            "lTotalCount": 3485694,
                            "sRoomName": "",
                            "sIntroduction": "抽苹果17",
                            "sPreviewUrl": "",
                            "iLiveSourceType": 6,
                            "iScreenType": 1,
                            "sScreenshot": "http:\u002F\u002Flive-cover.msstatic.com\u002Fhuyalive\u002F1677942333-1677942333-7206707444808941568-3356008122-10057-A-0-1-imgplus\u002F20251022225248.jpg",
                            "iIsSecret": 0,
                            "iCameraOpen": 0,
                            "iIsBluRay": 0,
                            "sBluRayMBitRate": "",
                            "iBitRate": 2000,
                            "lLiveCompatibleFlag": 0,
                            "iRecommendStatus": 498,
                            "sRecommendTagName": "白金精选",
                            "iIsRoomPay": 0,
                            "sRoomPayTag": "",
                            "iIsWatchTogetherVip": 0,
                            "iStartTime": 1761131571,
                            "iTime": 13269,
                            "iUpdateCacheTime": 1761144840,
                            "mpCorner": {
                                "_kproto": {
                                    "_classname": "string"
                                },
                                "_bKey": 0,
                                "_bValue": 0,
                                "value": {
                                    "ListPos1": {
                                        "sContent": "白金精选",
                                        "sIcon": "",
                                        "_classname": "LiveList.CornerInfo"
                                    }
                                },
                                "_classname": "map\u003Cstring,LiveList.CornerInfo\u003E"
                            },
                            "tImgRecInfo": {
                                "sType": "",
                                "sValue": "",
                                "sTypeDesc": "",
                                "_classname": "LiveList.ImgRecInfo"
                            },
                            "_classname": "LiveList.LiveListInfo"
                        }, {
                            "lUid": 1199526826004,
                            "lYyid": 35184434163962,
                            "sNick": "集梦张开朗【233】",
                            "iSex": 1,
                            "iLevel": 46,
                            "sAvatar180": "https:\u002F\u002Fhuyaimg.msstatic.com\u002Favatar\u002F1006\u002F12\u002F5dc2603f810eb80e5197aab7dcfc00_180_135.jpg?1760729975",
                            "lProfileRoom": 233,
                            "sPrivateHost": "35184434163962",
                            "sProfileHomeHost": "35184434163962",
                            "iIsPlatinum": 1,
                            "lActivityId": 0,
                            "lActivityCount": 947339,
                            "iGid": 2165,
                            "iGameId": 0,
                            "sGameFullName": "户外",
                            "sGameHostName": "huwai",
                            "iBussType": 8,
                            "lLiveId": 7564014303095539000,
                            "lChannel": 1199526826004,
                            "lLiveChannel": 1199526826004,
                            "lUserCount": 3161062,
                            "lTotalCount": 3161062,
                            "sRoomName": "",
                            "sIntroduction": "西安 年度快乐",
                            "sPreviewUrl": "",
                            "iLiveSourceType": 6,
                            "iScreenType": 1,
                            "sScreenshot": "http:\u002F\u002Flive-cover.msstatic.com\u002Fhuyalive\u002F1199526826004-1199526826004-5286891796897464320-2399053775464-10057-A-0-1-imgplus\u002F20251022225253.jpg",
                            "iIsSecret": 0,
                            "iCameraOpen": 0,
                            "iIsBluRay": 0,
                            "sBluRayMBitRate": "",
                            "iBitRate": 2400,
                            "lLiveCompatibleFlag": 0,
                            "iRecommendStatus": 2381,
                            "sRecommendTagName": "白金偶像",
                            "iIsRoomPay": 0,
                            "sRoomPayTag": "",
                            "iIsWatchTogetherVip": 0,
                            "iStartTime": 1761134319,
                            "iTime": 10521,
                            "iUpdateCacheTime": 1761144840,
                            "mpCorner": {
                                "_kproto": {
                                    "_classname": "string"
                                },
                                "_bKey": 0,
                                "_bValue": 0,
                                "value": {
                                    "ListPos1": {
                                        "sContent": "白金偶像",
                                        "sIcon": "",
                                        "_classname": "LiveList.CornerInfo"
                                    }
                                },
                                "_classname": "map\u003Cstring,LiveList.CornerInfo\u003E"
                            },
                            "tImgRecInfo": {
                                "sType": "",
                                "sValue": "",
                                "sTypeDesc": "",
                                "_classname": "LiveList.ImgRecInfo"
                            },
                            "_classname": "LiveList.LiveListInfo"
                        }],
                        "_classname": "list\u003CLiveList.LiveListInfo\u003E"
                    },
                    "_classname": "LiveList.RoomRecommendLiveTag"
                }],
                "welcomeText": "18点直播至24点。\n马甲格式：XXX【熬鹰小队】"
    }
        "#;

        let result = LiveStreamExtractor::extract_infos(js_content);
        assert!(result.is_ok());

        let (user_info, room_info, stream_info) = result.unwrap();

        assert_eq!(user_info.user_id, "156976698");
        assert_eq!(user_info.user_name, "三导-赵迪克【熬鹰小队主鹰】");
        assert_eq!(user_info.user_avatar, "https://huyaimg.msstatic.com/avatar/1003/23/3be5ff7cff0f6d08fee796ac537ef0_180_135.jpg?1525686175");
        assert_eq!(room_info.room_id, "857824");
        assert!(stream_info.hls_url.starts_with("https://"));
    }
}
