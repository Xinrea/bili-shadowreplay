//! Read the linked SDK's identity and codec registry without launching tools.

use std::{ffi::CStr, path::Path, ptr, sync::Arc};

use ffmpeg_next::ffi;

/// FFmpeg's linked-library release identifier, independent of executables on PATH.
pub fn linked_ffmpeg_version() -> String {
    // SAFETY: FFmpeg returns a static, NUL-terminated release identifier.
    unsafe { CStr::from_ptr(ffi::av_version_info()) }
        .to_string_lossy()
        .into_owned()
}

/// Registered video encoders, sorted as in FFmpeg's encoder listing.
/// Registration does not imply that the encoder's hardware is available.
pub fn available_video_encoders() -> Vec<String> {
    let mut opaque = ptr::null_mut();
    let mut encoders = Vec::new();
    loop {
        // SAFETY: av_codec_iterate advances its opaque cursor and returns
        // static codec descriptors owned by the linked library.
        let codec = unsafe { ffi::av_codec_iterate(&mut opaque) };
        if codec.is_null() {
            break;
        }
        unsafe {
            if (*codec).type_ == ffi::AVMediaType::AVMEDIA_TYPE_VIDEO
                && ffi::av_codec_is_encoder(codec) != 0
                && !(*codec).name.is_null()
            {
                encoders.push(CStr::from_ptr((*codec).name).to_string_lossy().into_owned());
            }
        }
    }
    encoders.sort_unstable();
    encoders.dedup();
    encoders
}

/// Conservative header preflight for the SDR BT.709 GPU compositor. Unknown
/// tags retain the application's existing SDR interpretation. Explicit HDR or
/// other primaries must use a pipeline that performs their colour conversion.
/// The worker still validates actual filtered frames, including stream changes.
pub async fn video_is_sdr_bt709(path: &Path) -> Result<bool, String> {
    video_has_supported_sdr_transfer(path, false).await
}

/// Header preflight for the shared GPU compositor, which converts sRGB video
/// to BT.709 before blending. Actual filtered frames are validated separately.
pub async fn video_supports_gpu_sdr(path: &Path) -> Result<bool, String> {
    video_has_supported_sdr_transfer(path, true).await
}

async fn video_has_supported_sdr_transfer(path: &Path, allow_srgb: bool) -> Result<bool, String> {
    let path = path.to_owned();
    crate::metadata::run_media(move |cancelled| {
        let input = crate::metadata::open_input(&path, Arc::clone(&cancelled))?;
        for stream in input.streams() {
            let parameters = stream.parameters();
            if parameters.medium() != ffmpeg_next::media::Type::Video
                || stream
                    .disposition()
                    .contains(ffmpeg_next::format::stream::Disposition::ATTACHED_PIC)
            {
                continue;
            }
            let raw = unsafe { &*parameters.as_ptr() };
            match gpu_sdr_transfer(raw.color_primaries, raw.color_trc) {
                Some(SdrTransfer::Bt709) => {}
                Some(SdrTransfer::Srgb) if allow_srgb => {}
                _ => return Ok(false),
            }
        }
        Ok(true)
    })
    .await
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SdrTransfer {
    Bt709,
    Srgb,
}

pub(crate) fn gpu_sdr_transfer(
    primaries: ffi::AVColorPrimaries,
    transfer: ffi::AVColorTransferCharacteristic,
) -> Option<SdrTransfer> {
    if !matches!(
        primaries,
        ffi::AVColorPrimaries::AVCOL_PRI_UNSPECIFIED | ffi::AVColorPrimaries::AVCOL_PRI_BT709
    ) {
        return None;
    }
    match transfer {
        ffi::AVColorTransferCharacteristic::AVCOL_TRC_UNSPECIFIED
        | ffi::AVColorTransferCharacteristic::AVCOL_TRC_BT709 => Some(SdrTransfer::Bt709),
        ffi::AVColorTransferCharacteristic::AVCOL_TRC_IEC61966_2_1 => Some(SdrTransfer::Srgb),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linked_identity_and_registry_work_without_executables() {
        let version = linked_ffmpeg_version();
        assert!(!version.trim().is_empty());
        let names = available_video_encoders();
        assert!(!names.is_empty());
        assert!(names.windows(2).all(|pair| pair[0] < pair[1]));
        for name in names {
            let codec = ffmpeg_next::encoder::find_by_name(&name).unwrap();
            assert_eq!(codec.medium(), ffmpeg_next::media::Type::Video);
        }
    }

    #[test]
    fn gpu_colour_preflight_keeps_hdr_and_other_primaries_on_the_cpu_path() {
        use ffi::{AVColorPrimaries::*, AVColorTransferCharacteristic::*};
        assert_eq!(
            gpu_sdr_transfer(AVCOL_PRI_UNSPECIFIED, AVCOL_TRC_UNSPECIFIED),
            Some(SdrTransfer::Bt709)
        );
        assert_eq!(
            gpu_sdr_transfer(AVCOL_PRI_BT709, AVCOL_TRC_BT709),
            Some(SdrTransfer::Bt709)
        );
        assert_eq!(
            gpu_sdr_transfer(AVCOL_PRI_BT709, AVCOL_TRC_IEC61966_2_1),
            Some(SdrTransfer::Srgb)
        );
        assert_eq!(
            gpu_sdr_transfer(AVCOL_PRI_UNSPECIFIED, AVCOL_TRC_IEC61966_2_1),
            Some(SdrTransfer::Srgb)
        );
        assert_eq!(
            gpu_sdr_transfer(AVCOL_PRI_BT2020, AVCOL_TRC_SMPTE2084),
            None
        );
        assert_eq!(
            gpu_sdr_transfer(AVCOL_PRI_BT709, AVCOL_TRC_ARIB_STD_B67),
            None
        );
        assert_eq!(gpu_sdr_transfer(AVCOL_PRI_SMPTE170M, AVCOL_TRC_BT709), None);
        assert_eq!(
            gpu_sdr_transfer(AVCOL_PRI_BT2020, AVCOL_TRC_IEC61966_2_1),
            None
        );
    }
}
