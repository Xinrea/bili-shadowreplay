//! Minimal safe Rust bindings for whisper.cpp.
//!
//! Only exposes the subset of the API that this project needs:
//! model loading, state creation, parameter configuration, inference,
//! and segment-level result retrieval.

#![allow(non_camel_case_types, dead_code)]

use std::ffi::{c_char, c_float, c_int, CStr, CString};
use std::ptr;

// ── Token-level types ──────────────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct WhisperTokenData {
    id: c_int,
    tid: c_int,
    p: f32,
    plog: f32,
    pt: f32,
    ptsum: f32,
    t0: i64,
    t1: i64,
    t_dtw: i64,
    vlen: f32,
}

/// Public-safe token data used for building precise subtitles.
#[derive(Clone, Debug)]
pub struct TokenData {
    /// Token text (may be a special token like [_BEG_], [_TT_150], etc.)
    pub text: String,
    /// Start time in centiseconds
    pub t0: i64,
    /// End time in centiseconds
    pub t1: i64,
    /// Probability of this token
    pub p: f32,
}

// ── FFI declarations ───────────────────────────────────────────────

// whisper_context_params is ~32+ bytes in whisper.h v1.7.4.
// We use an opaque buffer to avoid mismatched struct definitions.
// use_gpu is at byte offset 0 (bool = 1 byte).
#[repr(C)]
struct WhisperContextParams([u8; 256]);

extern "C" {
    fn whisper_context_default_params_by_ref() -> *mut WhisperContextParams;
}

#[repr(C)]
#[allow(non_camel_case_types)]
enum whisper_sampling_strategy {
    WHISPER_SAMPLING_GREEDY = 0,
    WHISPER_SAMPLING_BEAM_SEARCH = 1,
}

#[repr(C)]
#[allow(non_camel_case_types)]
struct whisper_full_params {
    _opaque: [u8; 1024], // opaque; we only ever hold a pointer from whisper
}

type whisper_progress_callback =
    unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void, c_int, *mut std::ffi::c_void);

extern "C" {
    fn whisper_init_from_file_with_params(
        path_model: *const c_char,
        params: WhisperContextParams,
    ) -> *mut std::ffi::c_void;

    fn whisper_init_state(ctx: *mut std::ffi::c_void) -> *mut std::ffi::c_void;

    fn whisper_free(ctx: *mut std::ffi::c_void);
    fn whisper_free_state(state: *mut std::ffi::c_void);
    fn whisper_free_params(params: *mut std::ffi::c_void);

    fn whisper_full_default_params_by_ref(
        strategy: whisper_sampling_strategy,
    ) -> *mut whisper_full_params;

    fn whisper_full(
        ctx: *mut std::ffi::c_void,
        params: *mut whisper_full_params,
        samples: *const f32,
        n_samples: c_int,
    ) -> c_int;

    fn whisper_full_with_state(
        ctx: *mut std::ffi::c_void,
        state: *mut std::ffi::c_void,
        params: *mut whisper_full_params,
        samples: *const f32,
        n_samples: c_int,
    ) -> c_int;

    fn whisper_full_n_segments_from_state(state: *mut std::ffi::c_void) -> c_int;

    fn whisper_full_get_segment_t0_from_state(
        state: *mut std::ffi::c_void,
        i_segment: c_int,
    ) -> i64;

    fn whisper_full_get_segment_t1_from_state(
        state: *mut std::ffi::c_void,
        i_segment: c_int,
    ) -> i64;

    fn whisper_full_get_segment_text_from_state(
        state: *mut std::ffi::c_void,
        i_segment: c_int,
    ) -> *const c_char;

    fn whisper_full_n_tokens_from_state(
        state: *mut std::ffi::c_void,
        i_segment: c_int,
    ) -> c_int;

    fn whisper_full_get_token_text_from_state(
        ctx: *mut std::ffi::c_void,
        state: *mut std::ffi::c_void,
        i_segment: c_int,
        i_token: c_int,
    ) -> *const c_char;

    fn whisper_full_get_token_data_from_state(
        state: *mut std::ffi::c_void,
        i_segment: c_int,
        i_token: c_int,
    ) -> WhisperTokenData;

    fn whisper_print_system_info() -> *const c_char;
    fn whisper_rs_params_set_greedy_best_of(params: *mut whisper_full_params, best_of: c_int);
    fn whisper_rs_params_set_beam_search(
        params: *mut whisper_full_params,
        beam_size: c_int,
        patience: c_float,
    );
    fn whisper_rs_params_set_print_special(params: *mut whisper_full_params, value: bool);
    fn whisper_rs_params_set_print_progress(params: *mut whisper_full_params, value: bool);
    fn whisper_rs_params_set_print_realtime(params: *mut whisper_full_params, value: bool);
    fn whisper_rs_params_set_print_timestamps(params: *mut whisper_full_params, value: bool);
    fn whisper_rs_params_set_token_timestamps(params: *mut whisper_full_params, value: bool);
    fn whisper_rs_params_set_max_len(params: *mut whisper_full_params, value: c_int);
    fn whisper_rs_params_set_language(params: *mut whisper_full_params, value: *const c_char);
    fn whisper_rs_params_set_initial_prompt(params: *mut whisper_full_params, value: *const c_char);
}

// ── Public types ────────────────────────────────────────────────────

/// Audio sampling strategy for whisper.
#[derive(Clone, Copy)]
pub enum SamplingStrategy {
    Greedy { best_of: i32 },
    BeamSearch { beam_size: i32, patience: f32 },
}

/// Thread-safe context holding the loaded whisper model.
pub struct WhisperContext {
    ctx: *mut std::ffi::c_void,
}

unsafe impl Send for WhisperContext {}
unsafe impl Sync for WhisperContext {}

/// A whisper processing state, created from a context.
/// NOT Send/Sync — one state per inference session.
pub struct WhisperState {
    state: *mut std::ffi::c_void,
}

// SAFETY: WhisperState is used exclusively within a single async task
// and never shared across threads. The raw pointer is only accessed
// sequentially from the owning task.
unsafe impl Send for WhisperState {}

/// Full inference parameters.
pub struct FullParams {
    params: *mut whisper_full_params,
    // Keep CStrings alive for the lifetime of params
    _language: Option<CString>,
    _prompt: Option<CString>,
}

unsafe impl Send for FullParams {}

impl Drop for WhisperContext {
    fn drop(&mut self) {
        if !self.ctx.is_null() {
            unsafe { whisper_free(self.ctx) };
        }
    }
}

impl Drop for WhisperState {
    fn drop(&mut self) {
        if !self.state.is_null() {
            unsafe { whisper_free_state(self.state) };
        }
    }
}

impl Drop for FullParams {
    fn drop(&mut self) {
        if !self.params.is_null() {
            unsafe { whisper_free_params(self.params as *mut std::ffi::c_void) };
        }
    }
}

// ── impl WhisperContext ─────────────────────────────────────────────

impl WhisperContext {
    /// Load a whisper model from the given file path.
    pub fn new(model_path: &str) -> Result<Self, String> {
        let path = CString::new(model_path).map_err(|e| format!("Invalid path: {e}"))?;

        let ctx = unsafe {
            let params_ptr = whisper_context_default_params_by_ref();
            if params_ptr.is_null() {
                return Err("Failed to get default context params".to_string());
            }
            // Read the full struct (256 opaque bytes) to get all defaults,
            // then enable GPU at byte offset 0 (use_gpu).
            let mut params: WhisperContextParams = ptr::read(params_ptr);
            params.0[0] = 1; // use_gpu = true

            whisper_init_from_file_with_params(path.as_ptr(), params)
        };

        if ctx.is_null() {
            return Err(format!("Failed to load whisper model from {model_path}"));
        }

        Ok(Self { ctx })
    }

    /// Create a new processing state.
    pub fn create_state(&self) -> Result<WhisperState, String> {
        let state = unsafe { whisper_init_state(self.ctx) };
        if state.is_null() {
            return Err("Failed to create whisper state".to_string());
        }
        Ok(WhisperState { state })
    }
}

// ── impl FullParams ─────────────────────────────────────────────────

impl FullParams {
    /// Create default parameters with the given sampling strategy.
    pub fn new(strategy: SamplingStrategy) -> Self {
        let s = match strategy {
            SamplingStrategy::Greedy { .. } => {
                whisper_sampling_strategy::WHISPER_SAMPLING_GREEDY
            }
            SamplingStrategy::BeamSearch { .. } => {
                whisper_sampling_strategy::WHISPER_SAMPLING_BEAM_SEARCH
            }
        };

        let params = unsafe { whisper_full_default_params_by_ref(s) };
        assert!(!params.is_null(), "whisper_full_default_params_by_ref returned null");

        unsafe {
            match strategy {
                SamplingStrategy::Greedy { best_of } => {
                    whisper_rs_params_set_greedy_best_of(params, best_of.max(1));
                }
                SamplingStrategy::BeamSearch {
                    beam_size,
                    patience,
                } => {
                    whisper_rs_params_set_beam_search(params, beam_size.max(1), patience);
                }
            }
        }

        Self {
            params,
            _language: None,
            _prompt: None,
        }
    }

    /// Set the language hint (e.g. "zh", "en"). Pass None or "auto" for auto-detect.
    pub fn set_language(&mut self, lang: Option<&str>) {
        let cstr = lang.and_then(|l| {
            if l.is_empty() || l == "auto" {
                None
            } else {
                CString::new(l).ok()
            }
        });
        if let Some(ref c) = cstr {
            unsafe { whisper_rs_params_set_language(self.params, c.as_ptr()) };
        } else {
            unsafe { whisper_rs_params_set_language(self.params, ptr::null()) };
        }
        self._language = cstr;
    }

    /// Set the initial prompt to guide vocabulary and style.
    pub fn set_initial_prompt(&mut self, prompt: &str) {
        let cstr = CString::new(prompt).ok();
        if let Some(ref c) = cstr {
            unsafe { whisper_rs_params_set_initial_prompt(self.params, c.as_ptr()) };
        } else {
            unsafe { whisper_rs_params_set_initial_prompt(self.params, ptr::null()) };
        }
        self._prompt = cstr;
    }

    /// Whether to print special tokens to stdout.
    pub fn set_print_special(&mut self, v: bool) {
        unsafe { whisper_rs_params_set_print_special(self.params, v) };
    }

    /// Whether to print progress to stdout.
    pub fn set_print_progress(&mut self, v: bool) {
        unsafe { whisper_rs_params_set_print_progress(self.params, v) };
    }

    /// Whether to print realtime results to stdout.
    pub fn set_print_realtime(&mut self, v: bool) {
        unsafe { whisper_rs_params_set_print_realtime(self.params, v) };
    }

    /// Whether to print timestamps to stdout.
    pub fn set_print_timestamps(&mut self, v: bool) {
        unsafe { whisper_rs_params_set_print_timestamps(self.params, v) };
    }

    /// Enable per-token timestamps for more precise segment boundaries.
    pub fn set_token_timestamps(&mut self, v: bool) {
        unsafe { whisper_rs_params_set_token_timestamps(self.params, v) };
    }

    /// Maximum segment length in characters. 0 = no limit.
    /// Setting this forces whisper to split into shorter segments,
    /// improving timestamp granularity for subtitle display.
    pub fn set_max_len(&mut self, max_len: i32) {
        unsafe { whisper_rs_params_set_max_len(self.params, max_len) };
    }
}

// ── impl WhisperState ────────────────────────────────────────────────

impl WhisperState {
    /// Run inference on the given audio samples (16kHz mono f32 PCM).
    pub fn full(
        &mut self,
        ctx: &WhisperContext,
        params: &FullParams,
        samples: &[f32],
    ) -> Result<i32, String> {
        let ret = unsafe {
            whisper_full_with_state(
                ctx.ctx,
                self.state,
                params.params,
                samples.as_ptr(),
                samples.len() as c_int,
            )
        };
        if ret != 0 {
            Err(format!("whisper_full failed with code {ret}"))
        } else {
            Ok(ret)
        }
    }

    /// Number of transcribed segments.
    pub fn full_n_segments(&self) -> i32 {
        unsafe { whisper_full_n_segments_from_state(self.state) }
    }

    /// Get segment start timestamp in centiseconds (divide by 100.0 for seconds).
    pub fn get_segment_t0(&self, index: i32) -> i64 {
        unsafe { whisper_full_get_segment_t0_from_state(self.state, index) }
    }

    /// Get segment end timestamp in centiseconds (divide by 100.0 for seconds).
    pub fn get_segment_t1(&self, index: i32) -> i64 {
        unsafe { whisper_full_get_segment_t1_from_state(self.state, index) }
    }

    /// Get segment text. Returns None if index is out of range.
    pub fn get_segment_text(&self, index: i32) -> Option<String> {
        unsafe {
            let ptr = whisper_full_get_segment_text_from_state(self.state, index);
            if ptr.is_null() {
                None
            } else {
                Some(CStr::from_ptr(ptr).to_string_lossy().into_owned())
            }
        }
    }

    /// Number of tokens in a given segment.
    pub fn full_n_tokens(&self, i_segment: i32) -> i32 {
        unsafe { whisper_full_n_tokens_from_state(self.state, i_segment) }
    }

    /// Get all tokens for a segment with their text, timestamps, and
    /// probabilities.  Special tokens (`[_BEG_]`, `[_TT_*]`, etc.)
    /// are included — the caller should filter them.
    pub fn get_segment_tokens(
        &self,
        ctx: &WhisperContext,
        i_segment: i32,
    ) -> Vec<TokenData> {
        let n = self.full_n_tokens(i_segment);
        let mut tokens = Vec::with_capacity(n as usize);
        for i in 0..n {
            let text = unsafe {
                let ptr = whisper_full_get_token_text_from_state(
                    ctx.ctx,
                    self.state,
                    i_segment,
                    i,
                );
                if ptr.is_null() {
                    String::new()
                } else {
                    CStr::from_ptr(ptr).to_string_lossy().into_owned()
                }
            };
            let data =
                unsafe { whisper_full_get_token_data_from_state(self.state, i_segment, i) };
            tokens.push(TokenData {
                text,
                t0: data.t0,
                t1: data.t1,
                p: data.p,
            });
        }
        tokens
    }
}

// ── Audio conversion utilities ──────────────────────────────────────

/// Convert i16 PCM samples to f32 in range [-1.0, 1.0].
pub fn convert_integer_to_float_audio(
    input: &[i16],
    output: &mut [f32],
) -> Result<(), String> {
    if input.len() != output.len() {
        return Err(format!(
            "Input and output must have the same length: {} != {}",
            input.len(),
            output.len()
        ));
    }
    for (i, sample) in input.iter().enumerate() {
        output[i] = *sample as f32 / 32768.0;
    }
    Ok(())
}

/// Convert interleaved stereo f32 samples to mono by averaging channels.
/// Output length must be half of input length.
pub fn convert_stereo_to_mono_audio(
    input: &[f32],
    output: &mut [f32],
) -> Result<(), String> {
    if input.len() != output.len() * 2 {
        return Err(format!(
            "Input length ({}) must be 2x output length ({})",
            input.len(),
            output.len()
        ));
    }
    for i in 0..output.len() {
        output[i] = (input[i * 2] + input[i * 2 + 1]) * 0.5;
    }
    Ok(())
}

/// Print system info (GPU, CPU features, etc.) to stdout.
pub fn print_system_info() -> String {
    unsafe {
        let ptr = whisper_print_system_info();
        if ptr.is_null() {
            "unknown".to_string()
        } else {
            CStr::from_ptr(ptr).to_string_lossy().into_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_integer_to_float() {
        let input: Vec<i16> = vec![0, 16384, -16384, 32767, -32768];
        let mut output = vec![0.0f32; 5];
        convert_integer_to_float_audio(&input, &mut output).unwrap();
        assert!((output[0] - 0.0).abs() < 0.001);
        assert!((output[1] - 0.5).abs() < 0.001);
        assert!((output[2] + 0.5).abs() < 0.001);
        assert!((output[3] - 0.99997).abs() < 0.001);
        assert!((output[4] + 1.0).abs() < 0.001);
    }

    #[test]
    fn test_convert_stereo_to_mono() {
        let input: Vec<f32> = vec![1.0, 0.0, 0.5, 0.5, -1.0, 1.0];
        let mut output = vec![0.0f32; 3];
        convert_stereo_to_mono_audio(&input, &mut output).unwrap();
        assert!((output[0] - 0.5).abs() < 0.001); // (1.0 + 0.0) / 2
        assert!((output[1] - 0.5).abs() < 0.001); // (0.5 + 0.5) / 2
        assert!((output[2] - 0.0).abs() < 0.001); // (-1.0 + 1.0) / 2
    }
}
