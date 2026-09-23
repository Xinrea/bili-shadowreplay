//! Huya CDN anticode generation.
//!
//! Huya stream URLs must carry a signed query whose `wsSecret` is an MD5
//! computed from the `fm` template shipped inside the anticode itself. The
//! precomputed `wsSecret` served with the room page is bound to the page
//! request's identity and is rejected by some CDN nodes with HTTP 403, so it
//! must always be recomputed locally, following the algorithm used by the
//! Huya web player (as implemented in streamlink's huya plugin):
//!
//! `wsSecret = md5("{fm_salt}_{u}_{stream_name}_{md5("{seqid}|{ctype}|{t}")}_{wsTime}")`

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose, Engine as _};
use md5::compute as md5_hex;
use url::form_urlencoded;

/// Fixed playback parameters observed in the Huya web player.
const T: u32 = 100;
const VER: u32 = 1;
const SV: &str = "2401090219";
const CODEC: u32 = 264;
const RATIO: u32 = 0;

/// Anonymous viewer identity range used by the web player.
const UID_RANGE: std::ops::Range<u32> = 12340000..12349999;

/// URL构建器
pub struct UrlBuilder;

impl UrlBuilder {
    /// Recompute the signed anticode query for `stream_name`.
    ///
    /// `anti_code` is the raw `sHlsAntiCode` string (or the query part of a
    /// decoded `liveLineUrl`); the `fm`, `wsTime`, `ctype` and `fs` values are
    /// taken from it. Returns `Err` when the signature inputs are missing, in
    /// which case the caller should keep the server-provided query as-is.
    pub fn build_anticode(anti_code: &str, stream_name: &str) -> Result<String, String> {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let uid = fastrand::u32(UID_RANGE);
        Self::build_anticode_with(anti_code, stream_name, now_ms, uid)
    }

    /// [`Self::build_anticode`] with injectable randomness, for tests.
    fn build_anticode_with(
        anti_code: &str,
        stream_name: &str,
        now_ms: u64,
        uid: u32,
    ) -> Result<String, String> {
        let params: HashMap<String, String> = form_urlencoded::parse(anti_code.as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();

        let fm = params.get("fm").ok_or("anticode is missing `fm`")?;
        let ws_time = params.get("wsTime").ok_or("anticode is missing `wsTime`")?;
        let ctype = params.get("ctype").map(String::as_str).unwrap_or("huya_live");
        let fs = params.get("fs").map(String::as_str).unwrap_or("bgct");

        // `fm` is a base64 template like "<salt>_$0_$1_$2_$3"; only the salt
        // is part of the signature.
        let fm_decoded = urlencoding::decode(fm)
            .map_err(|e| format!("invalid `fm` encoding: {e}"))?
            .to_string();
        let fm_decoded = general_purpose::STANDARD
            .decode(fm_decoded.as_bytes())
            .map_err(|e| format!("invalid `fm` base64: {e}"))?;
        let fm_decoded =
            String::from_utf8(fm_decoded).map_err(|e| format!("invalid `fm` utf8: {e}"))?;
        let fm_salt = fm_decoded.split('_').next().unwrap_or_default();

        // The web player rotates the uid left by 8 bits before signing.
        let converted_uid = uid.rotate_left(8);
        let seq_id = uid as u64 + now_ms;

        let hash = format!("{:x}", md5_hex(format!("{seq_id}|{ctype}|{T}")));
        let ws_secret = format!(
            "{:x}",
            md5_hex(format!(
                "{fm_salt}_{converted_uid}_{stream_name}_{hash}_{ws_time}"
            ))
        );

        Ok(format!(
            "wsSecret={ws_secret}&wsTime={ws_time}&ctype={ctype}&fs={fs}&seqid={seq_id}&u={converted_uid}&sdk_sid={now_ms}&ratio={RATIO}&t={T}&ver={VER}&sv={SV}&codec={CODEC}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Anticode shape from a real m.huya.com room page, with the uid and
    /// timestamp pinned so the expected output can be asserted exactly.
    const ANTI_CODE: &str = "wsSecret=7abc7dec8809146f31f92046eb044e3b&wsTime=68fa41ba&fm=RFdxOEJjSjNoNkRKdDZUWV8kMF8kMV8kMl8kMw%3D%3D&ctype=tars_mobile&fs=bgct&t=103";
    const STREAM_NAME: &str = "156976698-156976698-674209784144068608-314076852-10057-A-0-1";

    #[test]
    fn test_build_anticode_recomputes_the_signature() {
        let anticode =
            UrlBuilder::build_anticode_with(ANTI_CODE, STREAM_NAME, 1790087654321, 12345678)
                .unwrap();

        // Verified against the streamlink huya algorithm.
        assert_eq!(
            anticode,
            "wsSecret=1617a1457574a61272203d8c6beb9a5c&wsTime=68fa41ba&ctype=tars_mobile&fs=bgct&seqid=1790099999999&u=3160493568&sdk_sid=1790087654321&ratio=0&t=100&ver=1&sv=2401090219&codec=264"
        );
    }

    #[test]
    fn test_build_anticode_defaults_missing_params() {
        let anticode = UrlBuilder::build_anticode_with(
            "wsTime=68fa41ba&fm=RFdxOEJjSjNoNkRKdDZUWV8kMF8kMV8kMl8kMw%3D%3D",
            STREAM_NAME,
            1790087654321,
            12345678,
        )
        .unwrap();

        assert!(anticode.contains("ctype=huya_live"));
        assert!(anticode.contains("fs=bgct"));
    }

    #[test]
    fn test_build_anticode_requires_signature_inputs() {
        assert!(UrlBuilder::build_anticode("wsTime=68fa41ba", STREAM_NAME).is_err());
        assert!(UrlBuilder::build_anticode(
            "fm=RFdxOEJjSjNoNkRKdDZUWV8kMF8kMV8kMl8kMw%3D%3D",
            STREAM_NAME
        )
        .is_err());
    }
}
