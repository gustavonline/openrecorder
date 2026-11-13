#![allow(unexpected_cfgs)]
#[cfg(target_os = "macos")]
#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {}
use std::sync::{
    atomic::{AtomicI8, Ordering},
    Arc,
};
/// Requested only from the person's camera button. No prompt during app startup.
pub fn request_camera() -> Arc<AtomicI8> {
    super::event_log::event(
        "camera_permission_requested",
        serde_json::json!({"explicitInputAction":true}),
    );
    request(c"vide")
}
pub fn request_microphone() -> Arc<AtomicI8> {
    super::event_log::event(
        "microphone_permission_requested",
        serde_json::json!({"explicitInputAction":true}),
    );
    request(c"soun")
}
/// What an OS request may do for the grant read right now. A previous denial,
/// a restriction or a system-managed grant never produces a prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestDecision {
    AlreadyAllowed,
    Prompt,
    Refuse,
}
pub fn decide_input_request(grant: Grant) -> RequestDecision {
    match grant {
        Grant::Allowed | Grant::SystemManaged => RequestDecision::AlreadyAllowed,
        Grant::NotAsked => RequestDecision::Prompt,
        Grant::Denied | Grant::Restricted => RequestDecision::Refuse,
    }
}
/// AVAuthorizationStatus: 0 not determined, 1 restricted, 2 denied, 3 authorized.
pub fn grant_from_av_status(status: i64) -> Grant {
    match status {
        0 => Grant::NotAsked,
        1 => Grant::Restricted,
        2 => Grant::Denied,
        3 => Grant::Allowed,
        _ => Grant::SystemManaged,
    }
}
/// Screen Recording has no silent "denied" state: has_permission is false both
/// before the first request and after a refusal, so a request is made at most
/// once per process and only from a person's press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenDecision {
    Allowed,
    Prompt,
    OpenSettings,
}
pub fn decide_screen_request(has_permission: bool, already_asked: bool) -> ScreenDecision {
    if has_permission {
        ScreenDecision::Allowed
    } else if already_asked {
        ScreenDecision::OpenSettings
    } else {
        ScreenDecision::Prompt
    }
}
fn request(_kind: &std::ffi::CStr) -> Arc<AtomicI8> {
    let outcome = Arc::new(AtomicI8::new(0));
    #[cfg(target_os = "macos")]
    unsafe {
        use objc::runtime::Object;
        use objc::{class, msg_send, sel, sel_impl};
        let media: *mut Object = msg_send![class!(NSString), alloc];
        let media: *mut Object = msg_send![media,initWithUTF8String:_kind.as_ptr()];
        // The status is read here, at request time, never from a cached value.
        let status: i64 = msg_send![class!(AVCaptureDevice),authorizationStatusForMediaType:media];
        let decision = decide_input_request(grant_from_av_status(status));
        if decision == RequestDecision::AlreadyAllowed {
            outcome.store(1, Ordering::Relaxed);
        } else if decision == RequestDecision::Prompt {
            let result = outcome.clone();
            let handler = block::ConcreteBlock::new(move |granted: bool| {
                result.store(if granted { 1 } else { -1 }, Ordering::Relaxed);
            })
            .copy();
            let _: () = msg_send![class!(AVCaptureDevice),requestAccessForMediaType:media completionHandler:&*handler];
        } else {
            outcome.store(-1, Ordering::Relaxed);
        }
        let _: () = msg_send![media, release];
    }
    #[cfg(not(target_os = "macos"))]
    outcome.store(1, Ordering::Relaxed);
    outcome
}

/// Request shareable content only after the person presses Record.
/// Unlike a settings deep link, this actually asks ScreenCaptureKit for access.
pub fn request_screen() -> Arc<AtomicI8> {
    super::event_log::event(
        "screen_permission_requested",
        serde_json::json!({"explicitInputAction":true}),
    );
    let outcome = Arc::new(AtomicI8::new(0));
    #[cfg(target_os = "macos")]
    unsafe {
        use objc::runtime::Object;
        use objc::{class, msg_send, sel, sel_impl};
        let result = outcome.clone();
        let block = block::ConcreteBlock::new(move |_content: *mut Object, error: *mut Object| {
            result.store(if error.is_null() { 1 } else { -1 }, Ordering::Relaxed);
        })
        .copy();
        let _: () = msg_send![class!(SCShareableContent),getShareableContentExcludingDesktopWindows:false onScreenWindowsOnly:true completionHandler:&*block];
    }
    #[cfg(not(target_os = "macos"))]
    outcome.store(1, Ordering::Relaxed);
    outcome
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Grant {
    NotAsked,
    Allowed,
    Denied,
    Restricted,
    SystemManaged,
}
impl Grant {
    pub fn label(self) -> &'static str {
        match self {
            Self::NotAsked => "Not asked",
            Self::Allowed => "Allowed",
            Self::Denied => "Not allowed",
            Self::Restricted => "Restricted",
            Self::SystemManaged => "System-managed",
        }
    }
}
/// Silent authorization status. No discovery, grant requests, or device opens.
pub fn input_grant(camera: bool) -> Grant {
    #[cfg(target_os = "macos")]
    unsafe {
        use objc::runtime::Object;
        use objc::{class, msg_send, sel, sel_impl};
        let kind = if camera { c"vide" } else { c"soun" };
        let media: *mut Object = msg_send![class!(NSString), alloc];
        let media: *mut Object = msg_send![media,initWithUTF8String:kind.as_ptr()];
        let status: i64 = msg_send![class!(AVCaptureDevice),authorizationStatusForMediaType:media];
        let _: () = msg_send![media, release];
        grant_from_av_status(status)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = camera;
        Grant::SystemManaged
    }
}
impl Grant {
    /// The OS will not allow this input until the person changes System Settings.
    pub fn is_blocked(self) -> bool {
        matches!(self, Self::Denied | Self::Restricted)
    }
    pub fn can_open(self) -> bool {
        matches!(self, Self::Allowed | Self::SystemManaged)
    }
}
/// Passive statuses read together so one UI pass never mixes stale and fresh values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GrantSnapshot {
    pub screen: bool,
    pub camera: Grant,
    pub microphone: Grant,
}
pub fn snapshot() -> GrantSnapshot {
    GrantSnapshot {
        screen: scap::has_permission(),
        camera: input_grant(true),
        microphone: input_grant(false),
    }
}
pub fn input_settings(camera: bool) -> Result<(), String> {
    let mut cmd;
    if cfg!(target_os = "macos") {
        cmd = std::process::Command::new("/usr/bin/open");
        cmd.arg(if camera {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Camera"
        } else {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
        });
    } else if cfg!(target_os = "windows") {
        cmd = std::process::Command::new("explorer.exe");
        cmd.arg(if camera {
            "ms-settings:privacy-webcam"
        } else {
            "ms-settings:privacy-microphone"
        });
    } else {
        return Ok(());
    }
    cmd.spawn().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn screen_settings() -> Result<(), String> {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("/usr/bin/open");
        c.arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture");
        c
    } else if cfg!(target_os = "windows") {
        let mut c = std::process::Command::new("explorer.exe");
        c.arg("ms-settings:privacy-graphicscaptureprogrammatic");
        c
    } else {
        return Ok(());
    };
    cmd.spawn().map_err(|e| e.to_string())?;
    Ok(())
}
/// Recording is user-initiated work; release the activity when the take finishes.
pub struct RecordingActivity {
    #[cfg(target_os = "macos")]
    token: *mut objc::runtime::Object,
}
impl RecordingActivity {
    pub fn begin() -> Self {
        #[cfg(target_os = "macos")]
        unsafe {
            use objc::runtime::Object;
            use objc::{class, msg_send, sel, sel_impl};
            let pool: *mut Object = msg_send![class!(NSAutoreleasePool), new];
            let info: *mut Object = msg_send![class!(NSProcessInfo), processInfo];
            let reason: *mut Object = msg_send![class!(NSString), alloc];
            let reason: *mut Object =
                msg_send![reason,initWithUTF8String:c"Recording a local video".as_ptr()];
            let token: *mut Object =
                msg_send![info,beginActivityWithOptions:0x00ff_ffffu64 reason:reason];
            let _: () = msg_send![token, retain];
            let _: () = msg_send![reason, release];
            let _: () = msg_send![pool, drain];
            return Self { token };
        }
        #[cfg(not(target_os = "macos"))]
        Self {}
    }
}
impl Drop for RecordingActivity {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        unsafe {
            use objc::runtime::Object;
            use objc::{class, msg_send, sel, sel_impl};
            let pool: *mut Object = msg_send![class!(NSAutoreleasePool), new];
            let info: *mut Object = msg_send![class!(NSProcessInfo), processInfo];
            let _: () = msg_send![info,endActivity:self.token];
            let _: () = msg_send![self.token, release];
            let _: () = msg_send![pool, drain];
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_a_never_asked_input_can_prompt() {
        assert_eq!(decide_input_request(Grant::NotAsked), RequestDecision::Prompt);
        for grant in [Grant::Denied, Grant::Restricted] {
            assert_eq!(decide_input_request(grant), RequestDecision::Refuse);
            assert!(grant.is_blocked() && !grant.can_open());
        }
        for grant in [Grant::Allowed, Grant::SystemManaged] {
            assert_eq!(decide_input_request(grant), RequestDecision::AlreadyAllowed);
            assert!(grant.can_open() && !grant.is_blocked());
        }
        assert!(!Grant::NotAsked.can_open() && !Grant::NotAsked.is_blocked());
    }
    #[test]
    fn av_status_maps_without_touching_the_os() {
        assert_eq!(grant_from_av_status(0), Grant::NotAsked);
        assert_eq!(grant_from_av_status(1), Grant::Restricted);
        assert_eq!(grant_from_av_status(2), Grant::Denied);
        assert_eq!(grant_from_av_status(3), Grant::Allowed);
        assert_eq!(grant_from_av_status(99), Grant::SystemManaged);
    }
    #[test]
    fn screen_request_happens_once_and_never_after_a_refusal() {
        assert_eq!(decide_screen_request(true, false), ScreenDecision::Allowed);
        assert_eq!(decide_screen_request(true, true), ScreenDecision::Allowed);
        assert_eq!(decide_screen_request(false, false), ScreenDecision::Prompt);
        assert_eq!(decide_screen_request(false, true), ScreenDecision::OpenSettings);
    }
}
