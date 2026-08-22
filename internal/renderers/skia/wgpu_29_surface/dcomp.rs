// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! DirectComposition presentation for Windows.
//!
//! A swapchain created straight onto an HWND can only ever advertise
//! `CompositeAlphaMode::Opaque` -- see the `SurfaceTarget::WndHandle` arm of
//! `wgpu_hal::dx12`'s surface capabilities. Anything the window draws with a
//! translucent alpha therefore composites against black rather than the desktop.
//!
//! Presenting through an `IDCompositionVisual` instead unlocks the premultiplied
//! alpha modes. The window has to be created with `WS_EX_NOREDIRECTIONBITMAP` so
//! it has no redirection surface for DWM to composite in place of ours; winit
//! does that when `with_no_redirection_bitmap(true)` is set, and correspondingly
//! skips its `DwmEnableBlurBehindWindow` call, which is only relevant to the
//! redirection-surface route.

use i_slint_core::platform::PlatformError;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};

/// The composition chain backing one window.
///
/// Field order matters for teardown: the visual is released before the target
/// that roots it, and the target before the device that created it.
pub struct Composition {
    pub visual: IDCompositionVisual,
    _target: IDCompositionTarget,
    _device: IDCompositionDevice,
}

/// Build a composition chain for `window_handle`, if it is a Win32 window.
///
/// Returns `Ok(None)` when the handle isn't Win32 (nothing to do, caller falls
/// back to presenting onto the window directly) and `Err` when the handle is
/// Win32 but DirectComposition could not be set up.
pub fn create(
    window_handle: &dyn HasWindowHandle,
) -> Result<Option<Composition>, PlatformError> {
    let handle = window_handle
        .window_handle()
        .map_err(|e| PlatformError::from(format!("Error obtaining window handle: {e}")))?;

    let RawWindowHandle::Win32(win32_handle) = handle.as_raw() else {
        return Ok(None);
    };

    let hwnd = HWND(win32_handle.hwnd.get() as *mut core::ffi::c_void);

    // Safety: `hwnd` comes from a live window handle, borrowed for this call.
    // Passing a null DXGI device lets DirectComposition pick the device itself,
    // which avoids having to construct one before wgpu has made its adapter.
    unsafe {
        let device: IDCompositionDevice = DCompositionCreateDevice(None).map_err(|e| {
            PlatformError::from(format!("Error creating DirectComposition device: {e}"))
        })?;

        // `false`: leave the visual behind any topmost windows, matching how a
        // regular window composites.
        let target = device.CreateTargetForHwnd(hwnd, false).map_err(|e| {
            PlatformError::from(format!("Error creating DirectComposition target: {e}"))
        })?;

        let visual = device.CreateVisual().map_err(|e| {
            PlatformError::from(format!("Error creating DirectComposition visual: {e}"))
        })?;

        target.SetRoot(&visual).map_err(|e| {
            PlatformError::from(format!("Error setting DirectComposition root: {e}"))
        })?;

        // The tree has to be committed before it will present anything. The
        // swapchain is attached to the visual by wgpu afterwards; committing
        // again is not required for content updates, only for tree changes.
        device.Commit().map_err(|e| {
            PlatformError::from(format!("Error committing DirectComposition device: {e}"))
        })?;

        Ok(Some(Composition { visual, _target: target, _device: device }))
    }
}
