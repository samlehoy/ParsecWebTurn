//! Opt-in host-local absolute mapping observation; no input registration/injection.
use std::{
    fs::File,
    io::{self, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

fn grid_match(point: (i32, i32), rect: (i32, i32, i32, i32)) -> Option<usize> {
    let (left, top, width, height) = rect;
    if width <= 0 || height <= 0 {
        return None;
    }
    let grid = [3277i64, 16384, 29490];
    (0..9).find(|index| {
        let x = i64::from(left) + (grid[index % 3] * i64::from(width - 1) + 16383) / 32767;
        let y = i64::from(top) + (grid[index / 3] * i64::from(height - 1) + 16383) / 32767;
        // Allow pixel quantization in MouHID and the desktop mapper.
        (i64::from(point.0) - x).abs() <= 3 && (i64::from(point.1) - y).abs() <= 3
    })
}

pub struct Observation {
    active: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Observation {
    pub fn start(path: &Path) -> io::Result<Self> {
        let mut log = File::options()
            .write(true)
            .create_new(true)
            .open(path.with_extension("mapping.jsonl"))?;
        let active = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (worker_active, worker_stop) = (active.clone(), stop.clone());
        let worker=std::thread::Builder::new().name("usb-absolute-mapping".into()).spawn(move || {
            let start=Instant::now();
            #[cfg(windows)]
            unsafe {
                #[repr(C)]
                #[derive(Default,Clone,Copy,PartialEq,Eq)]
                struct Point { x:i32,y:i32 }
                #[link(name="user32")]
                unsafe extern "system" {
                    fn GetCursorPos(point:*mut Point)->i32;
                    fn GetSystemMetrics(index:i32)->i32;
                }
                // DPI awareness keeps cursor positions and desktop bounds in
                // the same physical coordinate space; failure remains logged.
                #[link(name="user32")]
                unsafe extern "system" { fn SetProcessDpiAwarenessContext(context:isize)->i32; }
                let dpi=SetProcessDpiAwarenessContext(-4)!=0;
                let primary=(0,0,GetSystemMetrics(0),GetSystemMetrics(1));
                let desktop=(GetSystemMetrics(76),GetSystemMetrics(77),GetSystemMetrics(78),GetSystemMetrics(79));
                let _=writeln!(log,"{{\"event\":\"absolute_mapping_started\",\"unix_ms\":{},\"dpi_aware_set\":{dpi},\"primary_size\":[{},{}],\"virtual_bounds\":[{},{},{},{}]}}",super::unix_ms(),primary.2,primary.3,desktop.0,desktop.1,desktop.2,desktop.3);
                let mut previous=None;
                while !worker_stop.load(Ordering::Acquire) && start.elapsed()<Duration::from_secs(120) {
                    if worker_active.load(Ordering::Acquire) {
                        let mut point=Point::default();
                        let valid=GetCursorPos(&mut point)!=0;
                        let changed=valid && previous.is_some_and(|p|p!=point);
                        if valid { previous=Some(point); }
                        let primary_step=if valid { grid_match((point.x,point.y),primary) } else { None };
                        let virtual_step=if valid { grid_match((point.x,point.y),desktop) } else { None };
                        let number=|v:Option<usize>|v.map_or_else(||"null".into(),|n|n.to_string());
                        if writeln!(log,"{{\"event\":\"absolute_mapping_sample\",\"unix_ms\":{},\"cursor_read_ok\":{valid},\"position_changed\":{changed},\"primary_grid_step\":{},\"virtual_grid_step\":{}}}",super::unix_ms(),number(primary_step),number(virtual_step)).and_then(|_|log.flush()).is_err() { break; }
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                let _=writeln!(log,"{{\"event\":\"absolute_mapping_finished\",\"unix_ms\":{}}}",super::unix_ms());
            }
            #[cfg(not(windows))]
            { let _=(&mut log,start,worker_active,worker_stop); }
        })?;
        Ok(Self {
            active,
            stop,
            worker: Some(worker),
        })
    }
    pub fn active(&self, value: bool) {
        self.active.store(value, Ordering::Release);
    }
}
impl Drop for Observation {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absolute_mapping_distinguishes_primary_and_offset_desktop() {
        assert_eq!(grid_match((960, 540), (0, 0, 1920, 1080)), Some(4));
        assert_eq!(grid_match((-960, 540), (-1920, 0, 3840, 1080)), None);
        assert_eq!(grid_match((0, 540), (-1920, 0, 3840, 1080)), Some(4));
        assert_eq!(grid_match((0, 0), (0, 0, 0, 1080)), None);
        assert_eq!(grid_match((500, 500), (0, 0, 1920, 1080)), None);
    }
}
