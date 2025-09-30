use regex::Regex;
use serde_json::Value;

pub struct LiveStreamExtractor;

impl LiveStreamExtractor {
    /// 从 JavaScript 代码中提取指定的变量
    pub fn extract_variables(
        js_content: &str,
    ) -> Result<(Value, Value, Value), super::errors::HuyaClientError> {
        let tt_room_data = Self::extract_variable(js_content, "TT_ROOM_DATA")?;
        let tt_profile_info = Self::extract_variable(js_content, "TT_PROFILE_INFO")?;
        let hy_player_config = Self::extract_variable(js_content, "hyPlayerConfig")?;

        Ok((tt_room_data, tt_profile_info, hy_player_config))
    }

    /// 更健壮的提取方法，处理嵌套的大括号
    fn extract_variable(
        js_content: &str,
        var_name: &str,
    ) -> Result<Value, super::errors::HuyaClientError> {
        let var_pattern = format!(r"var\s+{}\s*=\s*", regex::escape(var_name));
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
    fn test_extract_variables() {
        let js_content = r#"
<script>
    var TT_META_DATA = {
        "time": 1759162226
    };
    var TT_ROOM_DATA = {
        "type": "NORMAL",
        "state": "ON",
        "isOn": true,
        "isOff": false,
        "isReplay": false,
        "isPayRoom": 0,
        "isSecret": 0,
        "roomPayPassword": "",
        "id": "1199649734706",
        "sid": "1199649734706",
        "channel": "1199649734706",
        "liveChannel": "1199649734706",
        "liveId": "7555535483704922830",
        "shortChannel": 0,
        "isBluRay": 0,
        "gameFullName": "二次元",
        "gameHostName": "2633",
        "screenType": 1,
        "startTime": 1759160190,
        "totalCount": 47526,
        "cameraOpen": 0,
        "liveCompatibleFlag": 0,
        "bussType": 8,
        "isPlatinum": 1,
        "isAutoBitrate": 0,
        "screenshot": "//live-cover.msstatic.com/huyalive/1199649734706-1199649734706-5814780652381274112-2399299592868-10057-A-0-1/20250930001000.jpg",
        "previewUrl": "",
        "gameId": 0,
        "liveSourceType": 0,
        "privateHost": "35184693538269",
        "profileRoom": 30456807,
        "recommendStatus": 0,
        "popular": 0,
        "gid": 2633,
        "introduction": "水水润润~",
        "isRedirectHuya": 0,
        "isShowMmsProgramList": 0,
        "adStatus": 1
    };
    var TT_PROFILE_INFO = {
        "sex": 2,
        "lp": "1199649734706",
        "aid": 0,
        "yyid": "35184693538269",
        "nick": "兔兔好棒",
        "avatar": "https://huyaimg.msstatic.com/avatar/1036/00/00470cfe1890ad5532ade2ce20253e_180_135.jpg?1742560471",
        "fans": 2790,
        "freezeLevel": 0,
        "host": "35184693538269",
        "profileRoom": 30456807
    };
    var TT_PLAYER_CFG = {
        "flashAdvanceVersion": "-1",
        "advanceChannel": "1029443883_1029443883",
        "flashDomain": "//hyplayer.msstatic.com/",
        "h5Advance": "-1",
        "h5domain": "https://a.msstatic.com/huya/h5player/room/",
        "delOriginalRate": "-1",
        "homePageH5Domain": "https://a.msstatic.com/huya/h5player/home/",
        "h5PlayerP2PVersion": "-1",
        "h5ShareEntry": "//a.msstatic.com/huya/h5player/outstation/guide4.js",
        "replayUseH5": "true",
        "yqkPlayerVer": "2008131707",
        "homePagePlayerIncludeSDK": "2308081431",
        "defaultFlashPlayer": "false",
        "h5sdk": "//a.msstatic.com/huya/h5player/sdk/",
        "h5outstationVer": "2201181143",
        "h5homeSdk": "1703061730",
        "h5AdvanceVersion": "-1",
        "playerConfig": "{&quot;danmu&quot;:&quot;1&quot;}",
        "h5sdkver": "1706131454",
        "h5ver": "1802021544",
        "flashAdvance": "-1",
        "h5gopChannel": "-1",
        "h5hlssdkver": "hlssdk_1801261608",
        "h5PlayerP2PChannel": "-1",
        "h5outstationDomain": "//a.msstatic.com/huya/h5player/outstation",
        "homePageH5Ver": "1808091448",
        "flashVersion": "v3.2_20110603",
        "useflash": "-1",
        "playerdata": "{&quot;usehttps&quot;:&quot;1&quot;,&quot;sdk&quot;:{}}",
        "yqkPlayerDomain": "//a.msstatic.com/huya/h5player/yqk/",
        "homePageH5": "true",
        "h5PlayerAndHuyaSDKChannel": "-1",
        "deleteOriginalPainting": "-1",
        "h5PlayerAndHuyaSDKVersion": "-1",
        "homepage": "hp_76",
        "h5PlayerIncludeSDK": "2212151455"
    };
    var TT_PLAYER_CONF = {
        "app": "live",
        "grayGameIdVersion": "{\"gid\":[{\"gid\":\"-1\",\"ver\":\"2410281700\",\"desc\":\"endad\"}]}",
        "grayPidVersion": "{\"pid\":[{\"pid\":\"2281473127,1099511697221,1346609715,1575250000,1656466770,1099531728424,1430972471,1199536349047,1001276654,1279512307519,13671279,869228469,1708907089,1173534659,1690094984,1860008570,1843949176,1199630339535\",\"ver\":\"2509191448\",\"desc\":\"ad_preload\"},{\"pid\":\"1239543865812\",\"ver\":\"2508271543\",\"desc\":\"change_bitrate\"}]}",
        "h5playerDomian": "https://a.msstatic.com/huya/h5player/room/",
        "h5playerVersion": "2509261431",
        "h5playerBackUpVersion": "2403041048",
        "module": "common",
        "playerConfig": "{\"qimeiVer\":\"0.6.3\",\"closeHdr\":\"0\",\"gzip\":\"1\",\"gzipExp\":3,\"adVolume\":\"0.2\",\"danmuMaskOpen\":\"1\",\"danmu\":\"1\",\"openMFVideo\":\"1\",\"lowLatencyGameId\":\"6613,7001\",\"lowLatencyPid\":\"1544844522,50075521,50043174\",\"hlsLivePid\":\"\",\"ob\":{\"x\":\"0.36\",\"y\":\"0.3\",\"scale\":\"0.25\",\"scaleMin\":\"0.25\",\"scaleMax\":\"1\",\"renderMod\":\"1\"}}",
        "playerdata": "{\"danmuP2P\":\"1\",\"isAutoNoPicture\":0,\"videoDns\":\"1\",\"danmuMask\":\"1\",\"flacSwitch\":\"1\",\"p2pLineClose\":\"\",\"usehttps\":\"1\",\"sdk\":{\"statLv\":[1,1,0,0,0,0,0,1,1,0,1,1,0,1,1,1,0],\"statIgnoreOpenCfg\":\"all|all:95\",\"statHoldFieldsSdk\":[\"a_render_silence_time\"],\"punchByNat\":0,\"maxResendTimes\":6,\"p2pUpNat\":1,\"p2pOrdered\":0,\"mpsOrdered\":0,\"mpsBufferSendModeCfg\":\"all|all:0\",\"p2pUpLimit\":1,\"mpsCroUseIpv6\":1,\"crossClientCloseCfg\":\"all|all:0\",\"mpsBufferBlockPer\":40,\"cdnCloseFirst\":0,\"connectPcdnTimeout\":3000,\"mpsBufferSendPowerNormal\":1.2,\"mpsBufferSendMaxSize\":256,\"mpsLossRateClose\":100,\"noDataTimeout\":2500,\"noPcdnDataTimeout\":4000,\"mpsLossRateOpen\":0,\"mpsBlackUids\":[1279546440890,1199629649413,1199626834332,368388675,1230013911,1259554372022],\"minPlayout\":350,\"maxPlayout\":750,\"patchSubTs\":300,\"p2pOpenBrowsers\":[\"opera\",\"firefox\",\"safari\"],\"mpsNotCrossBrowsers\":[\"firefox\",\"safari\"],\"jitCfgFlv\":[[0,3000,2000,4000]],\"mpsFailTime\":1,\"resendSafeBuf\":1000,\"rangeNums\":256,\"maxUplinkBw\":10000,\"mpsSrcWaitStopTime\":10000,\"mpsUseCfg\":[1,1,1],\"mpsCloseV6\":0,\"mpsCfg\":\"all|all:100\",\"mpsCrossCfg\":\"all|all:100\",\"mpsCrossV6\":1,\"mpsRspFlag\":1,\"p2pMediaLogCfg\":\"all|all:100\",\"jitCfgPidFlv\":[[\"1099531774561\",[[0,1000,500,1500]]]],\"wcs265BlackUids\":[21152545,1656754951,1344550525,1579609842,2233781281,1259523555338,1259520179264,2315326689,1467338235,2369473704,1199554450502,1199639682681,1199524191795],\"wupFailPer\":80,\"iaasCfg\":[0,1,3,0.3,4000,600,200,5000,5,0.4],\"notUsePcdnUids\":[],\"p2pSignalResendLog\":1,\"closeAllUids\":[1199620812769,1259518098973,1691592144,878924459,1199635224176,1199635204861,1199635489744,1199635199657,1199635491142,1199536199401,1567578875,1647705781,1337985088,1279525002112,368388675,1829378828,1259554372022],\"vodAv1Cfg\":\"all|all:0\",\"p2pWssLines\":\"1,3,5,8\",\"quickAccessLines\":\"1,3,5,6,7,8,9\",\"pcdnStun\":[\"1144904762\",\"-server.va.huya.com\",3493],\"resendAudioCfg\":\"all|all:500,1346609715|all:300\",\"pcdnRetryCnt\":2,\"signalCntMax\":10,\"rtPeerUgMinCfg\":[[1099531728402,[[0,700]]],[1571856353,[[0,700]]]],\"peerRtoCfg\":[[1,[[0,-200]]]],\"audioDtsCfg\":\"all|all:0\",\"rtPeerCfg\":[[0,3,0]],\"rangeJit\":0,\"openPcdnAB\":0,\"signalOpenExp\":\"1259515661837|all:10\",\"deleteH264Aud\":[1279522147815],\"mediaBaseIndexMode\":\"2\",\"useChangeRate\":[1199548593898,1239543865812],\"changeBitrates\":[[1239543865812, 8000]],\"p2pMediaCfg\":\"all|all:100,1099531627791|all:0\",\"p2pConfig\":{\"swapdomain\":{\"line_5\":[\"txtest.p2p.huya.com|tx.p2p.huya.com|tx3.p2p.huya.com\",\"tx2.p2p.huya.com\"]}},\"xp2pConfig\":[],\"s4kConfig\":[],\"h265Config\":[[[2000,4000],[120000,80000]],[4000,2000,1300,1000,500,350,17200],75,[[350,500,1000,1300,2000,4000,17200],[3,3,3,3,3,3,5]],30,3,[true,true]],\"h265Config2\":[6,1],\"h265MseConfig\":[1,[86],[500,1000,1300,2000,4000,17200],[],[[500,1000,1300,2000,4000,8000,10000,17200,17100],[3,3,3,3,5,11,11,11,11],[],[]],0],\"h265MseWhiteBlackUids\":[[],[2240935230]],\"isAutoH265\":0,\"enableAiMosaic\":1,\"aiBlackUids\":[1099531627889,50043344],\"aiRequestInterval\":2000,\"aiP2PFlvConfig\":[5000,10],\"aiRandomPercent\":100000,\"h265PercentConfig\":100000,\"setVideoCTCfg\":[-100,1000,10000,1,3,1500],\"isUseWebWorkerTick\":0,\"danmuPercent\":-1,\"av1PercentConfig\":100000,\"h265BlackUids\":[],\"autoDownTimer\":1,\"autoDownMaxBitrate\":0,\"autoReportCfg\":[0,3,0,0],\"continueBufferDeltaStart\":4500,\"aiABRandomPercent\":0,\"p2p302Config\":[[[6,1,1,1],[66,1,1,1],[1,1,1,1],[3,1,1,1],[5,1,1,1],[14,1,1,1],[13,1,1,1],[15,1,1,1]],1],\"aiBlackLines\":[],\"vodPcdnOpenBuffer\":4000,\"vodPcdnCloseBuffer\":3000,\"vodPcdnCoolTime\":60000,\"vodPcdnBufferCfg\":[[4000,3000,30000],[],[]],\"isMuteAct\":0,\"isReplayConfigSupportH265\":0,\"h265MseChromeConfig\":[107,30000,4,0,1],\"renderStat\":[1],\"pause500Cfg\":[300,[]],\"isAutoWcsReport\":0,\"webCodecCfg\":[4,20000,2000,1,1500,[5000,10000,20000]],\"webCodecAVDeltaCfg\":[5000,2000],\"webCodecBlackUids\":[2218841822,2385292509,1099531768232,1182368555,1199611344355,1279513816099,1571905909,1199639682681,1199552896543,1555556780,1199591052064,1099511667849,1199639978777,1199539288958,1099531627777],\"isVodSupportH265\":0,\"h265HardBlackBrows\":[],\"wcsSoft264Uids\":[878924459],\"popSize0Brows\":[[\"firefox\",115]],\"wcsSoftBrows\":[],\"enhanceVCfg\":[1,4000,6,1080],\"mse265BlackBrowVers\":[],\"eHBlackAnchoruids\":[],\"jumpBufferCfg\":[30,1000,10000,5000,30,25],\"enableEdgeBroMseCfg\":[],\"fpsBitrateCfg\":[100,[1239543149256,50043104,2179608815,1853167339,1346609715,1560173863],{\"4200\":\"17300\"},120],\"closeUids\":[1230013911],\"enhanceVodVCfg\":[1,4000,6,1080],\"payRoomCfg\":[1,5],\"wcs265BlackBrowNames\":[],\"replayWebcodecsGameIds\":[1,2336,3115],\"jitLow\":[3000,2000,4000]}}",
        "pushMsgControl": "https://fedlib.msstatic.com/fedbasic/huyabaselibs/push-msg-control/push-msg-control.global.0.0.3.prod.js",
        "tafSignal": "https://fedlib.msstatic.com/fedbasic/huyabaselibs/taf-signal/taf-signal.global.0.0.17.prod.js",
        "version": "20250929101329",
        "homePageH5Domain": "https://a.msstatic.com/huya/h5player/home/",
        "homePagePlayerIncludeSDK": "2508261427",
        "homePagePlayerConfig": "{\"test\":123}"
    };
    var TT_PROFILE_P2P_OPT = "";

    var useEncodeStream = false;
    var encodeStreamTag = 'liveRoom';
    var indexUrl = 'https://www.huya.com/';
    var APP_URL = 'https://www.huya.com/';
    var flashTime = new Date().getTime();
    var reportPageView = 'room';

    var hyPlayerConfig = {
        html5: 1,
        WEBYYSWF: 'jsscene',
        vappid: 10057,
        stream: {
            "data": [{
                "gameLiveInfo": {
                    "uid": "1199649734706",
                    "sex": 2,
                    "gameFullName": "二次元",
                    "gameHostName": "2633",
                    "startTime": 1759160190,
                    "activityId": 0,
                    "level": 17,
                    "totalCount": 47526,
                    "roomName": "",
                    "isSecret": 0,
                    "cameraOpen": 0,
                    "liveChannel": "1199649734706",
                    "bussType": 8,
                    "yyid": "35184693538269",
                    "screenshot": "http://live-cover.msstatic.com/huyalive/1199649734706-1199649734706-5814780652381274112-2399299592868-10057-A-0-1/20250930001000.jpg",
                    "activityCount": 2790,
                    "privateHost": "35184693538269",
                    "recommendStatus": 0,
                    "nick": "兔兔好棒",
                    "shortChannel": 0,
                    "avatar180": "https://huyaimg.msstatic.com/avatar/1036/00/00470cfe1890ad5532ade2ce20253e_180_135.jpg?1742560471",
                    "gid": 2633,
                    "channel": "1199649734706",
                    "introduction": "水水润润~",
                    "profileHomeHost": "35184693538269",
                    "liveSourceType": 0,
                    "screenType": 1,
                    "bitRate": 2000,
                    "gameType": 0,
                    "attendeeCount": 47526,
                    "multiStreamFlag": 0,
                    "codecType": 0,
                    "liveCompatibleFlag": 0,
                    "profileRoom": 30456807,
                    "liveId": "7555535483704922830",
                    "recommendTagName": "",
                    "contentIntro": "",
                    "mMiscInfo": {
                        "video_layout": "",
                        "c_onhook": "0",
                        "lowdelay_mode": "-1",
                        "StreamDelay": "0",
                        "VideoHeight": "1080",
                        "real_ua": "pc_exe_template&6011900&official",
                        "live_status_text": "直播中",
                        "VideoWidth": "1920",
                        "forbid_pk": "0",
                        "HuyaAudioACQEnable": "1",
                        "pc_director": "",
                        "HUYA_MAIXU": "2"
                    }
                },
                "gameStreamInfoList": [{
                    "sCdnType": "AL",
                    "iIsMaster": 0,
                    "lChannelId": "1199649734706",
                    "lSubChannelId": "1199649734706",
                    "lPresenterUid": "1199649734706",
                    "sStreamName": "1199649734706-1199649734706-5814780652381274112-2399299592868-10057-A-0-1",
                    "sFlvUrl": "http://al.flv.huya.com/src",
                    "sFlvUrlSuffix": "flv",
                    "sFlvAntiCode": "wsSecret=f2c98fe976ea448dec4161a199a9877d&wsTime=68dab09e&fm=RFdxOEJjSjNoNkRKdDZUWV8kMF8kMV8kMl8kMw%3D%3D&ctype=huya_live&fs=bgct",
                    "sHlsUrl": "http://al.hls.huya.com/src",
                    "sHlsUrlSuffix": "m3u8",
                    "sHlsAntiCode": "wsSecret=f2c98fe976ea448dec4161a199a9877d&wsTime=68dab09e&fm=RFdxOEJjSjNoNkRKdDZUWV8kMF8kMV8kMl8kMw%3D%3D&ctype=huya_live&fs=bgct",
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
                        "_classname": "list<string>"
                    },
                    "iIsP2PSupport": 0,
                    "sP2pUrl": "http://al.p2p.huya.com/huyalive",
                    "sP2pUrlSuffix": "slice",
                    "sP2pAntiCode": "wsSecret=f2c98fe976ea448dec4161a199a9877d&wsTime=68dab09e&fm=RFdxOEJjSjNoNkRKdDZUWV8kMF8kMV8kMl8kMw%3D%3D&ctype=huya_live&fs=bgct",
                    "lFreeFlag": 2,
                    "iIsHEVCSupport": 0,
                    "vP2pIPList": {
                        "_proto": {
                            "_classname": "string"
                        },
                        "_bValue": 0,
                        "value": [],
                        "_classname": "list<string>"
                    },
                    "mpExtArgs": {
                        "_kproto": {
                            "_classname": "string"
                        },
                        "_vproto": {
                            "_classname": "string"
                        },
                        "_bKey": 0,
                        "_bValue": 0,
                        "value": {},
                        "_classname": "map<string,string>"
                    },
                    "lTimespan": "15950727847",
                    "lUpdateTime": 0,
                    "_classname": "HUYA.StreamInfo"
                }, {
                    "sCdnType": "TX",
                    "iIsMaster": 0,
                    "lChannelId": "1199649734706",
                    "lSubChannelId": "1199649734706",
                    "lPresenterUid": "1199649734706",
                    "sStreamName": "1199649734706-1199649734706-5814780652381274112-2399299592868-10057-A-0-1",
                    "sFlvUrl": "http://txdirect.flv.huya.com/huyalive",
                    "sFlvUrlSuffix": "flv",
                    "sFlvAntiCode": "wsSecret=f2c98fe976ea448dec4161a199a9877d&wsTime=68dab09e&fm=RFdxOEJjSjNoNkRKdDZUWV8kMF8kMV8kMl8kMw%3D%3D&ctype=huya_live&fs=bgct",
                    "sHlsUrl": "http://txdirect.hls.huya.com/huyalive",
                    "sHlsUrlSuffix": "m3u8",
                    "sHlsAntiCode": "wsSecret=f2c98fe976ea448dec4161a199a9877d&wsTime=68dab09e&fm=RFdxOEJjSjNoNkRKdDZUWV8kMF8kMV8kMl8kMw%3D%3D&ctype=huya_live&fs=bgct",
                    "iLineIndex": 5,
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
                        "_classname": "list<string>"
                    },
                    "iIsP2PSupport": 0,
                    "sP2pUrl": "http://tx.p2p.huya.com/huyalive",
                    "sP2pUrlSuffix": "slice",
                    "sP2pAntiCode": "wsSecret=f2c98fe976ea448dec4161a199a9877d&wsTime=68dab09e&fm=RFdxOEJjSjNoNkRKdDZUWV8kMF8kMV8kMl8kMw%3D%3D&ctype=huya_live&fs=bgct",
                    "lFreeFlag": 1,
                    "iIsHEVCSupport": 0,
                    "vP2pIPList": {
                        "_proto": {
                            "_classname": "string"
                        },
                        "_bValue": 0,
                        "value": [],
                        "_classname": "list<string>"
                    },
                    "mpExtArgs": {
                        "_kproto": {
                            "_classname": "string"
                        },
                        "_vproto": {
                            "_classname": "string"
                        },
                        "_bKey": 0,
                        "_bValue": 0,
                        "value": {},
                        "_classname": "map<string,string>"
                    },
                    "lTimespan": "15950727847",
                    "lUpdateTime": 0,
                    "_classname": "HUYA.StreamInfo"
                }, {
                    "sCdnType": "HS",
                    "iIsMaster": 0,
                    "lChannelId": "1199649734706",
                    "lSubChannelId": "1199649734706",
                    "lPresenterUid": "1199649734706",
                    "sStreamName": "1199649734706-1199649734706-5814780652381274112-2399299592868-10057-A-0-1",
                    "sFlvUrl": "http://hs.flv.huya.com/src",
                    "sFlvUrlSuffix": "flv",
                    "sFlvAntiCode": "wsSecret=f2c98fe976ea448dec4161a199a9877d&wsTime=68dab09e&fm=RFdxOEJjSjNoNkRKdDZUWV8kMF8kMV8kMl8kMw%3D%3D&ctype=huya_live&fs=bgct",
                    "sHlsUrl": "http://hs.hls.huya.com/src",
                    "sHlsUrlSuffix": "m3u8",
                    "sHlsAntiCode": "wsSecret=f2c98fe976ea448dec4161a199a9877d&wsTime=68dab09e&fm=RFdxOEJjSjNoNkRKdDZUWV8kMF8kMV8kMl8kMw%3D%3D&ctype=huya_live&fs=bgct",
                    "iLineIndex": 14,
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
                        "_classname": "list<string>"
                    },
                    "iIsP2PSupport": 0,
                    "sP2pUrl": "http://hs.p2p.huya.com/huyalive",
                    "sP2pUrlSuffix": "slice",
                    "sP2pAntiCode": "wsSecret=f2c98fe976ea448dec4161a199a9877d&wsTime=68dab09e&fm=RFdxOEJjSjNoNkRKdDZUWV8kMF8kMV8kMl8kMw%3D%3D&ctype=huya_live&fs=bgct",
                    "lFreeFlag": 0,
                    "iIsHEVCSupport": 0,
                    "vP2pIPList": {
                        "_proto": {
                            "_classname": "string"
                        },
                        "_bValue": 0,
                        "value": [],
                        "_classname": "list<string>"
                    },
                    "mpExtArgs": {
                        "_kproto": {
                            "_classname": "string"
                        },
                        "_vproto": {
                            "_classname": "string"
                        },
                        "_bKey": 0,
                        "_bValue": 0,
                        "value": {},
                        "_classname": "map<string,string>"
                    },
                    "lTimespan": "15950727847",
                    "lUpdateTime": 0,
                    "_classname": "HUYA.StreamInfo"
                }]
            }],
            "count": 0,
            "vMultiStreamInfo": [{
                "sDisplayName": "超清",
                "iBitRate": 0,
                "iCodecType": 0,
                "iCompatibleFlag": 0,
                "iHEVCBitRate": -1,
                "_classname": "LiveRoom.LiveBitRateInfo"
            }],
            "iWebDefaultBitRate": 2500,
            "iFrameRate": 30
        }
    };

    window.TT_LIVE_TIMING = {};
</script>
        "#;

        let result = LiveStreamExtractor::extract_variables(js_content);
        assert!(result.is_ok());

        let (room_data, profile_info, player_config) = result.unwrap();

        assert_eq!(room_data["type"], "NORMAL");
        assert_eq!(profile_info["nick"], "兔兔好棒");
        assert_eq!(player_config["html5"], 1);
    }
}
