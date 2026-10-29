//! Read the linked SDK's identity and codec registry without launching tools.

use std::{ffi::CStr, ptr};

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
}
