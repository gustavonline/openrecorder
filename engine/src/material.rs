//! Optional platform material. Capture and all product flows remain shared Rust.
pub fn apply(window: impl raw_window_handle::HasWindowHandle) -> bool {
    #[cfg(target_os = "macos")]
    {
        // Keep winit's content view and responder/notification ownership intact.
        // Reparenting it into an effect view caused a focus-loss use-after-free.
        // The shared renderer supplies the opaque neutral surface instead.
        let _ = window;
        return false;
    }
    #[cfg(target_os = "windows")]
    {
        return window_vibrancy::apply_mica(&window, None).is_ok();
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = window;
        false
    }
}
