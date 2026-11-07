//! The audio library's supported storage formats, without command arguments.

use std::path::Path;

use super::{
    encode::{encode, EncodeProfile},
    remux::{remux_first_audio, CopyAudioFormat},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportAudioMode {
    CopyWav,
    CopyFlac,
    CopyAacM4a,
    Pcm16Wav,
    Aac256M4a,
}

impl ImportAudioMode {
    pub fn extension(self) -> &'static str {
        match self {
            Self::CopyWav | Self::Pcm16Wav => "wav",
            Self::CopyFlac => "flac",
            Self::CopyAacM4a | Self::Aac256M4a => "m4a",
        }
    }
}

/// Import only the first audio stream in the selected library format. AAC
/// MP4 outputs put their moov index first for seeking before download finishes.
pub async fn import_audio(
    input: &Path,
    output: &Path,
    mode: ImportAudioMode,
) -> Result<(), String> {
    match mode {
        ImportAudioMode::CopyWav => remux_first_audio(input, output, CopyAudioFormat::Wav).await,
        ImportAudioMode::CopyFlac => remux_first_audio(input, output, CopyAudioFormat::Flac).await,
        ImportAudioMode::CopyAacM4a => remux_first_audio(input, output, CopyAudioFormat::M4a).await,
        ImportAudioMode::Pcm16Wav => encode(input, output, EncodeProfile::Pcm16Wav).await,
        ImportAudioMode::Aac256M4a => encode(input, output, EncodeProfile::Aac256M4a).await,
    }
}
