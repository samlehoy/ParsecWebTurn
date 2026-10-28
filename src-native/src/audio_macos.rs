//! macOS CoreAudio AudioUnit Output.
use crate::audio_layout::Layout;
use anyhow::{bail, Result};
use serde::Serialize;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
};

#[derive(Clone, Default, Serialize)]
pub struct Snapshot {
    pub layout: Option<Layout>,
    pub minimum_ms: u32,
    pub maximum_ms: u32,
    pub custom_buffer: bool,
    pub low_latency_device: bool,
    pub device_capacity_frames: u32,
    pub device_opened: bool,
    pub frames_queued: u64,
    pub frames_written: u64,
    pub frames_dropped: u64,
    pub underruns: u64,
    pub queued_frames: usize,
    pub device_padding: u32,
    pub failure_hresult: Option<String>,
    pub resources_released: bool,
}

#[derive(Default)]
pub struct Registry {
    next: u32,
    outputs: BTreeMap<u32, Arc<Playback>>,
    completed: VecDeque<Snapshot>,
    rejected_layouts: BTreeMap<u32, Layout>,
}

impl Registry {
    #[cfg(feature = "diagnostics")]
    pub fn snapshots(&self) -> Vec<Snapshot> {
        self.completed
            .iter()
            .cloned()
            .chain(self.outputs.values().map(|v| v.snapshot()))
            .collect()
    }
    pub fn retain_snapshot(&mut self, snapshot: Snapshot) {
        if self.completed.len() == 8 {
            self.completed.pop_front();
        }
        self.completed.push_back(snapshot);
    }
    pub fn insert(&mut self, output: Playback) -> Result<u32> {
        if self.outputs.len() >= 4 {
            bail!("audio context limit");
        }
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("audio handle overflow"))?;
        self.outputs.insert(self.next, Arc::new(output));
        Ok(self.next)
    }
    pub fn get(&self, handle: u32) -> Result<Arc<Playback>> {
        self.outputs
            .get(&handle)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("invalid audio context"))
    }
    pub fn remove(&mut self, handle: u32) -> Result<Arc<Playback>> {
        self.rejected_layouts.remove(&handle);
        self.outputs
            .remove(&handle)
            .ok_or_else(|| anyhow::anyhow!("invalid audio context"))
    }
    pub fn rejected(&self, handle: u32, layout: Layout) -> bool {
        self.rejected_layouts.get(&handle) == Some(&layout)
    }
    pub fn reject(&mut self, handle: u32, layout: Layout) {
        self.rejected_layouts.insert(handle, layout);
    }
    pub fn replace(&mut self, handle: u32, output: Playback) -> Result<Arc<Playback>> {
        let slot = self
            .outputs
            .get_mut(&handle)
            .ok_or_else(|| anyhow::anyhow!("invalid audio context"))?;
        let old = std::mem::replace(slot, Arc::new(output));
        self.rejected_layouts.remove(&handle);
        Ok(old)
    }
}

#[derive(Default)]
struct State {
    report: Snapshot,
    pcm: VecDeque<i16>,
    stopped: bool,
    reset: bool,
}

pub struct Playback {
    pub layout: Layout,
    shared: Arc<Mutex<State>>,
    audio_unit: *mut std::ffi::c_void,
    max_frames: usize,
    custom_buffer: bool,
}

unsafe impl Send for Playback {}
unsafe impl Sync for Playback {}

impl Playback {
    pub fn create(
        window: Arc<crate::window::Window>,
        min_buffer: u32,
        max_buffer: u32,
    ) -> Result<Self> {
        Self::create_layout(window, min_buffer, max_buffer, Layout::STEREO)
    }

    pub fn create_layout(
        window: Arc<crate::window::Window>,
        min_buffer: u32,
        max_buffer: u32,
        layout: Layout,
    ) -> Result<Self> {
        let custom = window.audio_buffer();
        let (min_buffer, max_buffer) = custom.unwrap_or((min_buffer, max_buffer));
        if min_buffer > max_buffer || max_buffer == 0 || max_buffer > 500 {
            bail!("invalid audio latency limits");
        }
        let max_frames = max_buffer as usize * 48;
        let shared = Arc::new(Mutex::new(State {
            report: Snapshot {
                layout: Some(layout),
                minimum_ms: min_buffer,
                maximum_ms: max_buffer,
                custom_buffer: custom.is_some(),
                device_opened: true,
                ..Default::default()
            },
            ..Default::default()
        }));

        let au = unsafe { setup_audio_unit(&shared, layout.channels as u32)? };

        Ok(Self {
            layout,
            shared,
            audio_unit: au,
            max_frames,
            custom_buffer: custom.is_some(),
        })
    }

    pub fn queue(&self, samples: &[i16]) {
        let mut state = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        if state.stopped {
            return;
        }
        let channels = self.layout.channels as usize;
        let incoming_frames = samples.len() / channels;
        let current_frames = state.pcm.len() / channels;
        if current_frames + incoming_frames > self.max_frames {
            state.report.frames_dropped += incoming_frames as u64;
            return;
        }
        state.pcm.extend(samples.iter().copied());
        state.report.frames_queued += incoming_frames as u64;
        state.report.queued_frames = state.pcm.len() / channels;
    }

    pub fn reset(&self) {
        let mut state = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        state.pcm.clear();
        state.reset = true;
        state.report.queued_frames = 0;
    }

    pub fn queued_ms(&self) -> u32 {
        let state = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        ((state.pcm.len() / self.layout.channels as usize) / 48) as u32
    }

    pub fn queued_frames(&self) -> u32 {
        let state = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        (state.pcm.len() / self.layout.channels as usize) as u32
    }

    pub fn snapshot(&self) -> Snapshot {
        let mut report = self
            .shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .report
            .clone();
        report.custom_buffer = self.custom_buffer;
        report
    }

    pub fn close(&self) {
        {
            let mut state = self.shared.lock().unwrap_or_else(|e| e.into_inner());
            // stop() then Drop both reach here; dispose the unit only once.
            if state.stopped {
                return;
            }
            state.stopped = true;
            state.pcm.clear();
            state.report.resources_released = true;
        }
        // The render callback takes the same lock, so stop the unit without holding it.
        if !self.audio_unit.is_null() {
            unsafe {
                coreaudio_sys::AudioOutputUnitStop(self.audio_unit as _);
                coreaudio_sys::AudioComponentInstanceDispose(self.audio_unit as _);
            }
        }
    }

    pub fn stop(&self) {
        self.close();
    }
}

impl Drop for Playback {
    fn drop(&mut self) {
        self.close();
    }
}

unsafe fn setup_audio_unit(
    state: &Arc<Mutex<State>>,
    channels: u32,
) -> Result<*mut std::ffi::c_void> {
    let desc = coreaudio_sys::AudioComponentDescription {
        componentType: coreaudio_sys::kAudioUnitType_Output,
        componentSubType: coreaudio_sys::kAudioUnitSubType_DefaultOutput,
        componentManufacturer: coreaudio_sys::kAudioUnitManufacturer_Apple,
        componentFlags: 0,
        componentFlagsMask: 0,
    };
    let comp = coreaudio_sys::AudioComponentFindNext(std::ptr::null_mut(), &desc);
    if comp.is_null() {
        bail!("Default audio output component not found");
    }
    let mut au: coreaudio_sys::AudioComponentInstance = std::ptr::null_mut();
    let status = coreaudio_sys::AudioComponentInstanceNew(comp, &mut au);
    if status != 0 || au.is_null() {
        bail!("AudioComponentInstanceNew failed: {status}");
    }

    let mut stream_desc = coreaudio_sys::AudioStreamBasicDescription {
        mSampleRate: 48000.0,
        mFormatID: coreaudio_sys::kAudioFormatLinearPCM,
        mFormatFlags: coreaudio_sys::kAudioFormatFlagIsSignedInteger
            | coreaudio_sys::kAudioFormatFlagIsPacked,
        mBytesPerPacket: channels * 2,
        mFramesPerPacket: 1,
        mBytesPerFrame: channels * 2,
        mChannelsPerFrame: channels,
        mBitsPerChannel: 16,
        mReserved: 0,
    };

    let status = coreaudio_sys::AudioUnitSetProperty(
        au,
        coreaudio_sys::kAudioUnitProperty_StreamFormat,
        coreaudio_sys::kAudioUnitScope_Input,
        0,
        &mut stream_desc as *mut _ as *const _,
        std::mem::size_of_val(&stream_desc) as u32,
    );
    if status != 0 {
        coreaudio_sys::AudioComponentInstanceDispose(au);
        bail!("AudioUnitSetProperty StreamFormat failed: {status}");
    }

    let cb_struct = coreaudio_sys::AURenderCallbackStruct {
        inputProc: Some(audio_render_cb),
        inputProcRefCon: Arc::into_raw(state.clone()) as *mut _,
    };
    let status = coreaudio_sys::AudioUnitSetProperty(
        au,
        coreaudio_sys::kAudioUnitProperty_SetRenderCallback,
        coreaudio_sys::kAudioUnitScope_Input,
        0,
        &cb_struct as *const _ as *const _,
        std::mem::size_of_val(&cb_struct) as u32,
    );
    if status != 0 {
        coreaudio_sys::AudioComponentInstanceDispose(au);
        bail!("AudioUnitSetProperty SetRenderCallback failed: {status}");
    }

    let status = coreaudio_sys::AudioUnitInitialize(au);
    if status != 0 {
        coreaudio_sys::AudioComponentInstanceDispose(au);
        bail!("AudioUnitInitialize failed: {status}");
    }

    let status = coreaudio_sys::AudioOutputUnitStart(au);
    if status != 0 {
        coreaudio_sys::AudioUnitUninitialize(au);
        coreaudio_sys::AudioComponentInstanceDispose(au);
        bail!("AudioOutputUnitStart failed: {status}");
    }

    Ok(au as *mut _)
}

unsafe extern "C" fn audio_render_cb(
    in_ref_con: *mut std::ffi::c_void,
    _io_action_flags: *mut coreaudio_sys::AudioUnitRenderActionFlags,
    _in_time_stamp: *const coreaudio_sys::AudioTimeStamp,
    _in_bus_number: u32,
    in_number_frames: u32,
    io_data: *mut coreaudio_sys::AudioBufferList,
) -> coreaudio_sys::OSStatus {
    let state_ptr = in_ref_con as *const Mutex<State>;
    let state_arc = std::mem::ManuallyDrop::new(Arc::from_raw(state_ptr));
    if let Ok(mut state) = state_arc.lock() {
        if io_data.is_null() {
            return 0;
        }
        let buffers = (*io_data).mBuffers.as_mut_ptr();
        let buf = &mut *buffers;
        let out_ptr = buf.mData as *mut i16;
        let channels = state.report.layout.map_or(2, |l| l.channels as usize);
        let needed_samples = in_number_frames as usize * channels;
        let available = state.pcm.len();
        if available >= needed_samples {
            for i in 0..needed_samples {
                *out_ptr.add(i) = state.pcm.pop_front().unwrap_or(0);
            }
            state.report.frames_written += in_number_frames as u64;
        } else {
            for i in 0..available {
                *out_ptr.add(i) = state.pcm.pop_front().unwrap_or(0);
            }
            for i in available..needed_samples {
                *out_ptr.add(i) = 0;
            }
            state.report.frames_written += (available / channels) as u64;
            state.report.underruns += 1;
        }
    }
    0
}

pub fn device_layout() -> Result<Layout> {
    Ok(Layout::STEREO)
}
