use crossbeam::atomic::AtomicCell;
use druid::kurbo::{Affine, BezPath};
use druid::{
    commands, AppLauncher, BoxConstraints, Color, Cursor, Data, Env, Event, EventCtx,
    ExtEventSink, LayoutCtx, LifeCycle, LifeCycleCtx, PaintCtx, Point, Rect, RenderContext,
    Size, Target, UpdateCtx, Widget, WidgetPod, WindowConfig,
};
use nih_plug::debug::*;
use nih_plug::prelude::{Editor, GuiContext, ParentWindowHandle};
#[cfg(target_os = "macos")]
use std::ffi::{c_char, CStr};
#[cfg(target_os = "linux")]
use std::ffi::CStr;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[cfg(any(target_os = "windows", target_os = "linux"))]
use std::ffi::c_void;

#[cfg(target_os = "linux")]
use x11::xlib;

#[cfg(target_os = "macos")]
use cocoa::appkit::NSApp;
#[cfg(target_os = "macos")]
use cocoa::base::{id, nil, NO, YES};
#[cfg(target_os = "macos")]
use cocoa::foundation::{NSInteger, NSRect};
#[cfg(target_os = "macos")]
use objc::{msg_send, sel, sel_impl};

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

use crate::{DruidState, ResizableScaleConfig};

pub(crate) struct ResizableScale<T, W>
where
    T: Data,
    W: Widget<T>,
{
    child: WidgetPod<T, W>,
    druid_state: Arc<DruidState>,
    context: Arc<dyn GuiContext>,
    resize_config: ResizableScaleConfig,
    drag_active: bool,
    drag_start_pos: Point,
    drag_start_window_size: Size,
    last_applied_scale: f64,
}

impl<T, W> ResizableScale<T, W>
where
    T: Data,
    W: Widget<T>,
{
    pub(crate) fn new(
        druid_state: Arc<DruidState>,
        context: Arc<dyn GuiContext>,
        resize_config: ResizableScaleConfig,
        child: W,
    ) -> Self {
        let mut resize_config = resize_config;
        if !resize_config.handle_size.is_finite() || resize_config.handle_size <= 0.0 {
            resize_config.handle_size = 18.0;
        }

        if !resize_config.min_scale_factor.is_finite() || resize_config.min_scale_factor <= 0.0 {
            resize_config.min_scale_factor = 0.5;
        }

        if !resize_config.max_scale_factor.is_finite()
            || resize_config.max_scale_factor < resize_config.min_scale_factor
        {
            resize_config.max_scale_factor = resize_config.min_scale_factor.max(4.0);
        }

        Self {
            child: WidgetPod::new(child),
            last_applied_scale: druid_state.user_scale_factor(),
            druid_state,
            context,
            resize_config,
            drag_active: false,
            drag_start_pos: Point::ORIGIN,
            drag_start_window_size: Size::ZERO,
        }
    }

    fn current_scale(&self) -> f64 {
        self.druid_state.user_scale_factor()
    }

    fn handle_rect(&self, size: Size) -> Rect {
        Rect::from_origin_size(
            Point::new(
                (size.width - self.resize_config.handle_size).max(0.0),
                (size.height - self.resize_config.handle_size).max(0.0),
            ),
            Size::new(self.resize_config.handle_size, self.resize_config.handle_size),
        )
    }

    fn point_in_handle(&self, size: Size, point: Point) -> bool {
        let rect = self.handle_rect(size);
        if !rect.contains(point) {
            return false;
        }

        let local_x = point.x - rect.x0;
        let local_y = point.y - rect.y0;
        local_x + local_y >= self.resize_config.handle_size
    }

    fn scaled_mouse_event(&self, mouse: &druid::MouseEvent) -> druid::MouseEvent {
        let scale = self.current_scale();
        let mut mouse = mouse.clone();
        mouse.pos = Point::new(mouse.pos.x / scale, mouse.pos.y / scale);
        mouse.window_pos = Point::new(mouse.window_pos.x / scale, mouse.window_pos.y / scale);
        mouse
    }

    fn scaled_event(&self, event: &Event) -> Option<Event> {
        match event {
            Event::MouseDown(mouse) => Some(Event::MouseDown(self.scaled_mouse_event(mouse))),
            Event::MouseMove(mouse) => Some(Event::MouseMove(self.scaled_mouse_event(mouse))),
            Event::MouseUp(mouse) => Some(Event::MouseUp(self.scaled_mouse_event(mouse))),
            Event::Wheel(mouse) => Some(Event::Wheel(self.scaled_mouse_event(mouse))),
            Event::WindowSize(size) => Some(Event::WindowSize(Size::new(
                size.width / self.current_scale(),
                size.height / self.current_scale(),
            ))),
            _ => None,
        }
    }

    fn resize_window_event(&mut self, ctx: &mut EventCtx, user_scale_factor: f64) {
        let old_scale = self.druid_state.user_scale_factor();
        let user_scale_factor = user_scale_factor.clamp(
            self.resize_config.min_scale_factor,
            self.resize_config.max_scale_factor,
        );
        self.druid_state.set_user_scale_factor(user_scale_factor);

        if !self.context.request_resize() {
            self.druid_state.set_user_scale_factor(old_scale);
            self.last_applied_scale = old_scale;
            return;
        }

        self.last_applied_scale = user_scale_factor;

        let (width, height) = self.druid_state.size();
        ctx.submit_command(
            commands::CONFIGURE_WINDOW.with(
                WindowConfig::default().window_size(Size::new(width as f64, height as f64)),
            ),
        );
        ctx.request_layout();
        ctx.request_paint();
    }

    fn sync_scale_change_event(&mut self, ctx: &mut EventCtx) {
        let old_scale = self.last_applied_scale;
        let current_scale = self.current_scale();
        if (old_scale - current_scale).abs() <= f64::EPSILON {
            return;
        }

        if !self.context.request_resize() {
            self.druid_state.set_user_scale_factor(old_scale);
            return;
        }

        self.last_applied_scale = current_scale;
        let (width, height) = self.druid_state.size();
        ctx.submit_command(
            commands::CONFIGURE_WINDOW.with(
                WindowConfig::default().window_size(Size::new(width as f64, height as f64)),
            ),
        );
        ctx.request_layout();
        ctx.request_paint();
    }

    fn sync_scale_change_update(&mut self, ctx: &mut UpdateCtx) {
        let old_scale = self.last_applied_scale;
        let current_scale = self.current_scale();
        if (old_scale - current_scale).abs() <= f64::EPSILON {
            return;
        }

        if !self.context.request_resize() {
            self.druid_state.set_user_scale_factor(old_scale);
            return;
        }

        self.last_applied_scale = current_scale;
        let (width, height) = self.druid_state.size();
        ctx.submit_command(
            commands::CONFIGURE_WINDOW.with(
                WindowConfig::default().window_size(Size::new(width as f64, height as f64)),
            ),
        );
        ctx.request_layout();
        ctx.request_paint();
    }

    fn paint_handle(&self, ctx: &mut PaintCtx) {
        let rect = self.handle_rect(ctx.size());
        let line_color = Color::grey8(160);

        let mut outline = BezPath::new();
        outline.move_to(Point::new(rect.x0, rect.y1));
        outline.line_to(Point::new(rect.x1, rect.y1));
        outline.line_to(Point::new(rect.x1, rect.y0));
        ctx.stroke(outline, &Color::grey8(70), 1.0);

        for offset in [4.0, 8.0, 12.0] {
            ctx.stroke(
                druid::kurbo::Line::new(
                    Point::new(rect.x1 - offset, rect.y1),
                    Point::new(rect.x1, rect.y1 - offset),
                ),
                &line_color,
                1.5,
            );
        }
    }
}

impl<T, W> Widget<T> for ResizableScale<T, W>
where
    T: Data,
    W: Widget<T>,
{
    fn event(&mut self, ctx: &mut EventCtx, event: &Event, data: &mut T, env: &Env) {
        self.sync_scale_change_event(ctx);

        match event {
            Event::MouseDown(mouse) if self.point_in_handle(ctx.size(), mouse.pos) => {
                self.drag_active = true;
                self.drag_start_pos = mouse.pos;
                self.drag_start_window_size = ctx.size();
                ctx.set_active(true);
                ctx.set_handled();
                ctx.request_paint();
                return;
            }
            Event::MouseMove(mouse) if self.drag_active => {
                let (logical_width, logical_height) = self.druid_state.logical_size();
                let new_width = (self.drag_start_window_size.width + mouse.pos.x - self.drag_start_pos.x)
                    .max(logical_width as f64 * self.resize_config.min_scale_factor);
                let new_height = (self.drag_start_window_size.height + mouse.pos.y - self.drag_start_pos.y)
                    .max(logical_height as f64 * self.resize_config.min_scale_factor);
                let new_scale = (new_width / logical_width as f64)
                    .max(new_height / logical_height as f64);

                self.resize_window_event(ctx, new_scale);
                ctx.set_handled();
                return;
            }
            Event::MouseMove(mouse) => {
                if self.point_in_handle(ctx.size(), mouse.pos) {
                    ctx.set_cursor(&Cursor::ResizeLeftRight);
                }
            }
            Event::MouseUp(_) if self.drag_active => {
                self.drag_active = false;
                ctx.set_active(false);
                ctx.set_handled();
                ctx.request_paint();
                return;
            }
            _ => {}
        }

        if let Some(scaled_event) = self.scaled_event(event) {
            self.child.event(ctx, &scaled_event, data, env);
        } else {
            self.child.event(ctx, event, data, env);
        }
    }

    fn lifecycle(&mut self, ctx: &mut LifeCycleCtx, event: &LifeCycle, data: &T, env: &Env) {
        self.child.lifecycle(ctx, event, data, env);
    }

    fn update(&mut self, ctx: &mut UpdateCtx, old_data: &T, data: &T, env: &Env) {
        self.sync_scale_change_update(ctx);

        self.child.update(ctx, data, env);
        if !old_data.same(data) {
            return;
        }

        if ctx.env_changed() {
            ctx.request_paint();
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx, bc: &BoxConstraints, data: &T, env: &Env) -> Size {
        let scale = self.current_scale();
        let child_bc = BoxConstraints::new(
            Size::new(bc.min().width / scale, bc.min().height / scale),
            Size::new(bc.max().width / scale, bc.max().height / scale),
        );
        let child_size = self.child.layout(ctx, &child_bc, data, env);
        self.child.set_origin(ctx, Point::ORIGIN);

        bc.constrain(Size::new(child_size.width * scale, child_size.height * scale))
    }

    fn paint(&mut self, ctx: &mut PaintCtx, data: &T, env: &Env) {
        ctx.with_save(|ctx| {
            ctx.transform(Affine::scale(self.current_scale()));
            self.child.paint(ctx, data, env);
        });

        self.paint_handle(ctx);
    }
}

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
        if self
            .druid_state
            .open
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            nih_warn!("Ignoring duplicate Druid editor spawn while an editor is still open");
            return Box::new(DruidEditorHandle {
                druid_state: self.druid_state.clone(),
                event_sink: None,
                thread: None,
                owns_gui: false,
            });
        }

        let make_data = self.make_data.clone();
        let make_window = self.make_window.clone();
        let druid_state = self.druid_state.clone();
        #[cfg(target_os = "linux")]
        let existing_root_windows = unsafe { list_x11_root_windows() };

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

        #[cfg(target_os = "linux")]
        if let ParentWindowHandle::X11Window(parent_xid) = parent {
            let (width, height) = self.druid_state.size();
            let child_xid = unsafe {
                find_new_x11_window(
                    parent_xid as u64,
                    existing_root_windows.as_deref().unwrap_or(&[]),
                    Duration::from_secs(5),
                )
            };
            if let Some(child_xid) = child_xid {
                unsafe {
                    embed_x11_window(child_xid, parent_xid as u64, width, height);
                }
            } else {
                nih_error!("Failed to find Druid window for host X11 embedding");
            }
        }

        #[cfg(target_os = "macos")]
        if let ParentWindowHandle::AppKitNsView(parent_ns_view) = parent {
            if let Some(child_window) = find_druid_window(Duration::from_secs(5)) {
                let mut embedded = false;
                // Host views are sometimes not fully initialized yet when spawn() returns.
                for _ in 0..120 {
                    unsafe {
                        if embed_child_window_macos(child_window, parent_ns_view as id) {
                            for _ in 0..50 {
                                let _ = sync_child_window_frame_macos(child_window, parent_ns_view as id);
                                thread::sleep(Duration::from_millis(10));
                            }

                            embedded = true;
                            break;
                        }
                    }

                    thread::sleep(Duration::from_millis(10));
                }

                if !embedded {
                    nih_error!("Failed to embed Druid window into host AppKit view");
                }
            } else {
                nih_error!("Failed to find Druid window for host embedding");
            }
        }

        Box::new(DruidEditorHandle {
            druid_state: self.druid_state.clone(),
            event_sink: sink_receiver.recv_timeout(Duration::from_secs(2)).ok(),
            thread: Some(thread),
            owns_gui: true,
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
    owns_gui: bool,
}

impl Drop for DruidEditorHandle {
    fn drop(&mut self) {
        if !self.owns_gui {
            return;
        }

        if let Some(event_sink) = &self.event_sink {
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            let _ = event_sink.submit_command(commands::CLOSE_ALL_WINDOWS, (), Target::Global);

            #[cfg(target_os = "windows")]
            let _ = event_sink.submit_command(commands::QUIT_APP, (), Target::Global);

            #[cfg(target_os = "linux")]
            let _ = event_sink.submit_command(commands::QUIT_APP, (), Target::Global);
        }

        if let Some(thread) = self.thread.take() {
            if thread.is_finished() {
                let _ = thread.join();
                self.druid_state.open.store(false, Ordering::Release);
            } else {
                let druid_state = self.druid_state.clone();
                thread::spawn(move || {
                    let _ = thread.join();
                    druid_state.open.store(false, Ordering::Release);
                });
            }
        } else {
            self.druid_state.open.store(false, Ordering::Release);
        }
    }
}

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
fn find_druid_window(timeout: Duration) -> Option<id> {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if let Some(window) = unsafe { find_druid_window_once() } {
            return Some(window);
        }

        thread::sleep(Duration::from_millis(10));
    }

    None
}

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
unsafe fn find_druid_window_once() -> Option<id> {
    let app = NSApp();
    if app == nil {
        return None;
    }

    let windows: id = msg_send![app, windows];
    if windows == nil {
        return None;
    }

    let count: usize = msg_send![windows, count];
    for index in (0..count).rev() {
        let window: id = msg_send![windows, objectAtIndex: index];
        if window == nil {
            continue;
        }

        let class_name: id = msg_send![window, className];
        if class_name == nil {
            continue;
        }

        let class_name_utf8: *const c_char = msg_send![class_name, UTF8String];
        if class_name_utf8.is_null() {
            continue;
        }

        let class_name_bytes = CStr::from_ptr(class_name_utf8).to_bytes();
        let is_druid_window = class_name_bytes == b"DruidWindow"
            || class_name_bytes.windows(5).any(|segment| segment == b"Druid");
        if is_druid_window {
            return Some(window);
        }
    }

    None
}

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
unsafe fn embed_child_window_macos(child_window: id, parent_ns_view: id) -> bool {
    if child_window == nil || parent_ns_view == nil {
        return false;
    }

    let parent_window: id = msg_send![parent_ns_view, window];
    if parent_window == nil {
        return false;
    }

    if child_window == parent_window {
        nih_error!("Refusing to embed host NSWindow as Druid child window");
        return false;
    }

    let child_parent_window: id = msg_send![child_window, parentWindow];
    if child_parent_window != nil && child_parent_window != parent_window {
        let _: () = msg_send![child_parent_window, removeChildWindow: child_window];
    }

    let _: () = msg_send![child_window, setReleasedWhenClosed: NO];

    if !sync_child_window_frame_macos(child_window, parent_ns_view) {
        return false;
    }

    let _: () = msg_send![parent_window, addChildWindow: child_window ordered: NSWINDOW_ORDERING_MODE_ABOVE];
    let _: () = msg_send![child_window, orderFront: nil];

    true
}

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
unsafe fn sync_child_window_frame_macos(child_window: id, parent_ns_view: id) -> bool {
    if child_window == nil || parent_ns_view == nil {
        return false;
    }

    let parent_window: id = msg_send![parent_ns_view, window];
    if parent_window == nil {
        return false;
    }

    let bounds: NSRect = msg_send![parent_ns_view, bounds];
    if bounds.size.width <= 1.0 || bounds.size.height <= 1.0 {
        return false;
    }

    let rect_in_parent_window: NSRect = msg_send![parent_ns_view, convertRect: bounds toView: nil];
    let screen_rect: NSRect = msg_send![parent_window, convertRectToScreen: rect_in_parent_window];
    let _: () = msg_send![child_window, setFrame: screen_rect display: YES];

    true
}

#[cfg(target_os = "macos")]
const NSWINDOW_ORDERING_MODE_ABOVE: NSInteger = 1;

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

#[cfg(target_os = "linux")]
unsafe fn list_x11_root_windows() -> Option<Vec<u64>> {
    let display = xlib::XOpenDisplay(std::ptr::null());
    if display.is_null() {
        return None;
    }

    let screen = xlib::XDefaultScreen(display);
    let root = xlib::XRootWindow(display, screen);
    let windows = query_x11_children(display, root);

    xlib::XCloseDisplay(display);
    Some(windows)
}

#[cfg(target_os = "linux")]
unsafe fn query_x11_children(display: *mut xlib::Display, window: xlib::Window) -> Vec<u64> {
    let mut root_ret: xlib::Window = 0;
    let mut parent_ret: xlib::Window = 0;
    let mut children: *mut xlib::Window = std::ptr::null_mut();
    let mut nchildren: u32 = 0;

    let mut result = Vec::new();
    if xlib::XQueryTree(
        display,
        window,
        &mut root_ret,
        &mut parent_ret,
        &mut children,
        &mut nchildren,
    ) != 0
    {
        if !children.is_null() && nchildren > 0 {
            for i in 0..nchildren as isize {
                result.push(*children.offset(i));
            }

            xlib::XFree(children as *mut c_void);
        }
    }

    result
}

#[cfg(target_os = "linux")]
unsafe fn find_new_x11_window(
    parent_xid: u64,
    existing_windows: &[u64],
    timeout: Duration,
) -> Option<u64> {
    let display = xlib::XOpenDisplay(std::ptr::null());
    if display.is_null() {
        return None;
    }

    let screen = xlib::XDefaultScreen(display);
    let root = xlib::XRootWindow(display, screen);
    let deadline = std::time::Instant::now() + timeout;

    while std::time::Instant::now() < deadline {
        let windows = query_x11_children(display, root);
        for &window in windows.iter().rev() {
            if window == parent_xid || existing_windows.contains(&window) {
                continue;
            }

            let mut attrs = std::mem::zeroed::<xlib::XWindowAttributes>();
            if xlib::XGetWindowAttributes(display, window, &mut attrs) != 0
                && attrs.map_state == xlib::IsViewable
                && is_probably_druid_window(display, window)
            {
                xlib::XCloseDisplay(display);
                return Some(window);
            }
        }

        thread::sleep(Duration::from_millis(10));
    }

    xlib::XCloseDisplay(display);
    None
}

#[cfg(target_os = "linux")]
unsafe fn embed_x11_window(child_xid: u64, parent_xid: u64, width: u32, height: u32) {
    if child_xid == parent_xid {
        nih_error!("Refusing to embed parent X11 window as a child window");
        return;
    }

    let display = xlib::XOpenDisplay(std::ptr::null());
    if display.is_null() {
        nih_error!("Failed to open X11 display for window embedding");
        return;
    }

    xlib::XReparentWindow(display, child_xid, parent_xid, 0, 0);
    xlib::XResizeWindow(display, child_xid, width, height);
    xlib::XMapWindow(display, child_xid);
    xlib::XFlush(display);
    xlib::XCloseDisplay(display);
}

#[cfg(target_os = "linux")]
unsafe fn is_probably_druid_window(display: *mut xlib::Display, window: xlib::Window) -> bool {
    let mut class_hint = std::mem::zeroed::<xlib::XClassHint>();
    if xlib::XGetClassHint(display, window, &mut class_hint) == 0 {
        return false;
    }

    let mut is_druid = false;

    if !class_hint.res_name.is_null() {
        let name = CStr::from_ptr(class_hint.res_name).to_bytes();
        if name.windows(5).any(|segment| segment.eq_ignore_ascii_case(b"druid")) {
            is_druid = true;
        }

        xlib::XFree(class_hint.res_name as *mut c_void);
    }

    if !class_hint.res_class.is_null() {
        let class = CStr::from_ptr(class_hint.res_class).to_bytes();
        if class.windows(5).any(|segment| segment.eq_ignore_ascii_case(b"druid")) {
            is_druid = true;
        }

        xlib::XFree(class_hint.res_class as *mut c_void);
    }

    is_druid
}
