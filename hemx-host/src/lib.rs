#![cfg_attr(not(feature = "std"), no_std)]

//! Typed host capability contract for hemx integrations.
//!
//! `hemx-host` describes the boundary between a hemx app and the browser,
//! PWA, WebView, or native shell that can perform device/host work. It does
//! not render UI, own application state, or define provider policy. Host
//! results are facts for app code to handle before returning normal hemx
//! effects.

extern crate alloc;

/// Optional browser/PWA host adapter source.
///
/// The adapter exposes `window.hemxBrowserHost.perform(call)` for the serde JSON
/// shape of [`HostCall`] and returns the serde JSON shape of [`HostEvent`]. It
/// only uses browser host APIs such as `navigator.share` and `navigator.vibrate`;
/// it does not inspect or mutate the DOM.
pub const BROWSER_HOST_JS: &str = include_str!("../runtime/browser-host.js");

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

/// A stable capability name understood by an app and one or more host adapters.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum Capability {
    Haptics,
    Microphone,
    Camera,
    Share,
    SecureStorage,
    Notifications,
    Clipboard,
    FilePicker,
    Geolocation,
    Custom(String),
}

impl Capability {
    pub fn custom(name: impl Into<String>) -> Self {
        Self::Custom(name.into())
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Haptics => "haptics",
            Self::Microphone => "microphone",
            Self::Camera => "camera",
            Self::Share => "share",
            Self::SecureStorage => "secure_storage",
            Self::Notifications => "notifications",
            Self::Clipboard => "clipboard",
            Self::FilePicker => "file_picker",
            Self::Geolocation => "geolocation",
            Self::Custom(name) => name.as_str(),
        }
    }

    /// Capabilities that require a user-facing permission reason in the app
    /// manifest before standard hosts may expose them.
    pub fn needs_permission_reason(&self) -> bool {
        matches!(
            self,
            Self::Microphone
                | Self::Camera
                | Self::SecureStorage
                | Self::Notifications
                | Self::FilePicker
                | Self::Geolocation
        )
    }
}

/// The only shapes a host capability may take.
///
/// Keeping the shape set small prevents host plugins from becoming a second app
/// runtime.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum CapabilityShape {
    Fire,
    Request,
    Stream,
    Schedule,
}

/// One declared capability use in an app manifest or host profile.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CapabilityUse {
    pub capability: Capability,
    pub shape: CapabilityShape,
    pub reason: Option<String>,
}

impl CapabilityUse {
    pub fn new(capability: Capability, shape: CapabilityShape) -> Self {
        Self {
            capability,
            shape,
            reason: None,
        }
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    fn matches(&self, capability: &Capability, shape: CapabilityShape) -> bool {
        self.capability == *capability && self.shape == shape
    }
}

/// App-owned declaration of host capabilities it may request.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CapabilityManifest {
    pub uses: Vec<CapabilityUse>,
}

impl CapabilityManifest {
    pub fn new(uses: impl Into<Vec<CapabilityUse>>) -> Self {
        Self { uses: uses.into() }
    }

    pub fn allows(&self, capability: &Capability, shape: CapabilityShape) -> bool {
        self.uses.iter().any(|use_| use_.matches(capability, shape))
    }

    pub fn check(&self, host: &HostProfile) -> Result<(), HostCheckError> {
        for use_ in &self.uses {
            if use_.capability.needs_permission_reason()
                && use_
                    .reason
                    .as_ref()
                    .map(|reason| reason.trim().is_empty())
                    .unwrap_or(true)
            {
                return Err(HostCheckError::MissingPermissionReason {
                    capability: use_.capability.clone(),
                });
            }

            if !host.supports(&use_.capability, use_.shape) {
                return Err(HostCheckError::UnsupportedCapability {
                    capability: use_.capability.clone(),
                    shape: use_.shape,
                    host: host.name.clone(),
                });
            }
        }

        Ok(())
    }

    pub fn validate_call(&self, host: &HostProfile, call: &HostCall) -> Result<(), HostCheckError> {
        let capability = call.capability();
        let shape = call.shape();

        if !self.allows(&capability, shape) {
            return Err(HostCheckError::UndeclaredCapability { capability, shape });
        }

        if !host.supports(&capability, shape) {
            return Err(HostCheckError::UnsupportedCapability {
                capability,
                shape,
                host: host.name.clone(),
            });
        }

        Ok(())
    }
}

/// Capabilities exposed by one concrete host adapter.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HostProfile {
    pub name: String,
    pub supports: Vec<CapabilityUse>,
}

impl HostProfile {
    pub fn new(name: impl Into<String>, supports: impl Into<Vec<CapabilityUse>>) -> Self {
        Self {
            name: name.into(),
            supports: supports.into(),
        }
    }

    pub fn supports(&self, capability: &Capability, shape: CapabilityShape) -> bool {
        self.supports
            .iter()
            .any(|use_| use_.matches(capability, shape))
    }
}

/// Browser/PWA host profile for the optional `BROWSER_HOST_JS` adapter.
///
/// Runtime feature availability is still checked by the JavaScript adapter;
/// this profile records the contract shapes the adapter owns.
pub fn browser_pwa_host_profile() -> HostProfile {
    HostProfile::new(
        "browser-pwa",
        [
            CapabilityUse::new(Capability::Haptics, CapabilityShape::Fire),
            CapabilityUse::new(Capability::Share, CapabilityShape::Request),
        ],
    )
}

/// Native-shell-shaped profile used by WebView adapters that expose device APIs
/// through the same host call/event contract.
pub fn native_shell_host_profile(name: impl Into<String>) -> HostProfile {
    HostProfile::new(
        name,
        [
            CapabilityUse::new(Capability::Haptics, CapabilityShape::Fire),
            CapabilityUse::new(Capability::Share, CapabilityShape::Request),
            CapabilityUse::new(Capability::Microphone, CapabilityShape::Stream),
            CapabilityUse::new(Capability::Notifications, CapabilityShape::Schedule),
        ],
    )
}

/// A host-check failure that can be reported by build tooling, tests, or a host
/// adapter before executing a capability call.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum HostCheckError {
    UndeclaredCapability {
        capability: Capability,
        shape: CapabilityShape,
    },
    UnsupportedCapability {
        capability: Capability,
        shape: CapabilityShape,
        host: String,
    },
    MissingPermissionReason {
        capability: Capability,
    },
}

impl core::fmt::Display for HostCheckError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UndeclaredCapability { capability, shape } => write!(
                f,
                "host capability `{}` with shape {:?} is used but not declared",
                capability.as_str(),
                shape
            ),
            Self::UnsupportedCapability {
                capability,
                shape,
                host,
            } => write!(
                f,
                "host `{host}` does not support capability `{}` with shape {:?}",
                capability.as_str(),
                shape
            ),
            Self::MissingPermissionReason { capability } => write!(
                f,
                "host capability `{}` requires a permission reason",
                capability.as_str()
            ),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for HostCheckError {}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct HostCallId(pub String);

impl HostCallId {
    pub fn new(value: impl ToString) -> Self {
        Self(value.to_string())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct HostStreamId(pub String);

impl HostStreamId {
    pub fn new(value: impl ToString) -> Self {
        Self(value.to_string())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum HapticPattern {
    Selection,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SharePayload {
    pub title: Option<String>,
    pub text: Option<String>,
    pub url: Option<String>,
}

impl SharePayload {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            title: None,
            text: Some(text.into()),
            url: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MicrophoneConfig {
    pub mime_type: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NotificationRequest {
    pub title: String,
    pub body: Option<String>,
}

/// Typed calls that app code may ask a host adapter to perform.
///
/// Adapters execute these calls and return [`HostEvent`] values. They must not
/// mutate DOM or application/domain state themselves.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum HostCall {
    Haptic {
        id: HostCallId,
        pattern: HapticPattern,
    },
    Share {
        id: HostCallId,
        payload: SharePayload,
    },
    StartMicrophone {
        stream: HostStreamId,
        config: MicrophoneConfig,
    },
    StopStream {
        stream: HostStreamId,
    },
    ScheduleNotification {
        id: HostCallId,
        notification: NotificationRequest,
    },
    Custom {
        id: HostCallId,
        capability: Capability,
        shape: CapabilityShape,
        op: String,
        payload: Vec<u8>,
    },
}

impl HostCall {
    pub fn capability(&self) -> Capability {
        match self {
            Self::Haptic { .. } => Capability::Haptics,
            Self::Share { .. } => Capability::Share,
            Self::StartMicrophone { .. } | Self::StopStream { .. } => Capability::Microphone,
            Self::ScheduleNotification { .. } => Capability::Notifications,
            Self::Custom { capability, .. } => capability.clone(),
        }
    }

    pub fn shape(&self) -> CapabilityShape {
        match self {
            Self::Haptic { .. } => CapabilityShape::Fire,
            Self::Share { .. } => CapabilityShape::Request,
            Self::StartMicrophone { .. } | Self::StopStream { .. } => CapabilityShape::Stream,
            Self::ScheduleNotification { .. } => CapabilityShape::Schedule,
            Self::Custom { shape, .. } => *shape,
        }
    }
}

/// Boring, typed host failure classes that app code can handle before it emits UI.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum HostFailureKind {
    PermissionDenied,
    Timeout,
    Unavailable,
    Error,
}

/// One host failure result shape for denied, timeout, unavailable, and error cases.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HostFailure {
    pub id: Option<HostCallId>,
    pub capability: Option<Capability>,
    pub kind: HostFailureKind,
    pub message: String,
}

impl HostFailure {
    pub fn new(kind: HostFailureKind, message: impl Into<String>) -> Self {
        Self {
            id: None,
            capability: None,
            kind,
            message: message.into(),
        }
    }

    pub fn with_id(mut self, id: HostCallId) -> Self {
        self.id = Some(id);
        self
    }

    pub fn with_capability(mut self, capability: Capability) -> Self {
        self.capability = Some(capability);
        self
    }
}

/// Facts and results produced by a host adapter.
///
/// App code decides what a host event means for the product/domain before any
/// hemx UI effect is returned.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum HostEvent {
    Acknowledged {
        id: HostCallId,
    },
    Failed(HostFailure),
    ShareCompleted {
        id: HostCallId,
        completed: bool,
    },
    StreamChunk {
        stream: HostStreamId,
        bytes: Vec<u8>,
        mime_type: Option<String>,
    },
    StreamEnded {
        stream: HostStreamId,
    },
    NotificationFired {
        id: HostCallId,
        action: Option<String>,
    },
    Custom {
        id: Option<HostCallId>,
        capability: Capability,
        payload: Vec<u8>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use hemx_core::{event, Effect, IntoEffect, Payload, Slot};

    fn web_host() -> HostProfile {
        browser_pwa_host_profile()
    }

    fn native_shell_host() -> HostProfile {
        native_shell_host_profile("ios-android-webview-test")
    }

    #[test]
    fn capability_names_and_standard_profile_identity_are_stable() {
        let names = [
            (Capability::Haptics, "haptics"),
            (Capability::Microphone, "microphone"),
            (Capability::Camera, "camera"),
            (Capability::Share, "share"),
            (Capability::SecureStorage, "secure_storage"),
            (Capability::Notifications, "notifications"),
            (Capability::Clipboard, "clipboard"),
            (Capability::FilePicker, "file_picker"),
            (Capability::Geolocation, "geolocation"),
            (Capability::custom("vendor.camera"), "vendor.camera"),
        ];

        for (capability, expected) in names {
            assert_eq!(capability.as_str(), expected);
        }
        assert_eq!(web_host().name, "browser-pwa");
    }

    #[test]
    fn browser_host_js_is_a_thin_host_adapter_not_a_dom_runtime() {
        assert!(BROWSER_HOST_JS.contains("window.hemxBrowserHost"));
        assert!(BROWSER_HOST_JS.contains("navigator.vibrate"));
        assert!(BROWSER_HOST_JS.contains("navigator.share"));
        assert!(
            BROWSER_HOST_JS.contains("return { ShareCompleted: { id: data.id, completed: true } }")
        );
        assert!(BROWSER_HOST_JS.contains("kind, message"));
        assert!(BROWSER_HOST_JS.contains("\"Unavailable\""));
        assert!(!BROWSER_HOST_JS.contains("querySelector"));
        assert!(!BROWSER_HOST_JS.contains("innerHTML"));
        assert!(!BROWSER_HOST_JS.contains("classList"));
        assert!(!BROWSER_HOST_JS.contains("dispatchEvent"));
        assert!(!BROWSER_HOST_JS.contains("localStorage"));
    }

    #[test]
    fn manifest_checks_declared_permissions_and_host_support() {
        let manifest = CapabilityManifest::new([CapabilityUse::new(
            Capability::Microphone,
            CapabilityShape::Stream,
        )]);

        assert_eq!(
            manifest.check(&web_host()),
            Err(HostCheckError::MissingPermissionReason {
                capability: Capability::Microphone,
            })
        );

        let manifest = CapabilityManifest::new([CapabilityUse::new(
            Capability::Share,
            CapabilityShape::Request,
        )]);

        assert_eq!(manifest.check(&web_host()), Ok(()));
    }

    #[test]
    fn manifest_and_host_support_require_the_exact_capability_shape() {
        let wrong_shape_host = HostProfile::new(
            "wrong-shape",
            [CapabilityUse::new(Capability::Share, CapabilityShape::Fire)],
        );
        let wrong_capability_host = HostProfile::new(
            "wrong-capability",
            [CapabilityUse::new(
                Capability::Haptics,
                CapabilityShape::Request,
            )],
        );
        let manifest = CapabilityManifest::new([CapabilityUse::new(
            Capability::Share,
            CapabilityShape::Request,
        )]);

        assert!(!wrong_shape_host.supports(&Capability::Share, CapabilityShape::Request));
        assert!(!wrong_capability_host.supports(&Capability::Share, CapabilityShape::Request));
        assert_eq!(
            manifest.check(&wrong_shape_host),
            Err(HostCheckError::UnsupportedCapability {
                capability: Capability::Share,
                shape: CapabilityShape::Request,
                host: "wrong-shape".into(),
            })
        );
    }

    #[test]
    fn host_calls_must_be_declared_and_supported() {
        let manifest = CapabilityManifest::new([CapabilityUse::new(
            Capability::Share,
            CapabilityShape::Request,
        )]);
        let payload = SharePayload::text("log");
        assert_eq!(
            payload,
            SharePayload {
                title: None,
                text: Some("log".into()),
                url: None,
            }
        );
        let call = HostCall::Share {
            id: HostCallId::new("share-1"),
            payload,
        };
        assert_eq!(manifest.validate_call(&web_host(), &call), Ok(()));

        let unsupported_host = HostProfile::new("offline-shell", []);
        let unsupported = manifest
            .validate_call(&unsupported_host, &call)
            .expect_err("declared calls still require host support");
        assert_eq!(
            unsupported,
            HostCheckError::UnsupportedCapability {
                capability: Capability::Share,
                shape: CapabilityShape::Request,
                host: "offline-shell".into(),
            }
        );
        assert_eq!(
            unsupported.to_string(),
            "host `offline-shell` does not support capability `share` with shape Request"
        );

        let haptic = HostCall::Haptic {
            id: HostCallId::new("tap"),
            pattern: HapticPattern::Success,
        };
        assert_eq!(
            manifest.validate_call(&web_host(), &haptic),
            Err(HostCheckError::UndeclaredCapability {
                capability: Capability::Haptics,
                shape: CapabilityShape::Fire,
            })
        );
    }

    enum AppCommand {
        MarkShared,
        MarkHapticAck,
    }

    fn handle_host_event(event: HostEvent) -> Option<AppCommand> {
        match event {
            HostEvent::ShareCompleted {
                completed: true, ..
            } => Some(AppCommand::MarkShared),
            HostEvent::Acknowledged { id } if id.0 == "tap" => Some(AppCommand::MarkHapticAck),
            _ => None,
        }
    }

    fn apply_app_command(command: AppCommand) -> Effect {
        match command {
            AppCommand::MarkShared => Slot::<()>::new(7).text("export shared"),
            AppCommand::MarkHapticAck => Slot::<()>::new(8).text("set complete"),
        }
    }

    #[test]
    fn host_failures_use_one_typed_result_shape() {
        let denied = HostEvent::Failed(
            HostFailure::new(HostFailureKind::PermissionDenied, "share denied")
                .with_id(HostCallId::new("share-1"))
                .with_capability(Capability::Share),
        );
        let timeout = HostEvent::Failed(
            HostFailure::new(HostFailureKind::Timeout, "share timed out")
                .with_id(HostCallId::new("share-1"))
                .with_capability(Capability::Share),
        );
        let unavailable = HostEvent::Failed(
            HostFailure::new(HostFailureKind::Unavailable, "share unsupported")
                .with_capability(Capability::Share),
        );
        let error = HostEvent::Failed(HostFailure::new(HostFailureKind::Error, "host crashed"));

        for event in [denied, timeout, unavailable, error] {
            match event {
                HostEvent::Failed(failure) => assert!(!failure.message.is_empty()),
                other => panic!("unexpected host event shape: {other:?}"),
            }
        }
    }

    #[test]
    fn host_failure_builders_preserve_call_context() {
        let failure = HostFailure::new(HostFailureKind::Timeout, "host timed out")
            .with_id(HostCallId::new("share-1"))
            .with_capability(Capability::Share);

        assert_eq!(failure.id, Some(HostCallId::new("share-1")));
        assert_eq!(failure.capability, Some(Capability::Share));
        assert_eq!(failure.kind, HostFailureKind::Timeout);
        assert_eq!(failure.message, "host timed out");
    }

    #[test]
    fn web_pwa_host_result_routes_through_app_code_before_hemx_effect() {
        let manifest = CapabilityManifest::new([CapabilityUse::new(
            Capability::Share,
            CapabilityShape::Request,
        )]);
        let call = HostCall::Share {
            id: HostCallId::new("share-1"),
            payload: SharePayload::text("log"),
        };
        manifest
            .validate_call(&web_host(), &call)
            .expect("web/PWA host supports declared share request");

        let host_event = HostEvent::ShareCompleted {
            id: HostCallId::new("share-1"),
            completed: true,
        };

        let command = handle_host_event(host_event).expect("host event becomes app command");
        let batch = apply_app_command(command).into_batch(hemx_core::BuildFingerprint(11));

        assert_eq!(batch.ops.len(), 1);
        assert!(matches!(
            &batch.ops[0],
            Effect::Put {
                payload: Payload::Text(text),
                ..
            } if text == "export shared"
        ));
    }

    #[test]
    fn native_shell_boundary_uses_same_manifest_call_event_path() {
        let manifest = CapabilityManifest::new([
            CapabilityUse::new(Capability::Haptics, CapabilityShape::Fire),
            CapabilityUse::new(Capability::Microphone, CapabilityShape::Stream)
                .with_reason("Record dictated workout commands"),
        ]);
        manifest
            .check(&native_shell_host())
            .expect("native shell profile supports declared capabilities");

        let call = HostCall::Haptic {
            id: HostCallId::new("tap"),
            pattern: HapticPattern::Success,
        };
        manifest
            .validate_call(&native_shell_host(), &call)
            .expect("native shell supports declared haptic fire call");

        let command = handle_host_event(HostEvent::Acknowledged {
            id: HostCallId::new("tap"),
        })
        .expect("host ack becomes app command");
        let batch = apply_app_command(command).into_batch(hemx_core::BuildFingerprint(13));

        assert!(matches!(
            &batch.ops[0],
            Effect::Put {
                payload: Payload::Text(text),
                ..
            } if text == "set complete"
        ));
    }

    #[test]
    fn host_events_can_emit_to_existing_app_handlers_without_owning_state() {
        let effect = event("host:share-completed", "share-1");
        let batch = effect.into_batch(hemx_core::BuildFingerprint(12));

        assert!(matches!(
            &batch.ops[0],
            Effect::Emit { name, payload }
                if name == "host:share-completed" && payload == "share-1"
        ));
    }
}
