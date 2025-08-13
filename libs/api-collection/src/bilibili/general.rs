use pct_str::{PctString, URIReserved};
use std::time::SystemTime;

use crate::errors::ApiCollectionError;

pub async fn get_sign(
    client: &reqwest::Client,
    mut parameters: serde_json::Value,
) -> Result<String, ApiCollectionError> {
    let table = vec![
        46, 47, 18, 2, 53, 8, 23, 32, 15, 50, 10, 31, 58, 3, 45, 35, 27, 43, 5, 49, 33, 9, 42, 19,
        29, 28, 14, 39, 12, 38, 41, 13, 37, 48, 7, 16, 24, 55, 40, 61, 26, 17, 0, 1, 60, 51, 30, 4,
        22, 25, 54, 21, 56, 59, 6, 63, 57, 62, 11, 36, 20, 34, 44, 52,
    ];
    let nav_info: serde_json::Value = client
        .get("https://api.bilibili.com/x/web-interface/nav")
        .send()
        .await?
        .json()
        .await?;
    let re = regex::Regex::new(r"wbi/(.*).png").unwrap();
    let img = re
        .captures(nav_info["data"]["wbi_img"]["img_url"].as_str().unwrap())
        .unwrap()
        .get(1)
        .unwrap()
        .as_str();
    let sub = re
        .captures(nav_info["data"]["wbi_img"]["sub_url"].as_str().unwrap())
        .unwrap()
        .get(1)
        .unwrap()
        .as_str();
    let raw_string = format!("{}{}", img, sub);
    let mut encoded = Vec::new();
    table.into_iter().for_each(|x| {
        if x < raw_string.len() {
            encoded.push(raw_string.as_bytes()[x]);
        }
    });
    // only keep 32 bytes of encoded
    encoded = encoded[0..32].to_vec();
    let encoded = String::from_utf8(encoded).unwrap();
    // Timestamp in seconds
    let wts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    parameters
        .as_object_mut()
        .unwrap()
        .insert("wts".to_owned(), serde_json::Value::String(wts.to_string()));
    // Get all keys from parameters into vec
    let mut keys = parameters
        .as_object()
        .unwrap()
        .keys()
        .map(|x| x.to_owned())
        .collect::<Vec<String>>();
    // sort keys
    keys.sort();
    let mut params = String::new();
    keys.iter().for_each(|x| {
        params.push_str(x);
        params.push('=');
        // Value filters !'()* characters
        let value = parameters
            .get(x)
            .unwrap()
            .as_str()
            .unwrap()
            .replace(['!', '\'', '(', ')', '*'], "");
        let value = PctString::encode(value.chars(), URIReserved);
        params.push_str(value.as_str());
        // add & if not last
        if x != keys.last().unwrap() {
            params.push('&');
        }
    });
    // md5 params+encoded
    let w_rid = md5::compute(params.to_string() + encoded.as_str());
    let params = params + format!("&w_rid={:x}", w_rid).as_str();
    Ok(params)
}
