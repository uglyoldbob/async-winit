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

// This file is partially derived from `winit`, which was originally created by Pierre Krieger and
// contributers. It was originally released under the MIT license.

use crate::event_loop::EventLoop;
use crate::filter::{Filter, ReturnOrFinish};
use crate::sync::ThreadSafety;

use futures_lite::pin;

use std::future::Future;

/// Additional methods on [`EventLoop`] to return control flow to the caller.
pub trait EventLoopExtRunOnDemand {
    /// Initializes the `winit` event loop.
    ///
    /// Unlike [`EventLoop::block_on`], this function accepts non-`'static` (i.e. non-`move`) closures
    /// and returns control flow to the caller when `control_flow` is set to [`ControlFlow::Exit`].
    ///
    /// [`ControlFlow::Exit`]: crate::event_loop::ControlFlow::Exit
    fn block_on_demand<U, F>(
        &mut self,
        user_data: &mut U,
        future: F,
    ) -> ReturnOrFinish<Result<(), winit::error::EventLoopError>, F::Output>
    where
        F: Future;
}

impl<TS: ThreadSafety> EventLoopExtRunOnDemand for EventLoop<TS> {
    fn block_on_demand<U, F>(
        &mut self,
        user_data: &mut U,
        fut: F,
    ) -> ReturnOrFinish<Result<(), winit::error::EventLoopError>, F::Output>
    where
        F: Future,
    {
        use winit::platform::run_on_demand::EventLoopExtRunOnDemand as _;

        let inner = &mut self.inner;

        pin!(fut);

        let mut filter = Filter::<U, TS>::new(inner);

        let mut output = None;
        let exit = inner.run_on_demand({
            let output = &mut output;
            move |event, elwt| match filter.handle_event(user_data, fut.as_mut(), event, elwt) {
                ReturnOrFinish::FutureReturned(out) => {
                    *output = Some(out);
                    elwt.exit();
                }

                ReturnOrFinish::Output(()) => {}
            }
        });

        match output {
            Some(output) => ReturnOrFinish::FutureReturned(output),
            None => ReturnOrFinish::Output(exit),
        }
    }
}
