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

//! Platform specific code.

#[cfg(android_platform)]
pub mod android;

#[cfg(ios_platform)]
pub mod ios;

#[cfg(macos_platform)]
pub mod macos;

#[cfg(orbital_platform)]
pub mod orbital;

#[cfg(x11_platform)]
pub mod x11;

#[cfg(wayland_platform)]
pub mod wayland;

#[cfg(windows)]
pub mod windows;

#[cfg(any(windows, x11_platform, wayland_platform))]
pub mod run_return;

cfg_if::cfg_if! {
    if #[cfg(android_platform)] {
        pub(crate) use android::PlatformSpecific;
    } else if #[cfg(ios_platform)] {
        pub(crate) use ios::PlatformSpecific;
    } else if #[cfg(macos_platform)] {
        pub(crate) use macos::PlatformSpecific;
    } else if #[cfg(orbital_platform)] {
        pub(crate) use orbital::PlatformSpecific;
    } else if #[cfg(any(x11_platform, wayland_platform))] {
        #[cfg(all(feature = "x11", not(feature = "wayland")))]
        pub(crate) use x11::PlatformSpecific;

        #[cfg(all(not(feature = "x11"), feature = "wayland"))]
        pub(crate) use wayland::PlatformSpecific;

        #[cfg(all(feature = "x11", feature = "wayland"))]
        mod free_unix;
        #[cfg(all(feature = "x11", feature = "wayland"))]
        pub(crate) use free_unix::PlatformSpecific;
    } else if #[cfg(windows)] {
        pub(crate) use windows::PlatformSpecific;
    }
}

mod __private {
    use crate::event_loop::EventLoopBuilder;
    use crate::window::{Window, WindowBuilder};

    #[doc(hidden)]
    pub struct Internal(());

    macro_rules! sealed_trait {
        ($($name: ident $tname: ident)*) => {$(
            #[doc(hidden)]
            pub trait $tname {
                fn __sealed_marker(i: Internal);
            }

            impl $tname for $name {
                fn __sealed_marker(_: Internal) {}
            }
        )*}
    }

    macro_rules! sealed_trait_with_gen {
        ($($name: ident $tname: ident)*) => {$(
            #[doc(hidden)]
            pub trait $tname {
                fn __sealed_marker(i: Internal);
            }

            impl<TS: crate::sync::ThreadSafety> $tname for $name<TS> {
                fn __sealed_marker(_: Internal) {}
            }
        )*}
    }

    sealed_trait! {
        EventLoopBuilder EventLoopBuilderPrivate
        WindowBuilder WindowBuilderPrivate
    }

    sealed_trait_with_gen! {
        //EventLoopWindowTarget EventLoopWindowTargetPrivate
        //EventLoop EventLoopPrivate
        Window WindowPrivate
    }
}
