//! Opt-in browser boundary for client-local hemx handlers.
//!
//! Application code reaches this crate through the `hemx` `client` feature and
//! `#[hemx::handler(client)]`; server-first applications do not depend on it.

use hemx_core::{BuildFingerprint, IntoEffect};

#[doc(hidden)]
pub use wasm_bindgen::prelude::wasm_bindgen;
#[doc(hidden)]
pub use wasm_bindgen::*;

pub const CLIENT_EVENT_ABI_VERSION: u32 = 1;
pub const CLIENT_STATE_ABI_VERSION: u32 = 1;
const MAX_CLIENT_EVENT_KIND_BYTES: usize = 256;
const MAX_CLIENT_EVENT_VALUE_BYTES: usize = 64 * 1024;
const MAX_CLIENT_EVENT_KEY_BYTES: usize = 1024;
const MAX_CLIENT_STATE_BYTES: usize = 1024 * 1024;

/// Versioned browser event accepted by client-local handlers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClientEvent {
    pub kind: String,
    pub value: Option<String>,
    pub checked: Option<bool>,
    pub key: Option<String>,
}

/// Explicit root-owned state passed to a client-local handler.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClientState {
    pub encoded: String,
}

/// Validates primitive wasm-bindgen values before application code runs.
///
/// Primitive arguments keep JavaScript from owning a second event/state codec.
/// The ordinary effect result uses `hemx-core`'s canonical `EffectBatch` codec.
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn decode_client_inputs(
    event_version: u32,
    kind: String,
    value: Option<String>,
    checked: Option<bool>,
    key: Option<String>,
    state_version: u32,
    encoded_state: String,
) -> Result<(ClientEvent, ClientState), String> {
    if event_version != CLIENT_EVENT_ABI_VERSION {
        return Err(format!(
            "unsupported client-local event ABI version {event_version}; expected {CLIENT_EVENT_ABI_VERSION}"
        ));
    }
    if kind.is_empty() || kind.len() > MAX_CLIENT_EVENT_KIND_BYTES {
        return Err(format!(
            "invalid client-local event payload: event kind must contain 1..={MAX_CLIENT_EVENT_KIND_BYTES} bytes"
        ));
    }
    if value
        .as_ref()
        .is_some_and(|value| value.len() > MAX_CLIENT_EVENT_VALUE_BYTES)
    {
        return Err(format!(
            "invalid client-local event payload: value exceeds {MAX_CLIENT_EVENT_VALUE_BYTES} bytes"
        ));
    }
    if key
        .as_ref()
        .is_some_and(|key| key.len() > MAX_CLIENT_EVENT_KEY_BYTES)
    {
        return Err(format!(
            "invalid client-local event payload: key exceeds {MAX_CLIENT_EVENT_KEY_BYTES} bytes"
        ));
    }
    if state_version != CLIENT_STATE_ABI_VERSION {
        return Err(format!(
            "unsupported client-local state ABI version {state_version}; expected {CLIENT_STATE_ABI_VERSION}"
        ));
    }
    if encoded_state.len() > MAX_CLIENT_STATE_BYTES {
        return Err(format!(
            "invalid client-local state payload: state exceeds {MAX_CLIENT_STATE_BYTES} bytes"
        ));
    }
    Ok((
        ClientEvent {
            kind,
            value,
            checked,
            key,
        },
        ClientState {
            encoded: encoded_state,
        },
    ))
}

/// Encodes a client handler result with the ordinary hemx effect wire format.
///
/// Keeping this conversion here gives generated WASM exports one ABI boundary
/// instead of teaching the proc macro a second effect protocol.
#[doc(hidden)]
pub fn encode_handler_effect(effect: impl IntoEffect, fingerprint: BuildFingerprint) -> Vec<u8> {
    effect.into_batch(fingerprint).to_wire()
}

#[cfg(test)]
mod tests {
    use super::{decode_client_inputs, encode_handler_effect, ClientEvent, ClientState};
    use hemx_core::{BuildFingerprint, EffectBatch, Slot};

    #[test]
    fn client_inputs_are_typed_and_versioned() {
        assert_eq!(
            decode_client_inputs(
                1,
                "click".to_owned(),
                None,
                None,
                None,
                1,
                "count=3".to_owned(),
            ),
            Ok((
                ClientEvent {
                    kind: "click".to_owned(),
                    value: None,
                    checked: None,
                    key: None,
                },
                ClientState {
                    encoded: "count=3".to_owned(),
                },
            ))
        ); //
        assert_eq!(
            decode_client_inputs(
                1,
                "click".to_owned(),
                None,
                None,
                None,
                2,
                "count=3".to_owned(),
            )
            .expect_err("reject unknown state ABI"),
            "unsupported client-local state ABI version 2; expected 1"
        ); //
    }

    #[test]
    fn client_input_boundary_accepts_limits_and_preserves_values() {
        let kind = "k".repeat(256);
        let value = "v".repeat(64 * 1024);
        let key = "x".repeat(1024);
        let state = "s".repeat(1024 * 1024);

        let (event, decoded_state) = decode_client_inputs(
            1,
            kind.clone(),
            Some(value.clone()),
            Some(true),
            Some(key.clone()),
            1,
            state.clone(),
        )
        .expect("documented client-local limits are inclusive");

        assert_eq!(
            event,
            ClientEvent {
                kind,
                value: Some(value),
                checked: Some(true),
                key: Some(key),
            }
        );
        assert_eq!(decoded_state, ClientState { encoded: state });
    }

    #[test]
    fn client_input_boundary_rejects_invalid_versions_and_payload_sizes() {
        let decode = |event_version, kind, value, key, state_version, state| {
            decode_client_inputs(event_version, kind, value, None, key, state_version, state)
        };

        for (result, expected) in [
            (
                decode(0, "click".into(), None, None, 1, String::new()),
                "unsupported client-local event ABI version 0; expected 1",
            ),
            (
                decode(1, String::new(), None, None, 1, String::new()),
                "invalid client-local event payload: event kind must contain 1..=256 bytes",
            ),
            (
                decode(1, "k".repeat(257), None, None, 1, String::new()),
                "invalid client-local event payload: event kind must contain 1..=256 bytes",
            ),
            (
                decode(
                    1,
                    "input".into(),
                    Some("v".repeat(64 * 1024 + 1)),
                    None,
                    1,
                    String::new(),
                ),
                "invalid client-local event payload: value exceeds 65536 bytes",
            ),
            (
                decode(
                    1,
                    "keydown".into(),
                    None,
                    Some("k".repeat(1025)),
                    1,
                    String::new(),
                ),
                "invalid client-local event payload: key exceeds 1024 bytes",
            ),
            (
                decode(1, "click".into(), None, None, 0, String::new()),
                "unsupported client-local state ABI version 0; expected 1",
            ),
            (
                decode(
                    1,
                    "click".into(),
                    None,
                    None,
                    1,
                    "s".repeat(1024 * 1024 + 1),
                ),
                "invalid client-local state payload: state exceeds 1048576 bytes",
            ),
        ] {
            assert_eq!(
                result.expect_err("invalid client input must fail closed"),
                expected
            );
        }
    }

    #[test]
    fn client_handler_uses_the_ordinary_effect_wire_format() {
        let fingerprint = BuildFingerprint(17);
        let effect = Slot::<()>::new(4).text("local");
        let wire = encode_handler_effect(effect.clone(), fingerprint);

        assert_eq!(
            EffectBatch::from_wire(&wire).expect("decode client effect"),
            EffectBatch {
                abi_version: hemx_core::EFFECT_BATCH_ABI_VERSION,
                fingerprint,
                ops: vec![effect],
            }
        ); //
    }
}
