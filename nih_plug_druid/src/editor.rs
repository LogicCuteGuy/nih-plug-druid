use crossbeam::atomic::AtomicCell;
use druid::{commands, AppLauncher, ExtEventSink, Target};
use nih_plug::debug::*;
use nih_plug::prelude::{Editor, GuiContext, ParentWindowHandle};
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[cfg(target_os = "windows")]
use std::ffi::c_void;

#[cfg(target_os = "windows")]
use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
#[cfg(target_os = "windows")]
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumThreadWindows, GetWindowLongPtrW, IsWindowVisible, SetParent, SetWindowLongPtrW,
    SetWindowPos, GWL_EXSTYLE, GWL_STYLE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOOWNERZORDER,
    SWP_NOZORDER, SWP_SHOWWINDOW, WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS,
    WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_OVERLAPPEDWINDOW, WS_POPUP,
    WS_SYSMENU, WS_THICKFRAME, WS_VISIBLE,
};

use crate::DruidState;

pub(crate) struct DruidEditor<T>
where
    T: druid::Data + Send + 'static,
{
    pub(crate) druid_state: Arc<DruidState>,
    pub(crate) make_data: Arc<dyn Fn() -> T + 'static + Send + Sync>,
    pub(crate) make_window: Arc<dyn Fn(Arc<dyn GuiContext>) -> druid::WindowDesc<T> + 'static + Send + Sync>,

    pub(crate) scaling_factor: AtomicCell<Option<f32>>,
}

impl<T> Editor for DruidEditor<T>
where
    T: druid::Data + Send + 'static,
{
    fn spawn(
        &self,
        parent: ParentWindowHandle,
        context: Arc<dyn GuiContext>,
    ) -> Box<dyn std::any::Any + Send> {
        let make_data = self.make_data.clone();
        let make_window = self.make_window.clone();
        let druid_state = self.druid_state.clone();

        let (sink_sender, sink_receiver) = mpsc::sync_channel(1);
        #[cfg(target_os = "windows")]
        let (thread_id_sender, thread_id_receiver) = mpsc::sync_channel(1);
        let thread = thread::spawn(move || {
            #[cfg(target_os = "windows")]
            {
                let _ = thread_id_sender.send(unsafe {
                    windows_sys::Win32::System::Threading::GetCurrentThreadId()
                });
            }

            let launcher = AppLauncher::with_window((make_window)(context));
            let event_sink = launcher.get_external_handle();
            let _ = sink_sender.send(event_sink);

            if let Err(err) = launcher.launch((make_data)()) {
                nih_error!("Failed to launch Druid editor: {err}");
            }

            druid_state.open.store(false, Ordering::Release);
        });

        #[cfg(target_os = "windows")]
        if let ParentWindowHandle::Win32Hwnd(parent_hwnd) = parent {
            if let Ok(thread_id) = thread_id_receiver.recv_timeout(Duration::from_secs(2)) {
                let (width, height) = self.druid_state.size();
                if let Some(child_hwnd) = find_thread_window(thread_id, Duration::from_secs(2)) {
                    unsafe {
                        embed_as_child_window(
                            child_hwnd,
                            parent_hwnd,
                            width as i32,
                            height as i32,
                        );
                    }
                } else {
                    nih_error!("Failed to find Druid window for host embedding");
                }
            }
        }

        self.druid_state.open.store(true, Ordering::Release);
        Box::new(DruidEditorHandle {
            druid_state: self.druid_state.clone(),
            event_sink: sink_receiver.recv_timeout(Duration::from_secs(2)).ok(),
            thread: Some(thread),
        })
    }

    fn size(&self) -> (u32, u32) {
        self.druid_state.size()
    }

    fn set_scale_factor(&self, factor: f32) -> bool {
        if self.druid_state.is_open() {
            return false;
        }

        self.scaling_factor.store(Some(factor));
        true
    }

    fn param_value_changed(&self, _id: &str, _normalized_value: f32) {}

    fn param_modulation_changed(&self, _id: &str, _modulation_offset: f32) {}

    fn param_values_changed(&self) {}
}

struct DruidEditorHandle {
    druid_state: Arc<DruidState>,
    event_sink: Option<ExtEventSink>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for DruidEditorHandle {
    fn drop(&mut self) {
        self.druid_state.open.store(false, Ordering::Release);

        if let Some(event_sink) = &self.event_sink {
            let _ = event_sink.submit_command(commands::QUIT_APP, (), Target::Global);
        }

        if let Some(thread) = self.thread.take() {
            if thread.is_finished() {
                let _ = thread.join();
            } else {
                nih_warn!("Druid GUI thread is still running; detaching thread to avoid host UI hang");
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn find_thread_window(thread_id: u32, timeout: Duration) -> Option<HWND> {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        let mut hwnd: HWND = std::ptr::null_mut();
        unsafe {
            EnumThreadWindows(
                thread_id,
                Some(enum_thread_windows_callback),
                &mut hwnd as *mut HWND as LPARAM,
            );
        }

        if !hwnd.is_null() {
            return Some(hwnd);
        }

        std::thread::sleep(Duration::from_millis(10));
    }

    None
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn enum_thread_windows_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
    if IsWindowVisible(hwnd) != 0 {
        let slot = lparam as *mut HWND;
        if !slot.is_null() {
            *slot = hwnd;
        }

        0
    } else {
        1
    }
}

#[cfg(target_os = "windows")]
unsafe fn embed_as_child_window(
    child_hwnd: HWND,
    parent_hwnd: *mut c_void,
    width: i32,
    height: i32,
) {
    let parent_hwnd = parent_hwnd as HWND;

    SetParent(child_hwnd, parent_hwnd);

    let style = GetWindowLongPtrW(child_hwnd, GWL_STYLE) as u32;
    let style = (style & !(WS_POPUP | WS_OVERLAPPEDWINDOW | WS_CAPTION | WS_THICKFRAME | WS_SYSMENU))
        | WS_CHILD
        | WS_VISIBLE
        | WS_CLIPCHILDREN
        | WS_CLIPSIBLINGS;
    SetWindowLongPtrW(child_hwnd, GWL_STYLE, style as isize);

    let ex_style = GetWindowLongPtrW(child_hwnd, GWL_EXSTYLE) as u32;
    let ex_style = ex_style & !(WS_EX_APPWINDOW | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE);
    SetWindowLongPtrW(child_hwnd, GWL_EXSTYLE, ex_style as isize);

    SetWindowPos(
        child_hwnd,
        std::ptr::null_mut(),
        0,
        0,
        width,
        height,
        SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_NOZORDER | SWP_SHOWWINDOW,
    );
}
