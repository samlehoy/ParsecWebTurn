//! macOS video: Annex B H.264/HEVC -> CMSampleBuffer -> AVSampleBufferDisplayLayer.
//! The layer decodes with VideoToolbox and presents through Core Animation, so
//! decoded pictures stay inside the system pipeline (no CPU readback here).
//! Whether VideoToolbox used the hardware decoder is not reported by the layer,
//! so `hardware_decode` stays unknown.
// ponytail: the layer owns decode and pacing; move to VTDecompressionSession + Metal
// if measured latency or frame pacing needs finer control.
use crate::video_output::{Queue, Snapshot};
use crate::video_stream::{Codec, PacketInfo};
use objc2::{msg_send, rc::Retained, runtime::AnyObject};
use std::{
    ffi::c_void,
    ptr,
    sync::{Arc, Mutex},
    time::Instant,
};

mod ffi {
    use std::ffi::c_void;
    pub type OSStatus = i32;

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct CGSize {
        pub width: f64,
        pub height: f64,
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        pub static kCFBooleanTrue: *const c_void;
        pub fn CFRelease(object: *const c_void);
        pub fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
        pub fn CFDictionarySetValue(dict: *const c_void, key: *const c_void, value: *const c_void);
    }

    #[link(name = "CoreMedia", kind = "framework")]
    extern "C" {
        pub static kCMSampleAttachmentKey_DisplayImmediately: *const c_void;
        pub fn CMVideoFormatDescriptionCreateFromH264ParameterSets(
            allocator: *const c_void,
            count: usize,
            pointers: *const *const u8,
            sizes: *const usize,
            nal_length_size: i32,
            out: *mut *mut c_void,
        ) -> OSStatus;
        pub fn CMVideoFormatDescriptionCreateFromHEVCParameterSets(
            allocator: *const c_void,
            count: usize,
            pointers: *const *const u8,
            sizes: *const usize,
            nal_length_size: i32,
            extensions: *const c_void,
            out: *mut *mut c_void,
        ) -> OSStatus;
        pub fn CMVideoFormatDescriptionGetPresentationDimensions(
            format: *mut c_void,
            use_pixel_aspect_ratio: u8,
            use_clean_aperture: u8,
        ) -> CGSize;
        pub fn CMBlockBufferCreateWithMemoryBlock(
            allocator: *const c_void,
            memory: *mut c_void,
            length: usize,
            block_allocator: *const c_void,
            custom_source: *const c_void,
            offset: usize,
            data_length: usize,
            flags: u32,
            out: *mut *mut c_void,
        ) -> OSStatus;
        pub fn CMBlockBufferReplaceDataBytes(
            source: *const c_void,
            destination: *mut c_void,
            offset: usize,
            length: usize,
        ) -> OSStatus;
        pub fn CMSampleBufferCreateReady(
            allocator: *const c_void,
            data: *mut c_void,
            format: *mut c_void,
            samples: isize,
            timing_entries: isize,
            timing: *const c_void,
            size_entries: isize,
            sizes: *const usize,
            out: *mut *mut c_void,
        ) -> OSStatus;
        pub fn CMSampleBufferGetSampleAttachmentsArray(
            sample: *mut c_void,
            create: u8,
        ) -> *const c_void;
    }

    #[link(name = "VideoToolbox", kind = "framework")]
    extern "C" {
        pub fn VTIsHardwareDecodeSupported(codec: u32) -> u8;
    }

    // Linking AVFoundation registers AVSampleBufferDisplayLayer with the runtime.
    #[link(name = "AVFoundation", kind = "framework")]
    extern "C" {
        pub static AVLayerVideoGravityResizeAspect: &'static objc2_foundation::NSString;
    }
}

const K_CM_BLOCK_BUFFER_ASSURE_MEMORY_NOW: u32 = 1;
const HEVC_CODEC_TYPE: u32 = u32::from_be_bytes(*b"hvc1");
/// CMSampleBufferRef as the Objective-C runtime encodes it, for msg_send checks.
#[repr(C)]
struct OpaqueSampleBuffer([u8; 0]);
unsafe impl objc2::encode::RefEncode for OpaqueSampleBuffer {
    const ENCODING_REF: objc2::encode::Encoding = objc2::encode::Encoding::Pointer(
        &objc2::encode::Encoding::Struct("opaqueCMSampleBuffer", &[]),
    );
}

/// AVQueuedSampleBufferRenderingStatusFailed.
const LAYER_FAILED: isize = 2;

/// Owned Core Foundation object, released on drop.
struct Cf(*mut c_void);
impl Drop for Cf {
    fn drop(&mut self) {
        unsafe { ffi::CFRelease(self.0) }
    }
}
// Core Media format descriptions and sample buffers are immutable once created.
unsafe impl Send for Cf {}

/// Annex B NAL units, without start codes or zero padding.
fn nal_units(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut offset = 0;
    std::iter::from_fn(move || {
        while bytes.get(offset) == Some(&0) {
            offset += 1;
        }
        if bytes.get(offset) != Some(&1) {
            return None;
        }
        let begin = offset + 1;
        let mut end = begin;
        while end < bytes.len() && !bytes[end..].starts_with(&[0, 0, 1]) {
            end += 1;
        }
        offset = end;
        let mut nal = &bytes[begin..end];
        while let [rest @ .., 0] = nal {
            nal = rest;
        }
        Some(nal)
    })
}

/// Parameter-set slot (VPS/SPS/PPS order) for a NAL, or None for picture data.
fn parameter_slot(codec: Codec, nal: &[u8]) -> Option<usize> {
    let header = *nal.first()?;
    match codec {
        Codec::H264 => match header & 31 {
            7 => Some(1),
            8 => Some(2),
            _ => None,
        },
        Codec::Hevc => match (header >> 1) & 63 {
            t @ 32..=34 => Some(usize::from(t - 32)),
            _ => None,
        },
    }
}

/// Turns Annex B packets into length-prefixed CMSampleBuffers, rebuilding the
/// format description whenever the stream's parameter sets change.
#[derive(Default)]
struct Converter {
    parameters: [Option<Vec<u8>>; 3],
    format: Option<Cf>,
    sample: Vec<u8>,
}

impl Converter {
    /// Ok(None): the packet carried only parameter sets, or no format exists yet.
    fn convert(&mut self, bytes: &[u8], codec: Codec) -> Result<Option<Cf>, String> {
        let mut changed = false;
        self.sample.clear();
        for nal in nal_units(bytes) {
            if nal.is_empty() {
                continue;
            }
            if let Some(slot) = parameter_slot(codec, nal) {
                if self.parameters[slot].as_deref() != Some(nal) {
                    self.parameters[slot] = Some(nal.to_vec());
                    changed = true;
                }
            } else {
                self.sample.extend_from_slice(
                    &u32::try_from(nal.len())
                        .map_err(|_| "nal size")?
                        .to_be_bytes(),
                );
                self.sample.extend_from_slice(nal);
            }
        }
        if changed {
            if let Some(format) = self.create_format(codec)? {
                self.format = Some(format);
            }
        }
        let Some(format) = &self.format else {
            return Ok(None);
        };
        if self.sample.is_empty() {
            return Ok(None);
        }
        unsafe {
            let mut block = ptr::null_mut();
            let status = ffi::CMBlockBufferCreateWithMemoryBlock(
                ptr::null(),
                ptr::null_mut(),
                self.sample.len(),
                ptr::null(),
                ptr::null(),
                0,
                self.sample.len(),
                K_CM_BLOCK_BUFFER_ASSURE_MEMORY_NOW,
                &mut block,
            );
            if status != 0 {
                return Err(format!("CMBlockBufferCreateWithMemoryBlock {status}"));
            }
            let block = Cf(block);
            let status = ffi::CMBlockBufferReplaceDataBytes(
                self.sample.as_ptr().cast(),
                block.0,
                0,
                self.sample.len(),
            );
            if status != 0 {
                return Err(format!("CMBlockBufferReplaceDataBytes {status}"));
            }
            let mut sample = ptr::null_mut();
            let size = self.sample.len();
            let status = ffi::CMSampleBufferCreateReady(
                ptr::null(),
                block.0,
                format.0,
                1,
                0,
                ptr::null(),
                1,
                &size,
                &mut sample,
            );
            if status != 0 {
                return Err(format!("CMSampleBufferCreateReady {status}"));
            }
            let sample = Cf(sample);
            // Live stream: show each picture as soon as it is decoded, no timeline.
            let attachments = ffi::CMSampleBufferGetSampleAttachmentsArray(sample.0, 1);
            if !attachments.is_null() {
                ffi::CFDictionarySetValue(
                    ffi::CFArrayGetValueAtIndex(attachments, 0),
                    ffi::kCMSampleAttachmentKey_DisplayImmediately,
                    ffi::kCFBooleanTrue,
                );
            }
            Ok(Some(sample))
        }
    }

    fn create_format(&self, codec: Codec) -> Result<Option<Cf>, String> {
        let first = match codec {
            Codec::H264 => 1,
            Codec::Hevc => 0,
        };
        let sets: Option<Vec<&[u8]>> = self.parameters[first..]
            .iter()
            .map(|p| p.as_deref())
            .collect();
        // Wait until the full parameter set is known; a partial set is not decodable.
        let Some(sets) = sets else {
            return Ok(None);
        };
        let pointers: Vec<*const u8> = sets.iter().map(|s| s.as_ptr()).collect();
        let sizes: Vec<usize> = sets.iter().map(|s| s.len()).collect();
        let mut format = ptr::null_mut();
        let status = unsafe {
            match codec {
                Codec::H264 => ffi::CMVideoFormatDescriptionCreateFromH264ParameterSets(
                    ptr::null(),
                    sets.len(),
                    pointers.as_ptr(),
                    sizes.as_ptr(),
                    4,
                    &mut format,
                ),
                Codec::Hevc => ffi::CMVideoFormatDescriptionCreateFromHEVCParameterSets(
                    ptr::null(),
                    sets.len(),
                    pointers.as_ptr(),
                    sizes.as_ptr(),
                    4,
                    ptr::null(),
                    &mut format,
                ),
            }
        };
        if status != 0 {
            return Err(format!("format description {status}"));
        }
        Ok(Some(Cf(format)))
    }

    fn dimensions(&self) -> Option<(u32, u32)> {
        let format = self.format.as_ref()?;
        let size =
            unsafe { ffi::CMVideoFormatDescriptionGetPresentationDimensions(format.0, 1, 1) };
        Some((size.width as u32, size.height as u32))
    }
}

/// Creates the display layer; called on the main thread by the window.
pub fn create_layer() -> Option<Retained<AnyObject>> {
    let class = objc2::runtime::AnyClass::get(c"AVSampleBufferDisplayLayer")?;
    let layer: Retained<AnyObject> = unsafe { msg_send![class, new] };
    let _: () =
        unsafe { msg_send![&*layer, setVideoGravity: ffi::AVLayerVideoGravityResizeAspect] };
    Some(layer)
}

struct State {
    queue: Queue,
    converter: Converter,
    shown: bool,
}

pub struct Pipeline {
    state: Mutex<State>,
    window: Arc<crate::window::Window>,
    generation: u64,
}

impl Pipeline {
    pub fn start(window: Arc<crate::window::Window>) -> Self {
        Self::start_mode(window, false)
    }

    #[cfg(feature = "bud")]
    pub fn start_bud(window: Arc<crate::window::Window>) -> Self {
        Self::start_mode(window, true)
    }

    fn start_mode(window: Arc<crate::window::Window>, bud_catch_up: bool) -> Self {
        let generation = window.begin_video();
        let mut queue = Queue::default();
        queue.bud_catch_up = bud_catch_up;
        queue.report.enabled = true;
        queue.report.decoder = Some("VideoToolbox (AVSampleBufferDisplayLayer)");
        queue.report.renderer = Some("Core Animation");
        queue.report.presentation_mode = Some("Display immediately");
        if window.video_layer() == 0 {
            queue.fail("video-layer-create", None);
        }
        Self {
            state: Mutex::new(State {
                queue,
                converter: Converter::default(),
                shown: false,
            }),
            window,
            generation,
        }
    }

    fn layer(&self) -> &AnyObject {
        // The window retains its layer for its whole lifetime, and we hold the window.
        unsafe { &*(self.window.video_layer() as *const AnyObject) }
    }

    pub fn submit(&self, bytes: &[u8], info: PacketInfo) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let state = &mut *state;
        let q = &mut state.queue;
        if q.stopped || q.report.failure_stage.is_some() {
            q.report.frames_dropped = q.report.frames_dropped.saturating_add(1);
            return false;
        }
        if q.report.codec.is_some_and(|codec| codec != info.codec) {
            q.fail("video-unexpected-codec-change", None);
            return false;
        }
        q.report.codec = Some(info.codec);

        let layer = self.layer();
        let status: isize = unsafe { msg_send![layer, status] };
        if status == LAYER_FAILED {
            let error: Option<Retained<AnyObject>> = unsafe { msg_send![layer, error] };
            let code = error
                .map(|e| unsafe { msg_send![&*e, code] })
                .unwrap_or(0isize);
            crate::event_log::record(
                "WARN",
                "video_layer_failed",
                serde_json::json!({"code": code}),
            );
            let _: () = unsafe { msg_send![layer, flush] };
            // The decoder lost its references; resume from the next key picture.
            q.recover(Instant::now());
            q.report.frames_dropped = q.report.frames_dropped.saturating_add(1);
            return false;
        }

        if q.report.waiting_for_idr && !info.idr && !info.parameters {
            q.report.frames_dropped = q.report.frames_dropped.saturating_add(1);
            return false;
        }
        let sample = match state.converter.convert(bytes, info.codec) {
            Ok(sample) => sample,
            Err(error) => {
                q.fail("video-sample-create", Some(error));
                return false;
            }
        };
        if state.converter.format.is_some() {
            q.report.decoder_initialized = true;
            if let Some((width, height)) = state.converter.dimensions() {
                q.report.width = Some(width);
                q.report.height = Some(height);
            }
        }
        // Parameter sets alone never end the wait for a key picture.
        if q.report.waiting_for_idr && !info.idr {
            return true;
        }
        let Some(sample) = sample else {
            return true;
        };
        if info.idr {
            q.report.waiting_for_idr = false;
            if q.report.recovering {
                q.recovered();
            }
        }
        let _: () =
            unsafe { msg_send![layer, enqueueSampleBuffer: sample.0.cast::<OpaqueSampleBuffer>()] };
        q.report.frames_submitted = q.report.frames_submitted.saturating_add(1);
        if !state.shown {
            state.shown = true;
            self.window.set_video_ready_for(self.generation, true);
        }
        *self
            .window
            .video_report
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(q.report.clone());
        true
    }

    pub fn stop(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.queue.stopped {
            return;
        }
        state.queue.stopped = true;
        state.queue.clear();
        state.converter = Converter::default();
        state.queue.report.worker_finished = true;
        state.queue.report.resources_released = true;
        *self
            .window
            .video_report
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(state.queue.report.clone());
        drop(state);
        self.window.finish_video(self.generation);
        let window = self.window.clone();
        dispatch2::DispatchQueue::main().exec_async(move || {
            let layer = window.video_layer();
            if layer != 0 {
                let _: () =
                    unsafe { msg_send![&*(layer as *const AnyObject), flushAndRemoveImage] };
            }
        });
    }

    pub fn take_keyframe_request(&self) -> Option<u64> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .queue
            .take_keyframe_request(Instant::now())
            .map(|_| state.queue.reset_generation)
    }

    pub fn recovery_request_failed(&self, generation: u64) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .queue
            .request_failed(generation);
    }

    pub fn snapshot(&self) -> Snapshot {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .queue
            .report
            .clone()
    }
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn hevc_available(_gpu: Option<&crate::connection_settings::GpuPreference>) -> bool {
    unsafe { ffi::VTIsHardwareDecodeSupported(HEVC_CODEC_TYPE) != 0 }
}

/// Main 10 support is not queryable without a trial decode, so it is not offered.
pub fn hevc_ten_bit_available(_gpu: Option<&crate::connection_settings::GpuPreference>) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CMTime {
        value: i64,
        timescale: i32,
        flags: u32,
        epoch: i64,
    }
    type Callback = extern "C" fn(*mut c_void, *mut c_void, i32, u32, *mut c_void, CMTime, CMTime);
    #[repr(C)]
    struct CallbackRecord {
        callback: Callback,
        refcon: *mut c_void,
    }
    #[link(name = "VideoToolbox", kind = "framework")]
    extern "C" {
        fn VTDecompressionSessionCreate(
            allocator: *const c_void,
            format: *mut c_void,
            decoder_specification: *const c_void,
            image_attributes: *const c_void,
            callback: *const CallbackRecord,
            out: *mut *mut c_void,
        ) -> i32;
        fn VTDecompressionSessionDecodeFrame(
            session: *mut c_void,
            sample: *mut c_void,
            flags: u32,
            frame_refcon: *mut c_void,
            info_out: *mut u32,
        ) -> i32;
        fn VTDecompressionSessionWaitForAsynchronousFrames(session: *mut c_void) -> i32;
        fn VTDecompressionSessionInvalidate(session: *mut c_void);
    }

    extern "C" fn decoded(
        refcon: *mut c_void,
        _frame: *mut c_void,
        status: i32,
        _flags: u32,
        image: *mut c_void,
        _pts: CMTime,
        _duration: CMTime,
    ) {
        if status == 0 && !image.is_null() {
            unsafe { *(refcon as *mut u32) += 1 };
        }
    }

    /// Feeds converted samples to VideoToolbox, the decoder behind the display
    /// layer, and returns how many pictures came out.
    fn decode(units: &[&[u8]], codec: Codec) -> u32 {
        let mut converter = Converter::default();
        let mut session: *mut c_void = ptr::null_mut();
        let mut pictures = 0u32;
        let record = CallbackRecord {
            callback: decoded,
            refcon: (&mut pictures as *mut u32).cast(),
        };
        for unit in units {
            let Some(sample) = converter.convert(unit, codec).unwrap() else {
                continue;
            };
            if session.is_null() {
                let format = converter.format.as_ref().unwrap().0;
                let status = unsafe {
                    VTDecompressionSessionCreate(
                        ptr::null(),
                        format,
                        ptr::null(),
                        ptr::null(),
                        &record,
                        &mut session,
                    )
                };
                assert_eq!(status, 0, "VTDecompressionSessionCreate");
            }
            let status = unsafe {
                VTDecompressionSessionDecodeFrame(
                    session,
                    sample.0,
                    0,
                    ptr::null_mut(),
                    ptr::null_mut(),
                )
            };
            assert_eq!(status, 0, "VTDecompressionSessionDecodeFrame");
        }
        unsafe {
            VTDecompressionSessionWaitForAsynchronousFrames(session);
            VTDecompressionSessionInvalidate(session);
            ffi::CFRelease(session);
        }
        pictures
    }

    #[test]
    fn h264_fixture_samples_decode() {
        assert_eq!(decode(&h264_units(), Codec::H264), 8);
    }

    fn h264_units() -> Vec<&'static [u8]> {
        let fixture: &'static [u8] = include_bytes!("../fixtures/synthetic-1920x1080.h264");
        // Split on access unit delimiters, as the Windows probe does.
        let mut starts: Vec<usize> = (0..fixture.len() - 4)
            .filter(|&i| fixture[i..].starts_with(&[0, 0, 0, 1]) && fixture[i + 4] & 31 == 9)
            .collect();
        assert_eq!(starts.len(), 8);
        starts.push(fixture.len());
        starts.windows(2).map(|w| &fixture[w[0]..w[1]]).collect()
    }

    /// Exercises the exact layer messages the pipeline sends (selectors and
    /// argument encodings); a wrong one raises instead of returning.
    #[test]
    fn display_layer_accepts_samples() {
        let layer = create_layer().expect("AVSampleBufferDisplayLayer");
        let mut converter = Converter::default();
        for unit in h264_units() {
            if let Some(sample) = converter.convert(unit, Codec::H264).unwrap() {
                let _: () = unsafe {
                    msg_send![&*layer, enqueueSampleBuffer: sample.0.cast::<OpaqueSampleBuffer>()]
                };
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
        let status: isize = unsafe { msg_send![&*layer, status] };
        let error: Option<Retained<AnyObject>> = unsafe { msg_send![&*layer, error] };
        assert_ne!(status, LAYER_FAILED, "{error:?}");
        let _: () = unsafe { msg_send![&*layer, flush] };
        let _: () = unsafe { msg_send![&*layer, flushAndRemoveImage] };
    }

    #[test]
    fn hevc_main10_fixture_samples_decode() {
        let mut fixture: &[u8] = include_bytes!("../fixtures/synthetic-720p-hevc10.bin");
        let mut units = Vec::new();
        while !fixture.is_empty() {
            let len = u32::from_le_bytes(fixture[..4].try_into().unwrap()) as usize;
            units.push(&fixture[4..4 + len]);
            fixture = &fixture[4 + len..];
        }
        let mut converter = Converter::default();
        converter.convert(units[0], Codec::Hevc).unwrap();
        assert_eq!(converter.dimensions(), Some((1280, 720)));
        assert_eq!(decode(&units, Codec::Hevc), units.len() as u32);
    }

    #[test]
    fn annex_b_split_drops_start_codes_and_padding() {
        let bytes = [
            0, 0, 0, 1, 0x67, 1, 2, 0, 0, 0, 1, 0x68, 3, 0, 0, 1, 0x65, 4,
        ];
        let nals: Vec<&[u8]> = nal_units(&bytes).collect();
        assert_eq!(nals, [&[0x67, 1, 2][..], &[0x68, 3], &[0x65, 4]]);
    }
}
