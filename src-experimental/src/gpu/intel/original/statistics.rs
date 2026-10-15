//! Public Windows presentation feedback only; never reads texture pixels.
use super::*;
use windows::Win32::System::Threading::WaitForSingleObject;

pub(super) struct Trace {
    available: OwnedHandle,
    lost: OwnedHandle,
    records: Vec<Value>,
    errors: Vec<String>,
    total: usize,
    on_screen: usize,
    scanout: usize,
    cross_adapter: usize,
    manager_lost: bool,
}
impl Trace {
    pub(super) fn new(manager: &IPresentationManager) -> Result<Self> {
        let mut errors = Vec::new();
        // Capability failures are recorded as unknown telemetry, not as proof
        // that the content could not be displayed.
        unsafe {
            for kind in [
                PresentStatisticsKind_PresentStatus,
                PresentStatisticsKind_CompositionFrame,
                PresentStatisticsKind_IndependentFlipFrame,
            ] {
                if let Err(e) = manager.EnablePresentStatisticsKind(kind, 1) {
                    errors.push(format!("enable statistics {}: {e}", kind.0));
                }
            }
            Ok(Self {
                available: OwnedHandle(manager.GetPresentStatisticsAvailableEvent()?),
                lost: OwnedHandle(manager.GetLostEvent()?),
                records: Vec::new(),
                errors,
                total: 0,
                on_screen: 0,
                scanout: 0,
                cross_adapter: 0,
                manager_lost: false,
            })
        }
    }
    pub(super) fn collect(&mut self, manager: &IPresentationManager) {
        unsafe {
            self.manager_lost |= WaitForSingleObject(self.lost.0, 0) == WAIT_OBJECT_0;
            // The event avoids treating the documented empty statistics queue
            // as an API failure. Bound each poll so feedback cannot stall input.
            for _ in 0..512 {
                if WaitForSingleObject(self.available.0, 0) != WAIT_OBJECT_0 {
                    break;
                }
                let item = match manager.GetNextPresentStatistics() {
                    Ok(item) => item,
                    Err(e) => {
                        if self.errors.len() < 16 {
                            self.errors.push(format!("read statistics: {e}"));
                        }
                        break;
                    }
                };
                self.total += 1;
                let mut record = json!({"present_id":item.GetPresentId(),"kind":item.GetKind().0});
                if let Ok(status) = item.cast::<IPresentStatusPresentStatistics>() {
                    record["status"] = json!(match status.GetPresentStatus() {
                        PresentStatus_Queued => "queued",
                        PresentStatus_Skipped => "skipped",
                        PresentStatus_Canceled => "canceled",
                        _ => "unknown",
                    });
                } else if let Ok(frame) = item.cast::<ICompositionFramePresentStatistics>() {
                    let mut count = 0;
                    let mut instances = std::ptr::null_mut();
                    frame.GetDisplayInstanceArray(&mut count, &mut instances);
                    // The returned array is owned by the statistics object and
                    // stays valid only while `item` is held. Copy metadata now.
                    let mut displays = Vec::new();
                    if !instances.is_null() && count <= 128 {
                        for instance in std::slice::from_raw_parts(instances, count as usize) {
                            if frame.GetContentTag() == 770 {
                                if matches!(
                                    instance.instanceKind,
                                    CompositionFrameInstanceKind_ComposedOnScreen
                                        | CompositionFrameInstanceKind_ScanoutOnScreen
                                ) {
                                    self.on_screen += 1;
                                }
                                if instance.instanceKind
                                    == CompositionFrameInstanceKind_ScanoutOnScreen
                                {
                                    self.scanout += 1;
                                }
                                if instance.requiredCrossAdapterCopy != 0 {
                                    self.cross_adapter += 1;
                                }
                            }
                            displays.push(json!({"kind":instance.instanceKind.0,
                                "required_cross_adapter_copy":instance.requiredCrossAdapterCopy != 0,
                                "display_adapter_luid":[instance.displayAdapterLUID.HighPart,instance.displayAdapterLUID.LowPart],
                                "render_adapter_luid":[instance.renderAdapterLUID.HighPart,instance.renderAdapterLUID.LowPart],
                                "final_transform":[instance.finalTransform.M11,instance.finalTransform.M12,
                                    instance.finalTransform.M21,instance.finalTransform.M22,instance.finalTransform.M31,instance.finalTransform.M32]}));
                        }
                    }
                    record["content_tag"] = json!(frame.GetContentTag());
                    record["composition_frame_id"] = json!(frame.GetCompositionFrameId());
                    record["display_instances"] = json!(displays);
                } else if let Ok(frame) = item.cast::<IIndependentFlipFramePresentStatistics>() {
                    if frame.GetContentTag() == 770 {
                        self.on_screen += 1;
                        self.scanout += 1;
                    }
                    record["content_tag"] = json!(frame.GetContentTag());
                    record["displayed_interrupt_time"] = json!(frame.GetDisplayedTime().value);
                }
                if self.records.len() < 2048 {
                    self.records.push(record);
                }
            }
        }
    }
    pub(super) fn report(&self) -> Value {
        json!({"statistics_items":self.total,"records":self.records,
            "records_omitted":self.total.saturating_sub(self.records.len()),"errors":self.errors,
            "manager_lost":self.manager_lost,"on_screen_observations":self.on_screen,
            "scanout_observations":self.scanout,"cross_adapter_copy_observations":self.cross_adapter,
            "display_feedback":if self.on_screen > 0 {"observed"} else {"not_confirmed"},
            "note":"Counts are feedback observations, not unique displayed frames. Missing feedback is unknown. OS feedback does not verify pixel correctness or all internal copies."})
    }
}
