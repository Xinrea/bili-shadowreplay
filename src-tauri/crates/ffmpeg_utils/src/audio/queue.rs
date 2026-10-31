//! Frame-size assembly outside libavfilter, preserving each natural block's
//! timestamp across format-graph replacement and discontinuities.

use ffmpeg_next::{self as ffmpeg, ffi, frame};
use std::collections::VecDeque;

#[derive(Default)]
pub(crate) struct AudioFrameQueue {
    blocks: VecDeque<Block>,
    samples: usize,
}
struct Block {
    frame: frame::Audio,
    offset: usize,
}

impl AudioFrameQueue {
    pub(crate) fn push(&mut self, frame: frame::Audio) -> Result<(), String> {
        self.samples = self
            .samples
            .checked_add(frame.samples())
            .ok_or("音频队列溢出")?;
        if frame.samples() != 0 {
            self.blocks.push_back(Block { frame, offset: 0 });
        }
        Ok(())
    }

    pub(crate) fn pop(&mut self, size: usize, eof: bool) -> Result<Option<frame::Audio>, String> {
        if self.samples == 0 || (!eof && self.samples < size) {
            return Ok(None);
        }
        if size == 0 {
            return Err("音频编码器帧长度无效".into());
        }
        let count = self.samples.min(size);
        let first = self.blocks.front().ok_or("音频队列为空")?;
        let mut output = frame::Audio::empty();
        unsafe {
            let result = ffi::av_frame_copy_props(output.as_mut_ptr(), first.frame.as_ptr());
            if result < 0 {
                return Err(format!(
                    "复制音频帧属性失败: {}",
                    ffmpeg::Error::from(result)
                ));
            }
            let raw = &mut *output.as_mut_ptr();
            raw.format = (*first.frame.as_ptr()).format;
            raw.sample_rate = first.frame.rate() as i32;
            raw.nb_samples = count as i32;
            raw.time_base = ffi::AVRational {
                num: 1,
                den: raw.sample_rate,
            };
            raw.pts = first
                .frame
                .pts()
                .ok_or("音频队列缺少时间戳")?
                .checked_add(first.offset as i64)
                .ok_or("音频时间戳溢出")?;
            raw.duration = count as i64;
            let result =
                ffi::av_channel_layout_copy(&mut raw.ch_layout, &(*first.frame.as_ptr()).ch_layout);
            if result < 0 {
                return Err(format!("复制音频声道失败: {}", ffmpeg::Error::from(result)));
            }
            let result = ffi::av_frame_get_buffer(output.as_mut_ptr(), 0);
            if result < 0 {
                return Err(format!("分配音频帧失败: {}", ffmpeg::Error::from(result)));
            }
        }
        let mut copied = 0;
        while copied < count {
            let first = self.blocks.front_mut().ok_or("音频队列为空")?;
            let take = (first.frame.samples() - first.offset).min(count - copied);
            let result = unsafe {
                ffi::av_samples_copy(
                    (*output.as_mut_ptr()).extended_data,
                    (*first.frame.as_ptr()).extended_data,
                    copied as i32,
                    first.offset as i32,
                    take as i32,
                    (*output.as_ptr()).ch_layout.nb_channels,
                    output.format().into(),
                )
            };
            if result < 0 {
                return Err(format!("复制音频样本失败: {}", ffmpeg::Error::from(result)));
            }
            copied += take;
            first.offset += take;
            if first.offset == first.frame.samples() {
                self.blocks.pop_front();
            }
        }
        self.samples -= count;
        Ok(Some(output))
    }
}
