/*

`async-winit` is free software: you can redistribute it and/or modify it under the terms of one of
the following licenses:

* GNU Lesser General Public License as published by the Free Software Foundation, either
  version 3 of the License, or (at your option) any later version.
* Mozilla Public License as published by the Mozilla Foundation, version 2.

`async-winit` is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even
the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU Affero General
Public License and the Patron License for more details.

You should have received a copy of the GNU Lesser General Public License and the Mozilla
Public License along with `async-winit`. If not, see <https://www.gnu.org/licenses/>.

*/

//! The shared reactor used by the runtime.

use crate::filter::ReactorWaker;
use crate::handler::Handler;
use crate::oneoff::Complete;
use crate::sync::{ThreadSafety, __private::*};
use crate::window::registration::Registration as WinRegistration;
use crate::window::WindowBuilder;

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::Waker;
use std::time::{Duration, Instant};

use winit::dpi::{PhysicalPosition, PhysicalSize, Position, Size};
use winit::error::{ExternalError, NotSupportedError, OsError};
use winit::monitor::MonitorHandle;
use winit::window::{
    CursorGrabMode, CursorIcon, Fullscreen, Icon, ImePurpose, ResizeDirection, Theme,
    UserAttentionType, Window, WindowId, WindowLevel,
};

const NEEDS_EXIT: i64 = 0x1;
const EXIT_CODE_SHIFT: u32 = 1;

#[doc(hidden)]
pub struct Reactor<T: ThreadSafety> {
    /// The exit code to exit with, if any.
    exit_code: T::AtomicI64,

    /// The channel used to send event loop operation requests.
    evl_ops: (T::Sender<EventLoopOp<T>>, T::Receiver<EventLoopOp<T>>),

    /// The list of windows.
    windows: T::Mutex<HashMap<WindowId, T::Rc<WinRegistration<T>>>>,

    /// The event loop proxy.
    ///
    /// Used to wake up the event loop.
    proxy: T::OnceLock<Arc<ReactorWaker>>,

    /// The timer wheel.
    timers: T::Mutex<BTreeMap<(Instant, usize), Waker>>,

    /// Queue of timer operations.
    timer_op_queue: T::ConcurrentQueue<TimerOp>,

    /// The last timer ID we used.
    timer_id: T::AtomicUsize,

    /// Registration for event loop events.
    pub(crate) evl_registration: GlobalRegistration<T>,
}

enum TimerOp {
    /// Add a new timer.
    InsertTimer(Instant, usize, Waker),

    /// Delete an existing timer.
    RemoveTimer(Instant, usize),
}

impl<TS: ThreadSafety> Reactor<TS> {
    /// Create an empty reactor.
    pub(crate) fn new() -> Self {
        println!("Creating a new reactor");
        static ALREADY_EXISTS: AtomicBool = AtomicBool::new(false);
        if ALREADY_EXISTS.swap(true, Ordering::SeqCst) {
            panic!("Only one instance of `Reactor` can exist at a time");
        }

        Reactor {
            exit_code: <TS::AtomicI64>::new(0),
            proxy: TS::OnceLock::new(),
            evl_ops: TS::channel_bounded(1024),
            windows: TS::Mutex::new(HashMap::new()),
            timers: TS::Mutex::new(BTreeMap::new()),
            timer_op_queue: TS::ConcurrentQueue::bounded(1024),
            timer_id: TS::AtomicUsize::new(1),
            evl_registration: GlobalRegistration::new(),
        }
    }

    /// Get the global instance of this reactor.
    pub(crate) fn get() -> TS::Rc<Self> {
        TS::get_reactor()
    }

    /// Set the event loop proxy.
    pub(crate) fn set_proxy(&self, proxy: Arc<ReactorWaker>) {
        self.proxy.set(proxy).ok();
    }

    /// Get whether or not we need to exit, and the code as well.
    pub(crate) fn exit_requested(&self) -> Option<i32> {
        let value = self.exit_code.load(Ordering::SeqCst);
        if value & NEEDS_EXIT != 0 {
            Some((value >> EXIT_CODE_SHIFT) as i32)
        } else {
            None
        }
    }

    /// Request that the event loop exit.
    pub(crate) fn request_exit(&self, code: i32) {
        let value = NEEDS_EXIT | (code as i64) << EXIT_CODE_SHIFT;

        // Set the exit code.
        self.exit_code.store(value, Ordering::SeqCst);

        // Wake up the event loop.
        self.notify();
    }

    /// Insert a new timer into the timer wheel.
    pub(crate) fn insert_timer(&self, deadline: Instant, waker: &Waker) -> usize {
        // Generate a new ID.
        let id = self.timer_id.fetch_add(1, Ordering::Relaxed);

        // Insert the timer into the timer wheel.
        let mut op = TimerOp::InsertTimer(deadline, id, waker.clone());
        while let Err(e) = self.timer_op_queue.push(op) {
            // Process incoming timer operations.
            let mut timers = self.timers.lock().unwrap();
            self.process_timer_ops(&mut timers);
            op = e;
        }

        // Notify that we have new timers.
        self.notify();

        // Return the ID.
        id
    }

    /// Remove a timer from the timer wheel.
    pub(crate) fn remove_timer(&self, deadline: Instant, id: usize) {
        let mut op = TimerOp::RemoveTimer(deadline, id);
        while let Err(e) = self.timer_op_queue.push(op) {
            // Process incoming timer operations.
            let mut timers = self.timers.lock().unwrap();
            self.process_timer_ops(&mut timers);
            op = e;
        }
    }

    /// Insert a window into the window list.
    pub(crate) fn insert_window(&self, id: WindowId) -> TS::Rc<WinRegistration<TS>> {
        println!("Insert window {:?}", id);
        let mut windows = self.windows.lock().unwrap();
        let registration = TS::Rc::new(WinRegistration::new());
        windows.insert(id, registration.clone());
        registration
    }

    /// Remove a window from the window list.
    pub(crate) fn remove_window(&self, id: WindowId) {
        println!("Removing a window {:?}", id);
        let mut windows = self.windows.lock().unwrap();
        windows.remove(&id);
    }

    /// Process pending timer operations.
    fn process_timer_ops(&self, timers: &mut BTreeMap<(Instant, usize), Waker>) {
        // Limit the number of operations we process at once to avoid starving other tasks.
        let limit = self.timer_op_queue.capacity();

        self.timer_op_queue
            .try_iter()
            .take(limit)
            .for_each(|op| match op {
                TimerOp::InsertTimer(deadline, id, waker) => {
                    timers.insert((deadline, id), waker);
                }
                TimerOp::RemoveTimer(deadline, id) => {
                    if let Some(waker) = timers.remove(&(deadline, id)) {
                        // Don't let a waker that panics on drop blow everything up.
                        std::panic::catch_unwind(|| drop(waker)).ok();
                    }
                }
            });
    }

    /// Process timers and return the amount of time to wait.
    pub(crate) fn process_timers(&self, wakers: &mut Vec<Waker>) -> Option<Instant> {
        // Process incoming timer operations.
        let mut timers = self.timers.lock().unwrap();
        self.process_timer_ops(&mut timers);

        let now = Instant::now();

        // Split timers into pending and ready timers.
        let pending = timers.split_off(&(now + Duration::from_nanos(1), 0));
        let ready = std::mem::replace(&mut *timers, pending);

        // Figure out how long it will be until the next timer is ready.
        let deadline = if ready.is_empty() {
            timers.keys().next().map(|(deadline, _)| *deadline)
        } else {
            // There are timers ready to fire now.
            Some(now)
        };

        drop(timers);

        // Push wakers for ready timers.
        wakers.extend(ready.into_values());

        deadline
    }

    /// Wake up the event loop.
    pub(crate) fn notify(&self) {
        if let Some(proxy) = self.proxy.get() {
            proxy.notify();
        }
    }

    /// Push an event loop operation.
    pub(crate) async fn push_event_loop_op(&self, op: EventLoopOp<TS>) {
        if self.evl_ops.0.send(op).await.is_err() {
            panic!("Failed to push event loop operation");
        }

        // Notify the event loop that there is a new operation.
        self.notify();
    }

    /// Drain the event loop operation queue.
    pub(crate) fn drain_loop_queue<T: 'static>(
        &self,
        elwt: &winit::event_loop::EventLoopWindowTarget<T>,
    ) {
        for _ in 0..self.evl_ops.1.capacity() {
            if let Some(op) = self.evl_ops.1.try_recv() {
                op.run(elwt);
            } else {
                break;
            }
        }
    }

    pub fn evl_ops_len(&self) -> usize {
        self.evl_ops.1.len()
    }

    /// Post an event to the reactor.
    pub(crate) async fn post_event<T: 'static>(&self, event: winit::event::Event<T>) {
        use winit::event::Event;

        match event {
            Event::WindowEvent { window_id, event } => {
                let registration = {
                    let windows = self.windows.lock().unwrap();
                    windows.get(&window_id).cloned()
                };
                if let Some(registration) = registration {
                    registration.signal(event).await;
                }
            }
            Event::Resumed => {
                self.evl_registration.resumed.run_with(&mut ()).await;
            }
            Event::Suspended => self.evl_registration.suspended.run_with(&mut ()).await,
            _ => {}
        }
    }
}

/// An operation to run in the main event loop thread.
pub(crate) enum EventLoopOp<TS: ThreadSafety> {
    /// Build a window.
    BuildWindow {
        /// The window builder to build.
        builder: Box<WindowBuilder>,

        /// The window has been built.
        waker: Complete<Result<winit::window::Window, OsError>, TS>,
    },

    /// Get the primary monitor.
    PrimaryMonitor(Complete<Option<MonitorHandle>, TS>),

    /// Get the list of monitors.
    AvailableMonitors(Complete<Vec<MonitorHandle>, TS>),

    /// Get the inner position of the window.
    InnerPosition {
        /// The window.
        window: TS::Rc<Window>,

        /// Wake up the task.
        waker: Complete<Result<PhysicalPosition<i32>, NotSupportedError>, TS>,
    },

    /// Get the outer position of the window.
    OuterPosition {
        /// The window.
        window: TS::Rc<Window>,

        /// Wake up the task.
        waker: Complete<Result<PhysicalPosition<i32>, NotSupportedError>, TS>,
    },

    /// Set the outer position.
    SetOuterPosition {
        /// The window.
        window: TS::Rc<Window>,

        /// The position.
        position: Position,

        /// Wake up the task.
        waker: Complete<(), TS>,
    },

    /// Get the inner size.
    InnerSize {
        /// The window.
        window: TS::Rc<Window>,

        /// Wake up the task.
        waker: Complete<PhysicalSize<u32>, TS>,
    },

    /// Set the min inner size.
    SetMinInnerSize {
        /// The window.
        window: TS::Rc<Window>,

        /// The size.
        size: Option<Size>,

        /// Wake up the task.
        waker: Complete<(), TS>,
    },

    /// Set the max inner size.
    SetMaxInnerSize {
        /// The window.
        window: TS::Rc<Window>,

        /// The size.
        size: Option<Size>,

        /// Wake up the task.
        waker: Complete<(), TS>,
    },

    /// Get the outer size.
    OuterSize {
        /// The window.
        window: TS::Rc<Window>,

        /// Wake up the task.
        waker: Complete<PhysicalSize<u32>, TS>,
    },

    /// Get the resize increments.
    ResizeIncrements {
        /// The window.
        window: TS::Rc<Window>,

        /// Wake up the task.
        waker: Complete<Option<PhysicalSize<u32>>, TS>,
    },

    /// Set the resize increments.
    SetResizeIncrements {
        /// The window.
        window: TS::Rc<Window>,

        /// The size.
        size: Option<Size>,

        /// Wake up the task.
        waker: Complete<(), TS>,
    },

    /// Set the title.
    SetTitle {
        /// The window.
        window: TS::Rc<Window>,

        /// The title.
        title: String,

        /// Wake up the task.
        waker: Complete<(), TS>,
    },

    /// Set whether the window is transparent.
    SetTransparent {
        /// The window.
        window: TS::Rc<Window>,

        /// Whether the window is transparent.
        transparent: bool,

        /// Wake up the task.
        waker: Complete<(), TS>,
    },

    /// Set whether or not the window is resizable.
    SetResizable {
        /// The window.
        window: TS::Rc<Window>,

        /// Whether or not the window is resizable.
        resizable: bool,

        /// Wake up the task.
        waker: Complete<(), TS>,
    },

    /// Set whether the window is visible.
    SetVisible {
        /// The window.
        window: TS::Rc<Window>,

        /// Whether the window is visible.
        visible: bool,

        /// Wake up the task.
        waker: Complete<(), TS>,
    },

    /// Get whether the window is resizable.
    Resizable {
        /// The window.
        window: TS::Rc<Window>,

        /// Wake up the task.
        waker: Complete<bool, TS>,
    },

    /// Get whether the window is visible.
    Visible {
        /// The window.
        window: TS::Rc<Window>,

        /// Wake up the task.
        waker: Complete<Option<bool>, TS>,
    },

    /// Set whether the window is minimized.
    SetMinimized {
        /// The window.
        window: TS::Rc<Window>,

        /// Whether the window is minimized.
        minimized: bool,

        /// Wake up the task.
        waker: Complete<(), TS>,
    },

    /// Get whether the window is minimized.
    Minimized {
        /// The window.
        window: TS::Rc<Window>,

        /// Wake up the task.
        waker: Complete<Option<bool>, TS>,
    },

    /// Set whether the window is maximized.
    SetMaximized {
        /// The window.
        window: TS::Rc<Window>,

        /// Whether the window is maximized.
        maximized: bool,

        /// Wake up the task.
        waker: Complete<(), TS>,
    },

    /// Get whether the window is maximized.
    Maximized {
        /// The window.
        window: TS::Rc<Window>,

        /// Wake up the task.
        waker: Complete<bool, TS>,
    },

    /// Set whether the window is fullscreen.
    SetFullscreen {
        /// The window.
        window: TS::Rc<Window>,

        /// Whether the window is fullscreen.
        fullscreen: Option<Fullscreen>,

        /// Wake up the task.
        waker: Complete<(), TS>,
    },

    /// Get whether the window is fullscreen.
    Fullscreen {
        /// The window.
        window: TS::Rc<Window>,

        /// Wake up the task.
        waker: Complete<Option<Fullscreen>, TS>,
    },

    /// Set whether the window is decorated.
    SetDecorated {
        /// The window.
        window: TS::Rc<Window>,

        /// Whether the window is decorated.
        decorated: bool,

        /// Wake up the task.
        waker: Complete<(), TS>,
    },

    /// Get whether the window is decorated.
    Decorated {
        /// The window.
        window: TS::Rc<Window>,

        /// Wake up the task.
        waker: Complete<bool, TS>,
    },

    /// Set the window level.
    SetWindowLevel {
        /// The window.
        window: TS::Rc<Window>,

        /// The window level.
        level: WindowLevel,

        /// Wake up the task.
        waker: Complete<(), TS>,
    },

    /// Set the window icon.
    SetWindowIcon {
        /// The window.
        window: TS::Rc<Window>,

        /// The window icon.
        icon: Option<Icon>,

        /// Wake up the task.
        waker: Complete<(), TS>,
    },

    /// Set the IME position.
    SetImeCursorArea {
        /// The window.
        window: TS::Rc<Window>,

        /// The IME position.
        position: Position,

        /// The IME size
        size: Size,

        /// Wake up the task.
        waker: Complete<(), TS>,
    },

    /// Set whether IME is allowed.
    SetImeAllowed {
        /// The window.
        window: TS::Rc<Window>,

        /// Whether IME is allowed.
        allowed: bool,

        /// Wake up the task.
        waker: Complete<(), TS>,
    },

    /// Set the IME purpose.
    SetImePurpose {
        /// The window.
        window: TS::Rc<Window>,

        /// The IME purpose.
        purpose: ImePurpose,

        /// Wake up the task.
        waker: Complete<(), TS>,
    },

    /// Focus the window.
    FocusWindow {
        /// The window.
        window: TS::Rc<Window>,

        /// Wake up the task.
        waker: Complete<(), TS>,
    },

    /// Tell whether or not the window is focused.
    Focused {
        /// The window.
        window: TS::Rc<Window>,

        /// Wake up the task.
        waker: Complete<bool, TS>,
    },

    /// Request user attention.
    RequestUserAttention {
        /// The window.
        window: TS::Rc<Window>,

        /// The request.
        request_type: Option<UserAttentionType>,

        /// Wake up the task.
        waker: Complete<(), TS>,
    },

    /// Set the theme of the window.
    SetTheme {
        /// The window.
        window: TS::Rc<Window>,

        /// The theme.
        theme: Option<Theme>,

        /// Wake up the task.
        waker: Complete<(), TS>,
    },

    /// Get the theme of the window.
    Theme {
        /// The window.
        window: TS::Rc<Window>,

        /// Wake up the task.
        waker: Complete<Option<Theme>, TS>,
    },

    /// Set whether the content is protected.
    SetProtectedContent {
        /// The window.
        window: TS::Rc<Window>,

        /// Whether the content is protected.
        protected: bool,

        /// Wake up the task.
        waker: Complete<(), TS>,
    },

    /// Get the title.
    Title {
        /// The window.
        window: TS::Rc<Window>,

        /// Wake up the task.
        waker: Complete<String, TS>,
    },

    /// Set the cursor icon.
    SetCursorIcon {
        /// The window.
        window: TS::Rc<Window>,

        /// The cursor icon.
        icon: CursorIcon,

        /// Wake up the task.
        waker: Complete<(), TS>,
    },

    /// Set the cursor position.
    SetCursorPosition {
        /// The window.
        window: TS::Rc<Window>,

        /// The cursor position.
        position: Position,

        /// Wake up the task.
        waker: Complete<Result<(), ExternalError>, TS>,
    },

    /// Set the cursor grab.
    SetCursorGrab {
        /// The window.
        window: TS::Rc<Window>,

        /// The mode to grab the cursor.
        mode: CursorGrabMode,

        /// Wake up the task.
        waker: Complete<Result<(), ExternalError>, TS>,
    },

    /// Set whether the cursor is visible.
    SetCursorVisible {
        /// The window.
        window: TS::Rc<Window>,

        /// Whether the cursor is visible.
        visible: bool,

        /// Wake up the task.
        waker: Complete<(), TS>,
    },

    /// Drag the window.
    DragWindow {
        /// The window.
        window: TS::Rc<Window>,

        /// Wake up the task.
        waker: Complete<Result<(), ExternalError>, TS>,
    },

    /// Drag-resize the window.
    DragResizeWindow {
        /// The window.
        window: TS::Rc<Window>,

        direction: ResizeDirection,

        /// Wake up the task.
        waker: Complete<Result<(), ExternalError>, TS>,
    },

    /// Set the cursor hit test.
    SetCursorHitTest {
        /// The window.
        window: TS::Rc<Window>,

        /// The cursor hit test.
        hit_test: bool,

        /// Wake up the task.
        waker: Complete<Result<(), ExternalError>, TS>,
    },

    /// Get the current monitor.
    CurrentMonitor {
        /// The window.
        window: TS::Rc<Window>,

        /// Wake up the task.
        waker: Complete<Option<MonitorHandle>, TS>,
    },
}

impl<TS: ThreadSafety> fmt::Debug for EventLoopOp<TS> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventLoopOp::BuildWindow { .. } => f
                .debug_struct("BuildWindow")
                .field("builder", &"...")
                .field("waker", &"...")
                .finish(),
            EventLoopOp::PrimaryMonitor(_) => f.debug_struct("PrimaryMonitor").finish(),
            EventLoopOp::AvailableMonitors(_) => f.debug_struct("AvailableMonitors").finish(),
            EventLoopOp::InnerPosition { .. } => f
                .debug_struct("InnerPosition")
                .field("window", &"...")
                .field("waker", &"...")
                .finish(),
            EventLoopOp::OuterPosition { .. } => f
                .debug_struct("OuterPosition")
                .field("window", &"...")
                .field("waker", &"...")
                .finish(),
            EventLoopOp::SetOuterPosition { .. } => f
                .debug_struct("SetOuterPosition")
                .field("window", &"...")
                .field("position", &"...")
                .field("waker", &"...")
                .finish(),
            EventLoopOp::InnerSize { .. } => f
                .debug_struct("InnerSize")
                .field("window", &"...")
                .field("waker", &"...")
                .finish(),
            EventLoopOp::OuterSize { .. } => f
                .debug_struct("OuterSize")
                .field("window", &"...")
                .field("waker", &"...")
                .finish(),
            EventLoopOp::SetMinInnerSize { .. } => f
                .debug_struct("SetMinInnerSize")
                .field("window", &"...")
                .field("size", &"...")
                .field("waker", &"...")
                .finish(),
            EventLoopOp::SetMaxInnerSize { .. } => f
                .debug_struct("SetMaxInnerSize")
                .field("window", &"...")
                .field("size", &"...")
                .field("waker", &"...")
                .finish(),
            _ => {
                // TODO: got bored
                f.debug_struct("EventLoopOp").finish()
            }
        }
    }
}

impl<TS: ThreadSafety> EventLoopOp<TS> {
    /// Run this event loop operation on a window target.
    fn run<T: 'static>(self, target: &winit::event_loop::EventLoopWindowTarget<T>) {
        match self {
            EventLoopOp::BuildWindow { builder, waker } => {
                waker.send(builder.into_winit_builder().build(target));
            }

            EventLoopOp::PrimaryMonitor(waker) => {
                waker.send(target.primary_monitor());
            }

            EventLoopOp::AvailableMonitors(waker) => {
                waker.send(target.available_monitors().collect());
            }

            EventLoopOp::InnerPosition { window, waker } => {
                waker.send(window.inner_position());
            }

            EventLoopOp::OuterPosition { window, waker } => {
                waker.send(window.outer_position());
            }

            EventLoopOp::SetOuterPosition {
                window,
                position,
                waker,
            } => {
                window.set_outer_position(position);
                waker.send(());
            }

            EventLoopOp::InnerSize { window, waker } => {
                waker.send(window.inner_size());
            }

            EventLoopOp::OuterSize { window, waker } => {
                waker.send(window.outer_size());
            }

            EventLoopOp::SetMinInnerSize {
                window,
                size,
                waker,
            } => {
                window.set_min_inner_size(size);
                waker.send(());
            }

            EventLoopOp::SetMaxInnerSize {
                window,
                size,
                waker,
            } => {
                window.set_max_inner_size(size);
                waker.send(());
            }

            EventLoopOp::ResizeIncrements { window, waker } => {
                waker.send(window.resize_increments());
            }

            EventLoopOp::SetResizeIncrements {
                window,
                size,
                waker,
            } => {
                window.set_resize_increments(size);
                waker.send(());
            }

            EventLoopOp::SetTitle {
                window,
                title,
                waker,
            } => {
                window.set_title(&title);
                waker.send(());
            }

            EventLoopOp::SetWindowIcon {
                window,
                icon,
                waker,
            } => {
                window.set_window_icon(icon);
                waker.send(());
            }

            EventLoopOp::Fullscreen { window, waker } => {
                waker.send(window.fullscreen());
            }

            EventLoopOp::SetFullscreen {
                window,
                fullscreen,
                waker,
            } => {
                window.set_fullscreen(fullscreen);
                waker.send(());
            }

            EventLoopOp::Maximized { window, waker } => {
                waker.send(window.is_maximized());
            }

            EventLoopOp::SetMaximized {
                window,
                maximized,
                waker,
            } => {
                window.set_maximized(maximized);
                waker.send(());
            }

            EventLoopOp::Minimized { window, waker } => {
                waker.send(window.is_minimized());
            }

            EventLoopOp::SetMinimized {
                window,
                minimized,
                waker,
            } => {
                window.set_minimized(minimized);
                waker.send(());
            }

            EventLoopOp::Visible { window, waker } => {
                waker.send(window.is_visible());
            }

            EventLoopOp::SetVisible {
                window,
                visible,
                waker,
            } => {
                window.set_visible(visible);
                waker.send(());
            }

            EventLoopOp::Decorated { window, waker } => {
                waker.send(window.is_decorated());
            }

            EventLoopOp::SetDecorated {
                window,
                decorated,
                waker,
            } => {
                window.set_decorations(decorated);
                waker.send(());
            }

            EventLoopOp::SetWindowLevel {
                window,
                level,
                waker,
            } => {
                window.set_window_level(level);
                waker.send(());
            }

            EventLoopOp::SetImeCursorArea {
                window,
                position,
                size,
                waker,
            } => {
                window.set_ime_cursor_area(position, size);
                waker.send(());
            }

            EventLoopOp::SetImeAllowed {
                window,
                allowed,
                waker,
            } => {
                window.set_ime_allowed(allowed);
                waker.send(());
            }

            EventLoopOp::SetImePurpose {
                window,
                purpose,
                waker,
            } => {
                window.set_ime_purpose(purpose);
                waker.send(());
            }

            EventLoopOp::FocusWindow { window, waker } => {
                window.focus_window();
                waker.send(());
            }

            EventLoopOp::Focused { window, waker } => {
                waker.send(window.has_focus());
            }

            EventLoopOp::RequestUserAttention {
                window,
                request_type,
                waker,
            } => {
                window.request_user_attention(request_type);
                waker.send(());
            }

            EventLoopOp::SetTheme {
                window,
                theme,
                waker,
            } => {
                window.set_theme(theme);
                waker.send(());
            }

            EventLoopOp::Theme { window, waker } => {
                waker.send(window.theme());
            }

            EventLoopOp::SetProtectedContent {
                window,
                protected,
                waker,
            } => {
                window.set_content_protected(protected);
                waker.send(());
            }

            EventLoopOp::Title { window, waker } => {
                waker.send(window.title());
            }

            EventLoopOp::SetCursorIcon {
                window,
                icon,
                waker,
            } => {
                window.set_cursor_icon(icon);
                waker.send(());
            }

            EventLoopOp::SetCursorGrab {
                window,
                mode,
                waker,
            } => {
                waker.send(window.set_cursor_grab(mode));
            }

            EventLoopOp::SetCursorVisible {
                window,
                visible,
                waker,
            } => {
                window.set_cursor_visible(visible);
                waker.send(());
            }

            EventLoopOp::DragWindow { window, waker } => {
                waker.send(window.drag_window());
            }

            EventLoopOp::DragResizeWindow {
                window,
                direction,
                waker,
            } => {
                waker.send(window.drag_resize_window(direction));
            }

            EventLoopOp::SetCursorHitTest {
                window,
                hit_test,
                waker,
            } => {
                waker.send(window.set_cursor_hittest(hit_test));
            }

            EventLoopOp::CurrentMonitor { window, waker } => {
                waker.send(window.current_monitor());
            }

            EventLoopOp::SetTransparent {
                window,
                transparent,
                waker,
            } => {
                window.set_transparent(transparent);
                waker.send(());
            }

            EventLoopOp::SetResizable {
                window,
                resizable,
                waker,
            } => {
                window.set_resizable(resizable);
                waker.send(());
            }

            EventLoopOp::Resizable { window, waker } => {
                waker.send(window.is_resizable());
            }

            EventLoopOp::SetCursorPosition {
                window,
                position,
                waker,
            } => {
                waker.send(window.set_cursor_position(position));
            }
        }
    }
}

pub(crate) struct GlobalRegistration<T: ThreadSafety> {
    pub(crate) resumed: Handler<(), T>,
    pub(crate) suspended: Handler<(), T>,
}

impl<TS: ThreadSafety> GlobalRegistration<TS> {
    pub(crate) fn new() -> Self {
        Self {
            resumed: Handler::new(),
            suspended: Handler::new(),
        }
    }
}
