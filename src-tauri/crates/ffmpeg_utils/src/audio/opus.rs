//! The two Opus profiles used by previews and subtitle-service uploads.

use std::path::Path;

use super::encode::{encode, EncodeProfile};

/// Encode a 16 kHz mono 64 kbit/s VBR Opus preview.
pub async fn encode_opus_sample(input: &Path, output: &Path) -> Result<(), String> {
    encode(input, output, EncodeProfile::OpusSample).await
}

/// Encode a 128 kbit/s VBR Opus upload with source-adaptive rate and layout.
pub async fn encode_opus_upload(input: &Path, output: &Path) -> Result<(), String> {
    encode(input, output, EncodeProfile::OpusUpload).await
}
