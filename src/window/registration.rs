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

//! Registration of the window into the reactor.

use crate::dpi::PhysicalSize;
use crate::handler::Handler;
use crate::sync::ThreadSafety;
use crate::Event;

use winit::dpi::PhysicalPosition;
use winit::event::{
    AxisId, DeviceId, ElementState, Ime, MouseButton, MouseScrollDelta, Touch, TouchPhase,
    WindowEvent,
};
use winit::keyboard::ModifiersState;
use winit::window::Theme;

#[derive(Clone)]
pub struct KeyboardInput {
    pub device_id: DeviceId,
    pub event: winit::event::KeyEvent,
    pub is_synthetic: bool,
}

#[derive(Clone)]
pub struct CursorMoved {
    pub device_id: DeviceId,
    pub position: PhysicalPosition<f64>,
}

#[derive(Clone)]
pub struct MouseWheel {
    pub device_id: DeviceId,
    pub delta: MouseScrollDelta,
    pub phase: TouchPhase,
}

#[derive(Clone)]
pub struct MouseInput {
    pub device_id: DeviceId,
    pub state: ElementState,
    pub button: MouseButton,
}

#[derive(Clone)]
pub struct TouchpadMagnify {
    pub device_id: DeviceId,
    pub delta: f64,
    pub phase: TouchPhase,
}

#[derive(Clone)]
pub struct TouchpadRotate {
    pub device_id: DeviceId,
    pub delta: f32,
    pub phase: TouchPhase,
}

#[derive(Clone)]
pub struct TouchpadPressure {
    pub device_id: DeviceId,
    pub pressure: f32,
    pub stage: i64,
}

#[derive(Clone)]
pub struct AxisMotion {
    pub device_id: DeviceId,
    pub axis: AxisId,
    pub value: f64,
}

pub struct ScaleFactor;

pub struct ScaleFactorChanging<'a> {
    pub scale_factor: f64,
    pub inner_size_writer: &'a mut winit::event::InnerSizeWriter,
}

#[derive(Clone)]
pub struct ScaleFactorChanged {
    pub scale_factor: f64,
    pub inner_size_writer: winit::event::InnerSizeWriter,
}

impl Event for ScaleFactor {
    type Clonable = ScaleFactorChanged;
    type Unique<'a> = ScaleFactorChanging<'a>;

    fn downgrade(unique: &mut Self::Unique<'_>) -> Self::Clonable {
        ScaleFactorChanged {
            scale_factor: unique.scale_factor,
            inner_size_writer: unique.inner_size_writer.clone(),
        }
    }
}

pub(crate) struct Registration<U, TS: ThreadSafety> {
    /// `RedrawRequested`
    pub(crate) redraw_requested: Handler<(), U, TS>,

    /// `Event::CloseRequested`.
    pub(crate) close_requested: Handler<(), U, TS>,

    /// `Event::Resized`.
    pub(crate) resized: Handler<PhysicalSize<u32>, U, TS>,

    /// `Event::Moved`.
    pub(crate) moved: Handler<PhysicalPosition<i32>, U, TS>,

    /// `Event::Destroyed`.
    pub(crate) destroyed: Handler<(), U, TS>,

    /// `Event::Focused`.
    pub(crate) focused: Handler<bool, U, TS>,

    /// `Event::ReceivedCharacter`.
    pub(crate) received_character: Handler<char, U, TS>,

    /// `Event::KeyboardInput`.
    pub(crate) keyboard_input: Handler<KeyboardInput, U, TS>,

    /// `Event::ModifiersState`
    pub(crate) modifiers_changed: Handler<ModifiersState, U, TS>,

    /// `Event::Ime`
    pub(crate) ime: Handler<Ime, U, TS>,

    /// `Event::CursorMoved`
    pub(crate) cursor_moved: Handler<CursorMoved, U, TS>,

    /// `Event::CursorEntered`
    pub(crate) cursor_entered: Handler<DeviceId, U, TS>,

    /// `Event::CursorLeft`
    pub(crate) cursor_left: Handler<DeviceId, U, TS>,

    /// `Event::MouseWheel`
    pub(crate) mouse_wheel: Handler<MouseWheel, U, TS>,

    /// `Event::MouseInput`
    pub(crate) mouse_input: Handler<MouseInput, U, TS>,

    /// `Event::TouchpadMagnify`
    pub(crate) touchpad_magnify: Handler<TouchpadMagnify, U, TS>,

    /// `Event::SmartMagnify`.
    pub(crate) smart_magnify: Handler<DeviceId, U, TS>,

    /// `Event::TouchpadRotate`
    pub(crate) touchpad_rotate: Handler<TouchpadRotate, U, TS>,

    /// `Event::TouchpadPressure`
    pub(crate) touchpad_pressure: Handler<TouchpadPressure, U, TS>,

    /// `Event::AxisMotion`
    pub(crate) axis_motion: Handler<AxisMotion, U, TS>,

    /// `Event::Touch`
    pub(crate) touch: Handler<Touch, U, TS>,

    /// `Event::ScaleFactorChanged`
    pub(crate) scale_factor_changed: Handler<ScaleFactor, U, TS>,

    /// `Event::ThemeChanged`
    pub(crate) theme_changed: Handler<Theme, U, TS>,

    /// `Event::Occluded`
    pub(crate) occluded: Handler<bool, U, TS>,
}

impl<U, TS: ThreadSafety> Registration<U, TS> {
    pub(crate) fn new() -> Self {
        Self {
            close_requested: Handler::new(),
            resized: Handler::new(),
            redraw_requested: Handler::new(),
            moved: Handler::new(),
            destroyed: Handler::new(),
            focused: Handler::new(),
            keyboard_input: Handler::new(),
            received_character: Handler::new(),
            modifiers_changed: Handler::new(),
            ime: Handler::new(),
            cursor_entered: Handler::new(),
            cursor_left: Handler::new(),
            cursor_moved: Handler::new(),
            axis_motion: Handler::new(),
            scale_factor_changed: Handler::new(),
            smart_magnify: Handler::new(),
            theme_changed: Handler::new(),
            touch: Handler::new(),
            touchpad_magnify: Handler::new(),
            touchpad_pressure: Handler::new(),
            touchpad_rotate: Handler::new(),
            mouse_input: Handler::new(),
            mouse_wheel: Handler::new(),
            occluded: Handler::new(),
        }
    }

    pub(crate) async fn signal(&self, user_data: &mut U, event: WindowEvent) {
        match event {
            WindowEvent::RedrawRequested => {
                self.redraw_requested.run_with(&mut (), user_data).await;
            }
            WindowEvent::CloseRequested => self.close_requested.run_with(&mut (), user_data).await,
            WindowEvent::Resized(mut size) => self.resized.run_with(&mut size, user_data).await,
            WindowEvent::Moved(mut posn) => self.moved.run_with(&mut posn, user_data).await,
            WindowEvent::AxisMotion {
                device_id,
                axis,
                value,
            } => {
                self.axis_motion
                    .run_with(
                        &mut AxisMotion {
                            device_id,
                            axis,
                            value,
                        },
                        user_data,
                    )
                    .await
            }
            WindowEvent::CursorEntered { mut device_id } => {
                self.cursor_entered
                    .run_with(&mut device_id, user_data)
                    .await
            }
            WindowEvent::CursorLeft { mut device_id } => {
                self.cursor_left.run_with(&mut device_id, user_data).await
            }
            WindowEvent::CursorMoved {
                device_id,
                position,
                ..
            } => {
                self.cursor_moved
                    .run_with(
                        &mut CursorMoved {
                            device_id,
                            position,
                        },
                        user_data,
                    )
                    .await
            }
            WindowEvent::Destroyed => self.destroyed.run_with(&mut (), user_data).await,
            WindowEvent::Focused(mut foc) => self.focused.run_with(&mut foc, user_data).await,
            WindowEvent::Ime(mut ime) => self.ime.run_with(&mut ime, user_data).await,
            WindowEvent::KeyboardInput {
                device_id,
                event,
                is_synthetic,
            } => {
                self.keyboard_input
                    .run_with(
                        &mut KeyboardInput {
                            device_id,
                            event,
                            is_synthetic,
                        },
                        user_data,
                    )
                    .await
            }
            WindowEvent::ModifiersChanged(mods) => {
                self.modifiers_changed
                    .run_with(&mut mods.state(), user_data)
                    .await
            }
            WindowEvent::MouseInput {
                device_id,
                state,
                button,
                ..
            } => {
                self.mouse_input
                    .run_with(
                        &mut MouseInput {
                            device_id,
                            state,
                            button,
                        },
                        user_data,
                    )
                    .await
            }
            WindowEvent::MouseWheel {
                device_id,
                delta,
                phase,
                ..
            } => {
                self.mouse_wheel
                    .run_with(
                        &mut MouseWheel {
                            device_id,
                            delta,
                            phase,
                        },
                        user_data,
                    )
                    .await
            }
            WindowEvent::Occluded(mut occ) => self.occluded.run_with(&mut occ, user_data).await,
            WindowEvent::ScaleFactorChanged {
                scale_factor,
                mut inner_size_writer,
            } => {
                self.scale_factor_changed
                    .run_with(
                        &mut ScaleFactorChanging {
                            scale_factor,
                            inner_size_writer: &mut inner_size_writer,
                        },
                        user_data,
                    )
                    .await
            }
            WindowEvent::SmartMagnify { mut device_id } => {
                self.smart_magnify.run_with(&mut device_id, user_data).await
            }
            WindowEvent::ThemeChanged(mut theme) => {
                self.theme_changed.run_with(&mut theme, user_data).await
            }
            WindowEvent::Touch(mut touch) => self.touch.run_with(&mut touch, user_data).await,
            WindowEvent::TouchpadMagnify {
                device_id,
                delta,
                phase,
            } => {
                self.touchpad_magnify
                    .run_with(
                        &mut TouchpadMagnify {
                            device_id,
                            delta,
                            phase,
                        },
                        user_data,
                    )
                    .await
            }
            WindowEvent::TouchpadPressure {
                device_id,
                pressure,
                stage,
            } => {
                self.touchpad_pressure
                    .run_with(
                        &mut TouchpadPressure {
                            device_id,
                            pressure,
                            stage,
                        },
                        user_data,
                    )
                    .await
            }
            WindowEvent::TouchpadRotate {
                device_id,
                delta,
                phase,
            } => {
                self.touchpad_rotate
                    .run_with(
                        &mut TouchpadRotate {
                            device_id,
                            delta,
                            phase,
                        },
                        user_data,
                    )
                    .await
            }
            _ => {}
        }
    }
}
