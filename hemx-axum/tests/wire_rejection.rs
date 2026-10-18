use axum::{
    body::to_bytes,
    http::{header, StatusCode},
    response::IntoResponse,
};
use futures_util::{stream, StreamExt};
use hemx_axum::{
    encode_sse_batch, sse, sse_event, EffectResponse, HEMX_CONTENT_TYPE, HEMX_FINGERPRINT_HEADER,
};
use hemx_core::{
    BuildFingerprint, Effect, EffectBatch, IntoEffect, ResourceId, ResourceKind, ResourceRef,
    WireError, MAX_EFFECTS, MAX_FIELD_BYTES,
};
use std::{convert::Infallible, error::Error};

fn empty_batch() -> EffectBatch {
    ().into_batch(BuildFingerprint(11))
}

fn invalid_batches() -> Vec<(EffectBatch, WireError)> {
    let mut incompatible = empty_batch();
    incompatible.abi_version = 1;
    vec![
        (incompatible, WireError::UnsupportedVersion),
        (
            vec![Effect::Dispatch {
                event: "private-payload-must-not-leak".into(),
                payload: vec![0; MAX_FIELD_BYTES + 1],
            }]
            .into_batch(BuildFingerprint(11)),
            WireError::FieldTooLarge,
        ),
        (
            vec![
                Effect::Focus {
                    target: ResourceRef::unscoped(ResourceId::new(ResourceKind::Slot, 1)),
                    prevent_scroll: false,
                };
                MAX_EFFECTS + 1
            ]
            .into_batch(BuildFingerprint(11)),
            WireError::TooManyEffects,
        ),
        (
            vec![
                Effect::Dispatch {
                    event: "private-payload-must-not-leak".into(),
                    payload: vec![0; MAX_FIELD_BYTES],
                };
                4
            ]
            .into_batch(BuildFingerprint(11)),
            WireError::BatchTooLarge,
        ),
    ]
}

#[tokio::test]
async fn invalid_effect_responses_return_actionable_500_without_wire_headers_or_payload() {
    for (batch, expected) in invalid_batches() {
        let response = EffectResponse { batch }.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_ne!(response.headers()[header::CONTENT_TYPE], HEMX_CONTENT_TYPE);
        assert!(!response.headers().contains_key(HEMX_FINGERPRINT_HEADER));
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        assert_eq!(
            body.as_ref(),
            format!("failed to encode hemx effect response: {expected}").as_bytes()
        );
        assert!(!String::from_utf8_lossy(&body).contains("private-payload"));
    }
}

#[test]
fn sse_encoders_reject_invalid_batches_without_an_event() {
    for (batch, expected) in invalid_batches() {
        assert_eq!(encode_sse_batch(&batch), Err(expected));
        assert_eq!(sse_event(batch).unwrap_err(), expected);
    }
}

fn wire_error_in_chain(error: &(dyn Error + 'static)) -> Option<WireError> {
    if let Some(error) = error.downcast_ref::<WireError>() {
        return Some(*error);
    }
    error.source().and_then(wire_error_in_chain)
}

#[tokio::test]
async fn sse_body_fails_on_encoding_error_after_preserving_valid_events() {
    for (batch, expected) in invalid_batches() {
        let response = sse(stream::iter([
            Ok::<_, Infallible>(empty_batch()),
            Ok(batch),
        ]))
        .into_response();
        let mut body = response.into_body().into_data_stream();
        assert_eq!(
            body.next().await.unwrap().unwrap().as_ref(),
            b"event: hemx\ndata: SEVNWAMAAAALAAAAAAAAAAAAAAA\n\n"
        );
        let error = body.next().await.unwrap().unwrap_err();
        assert_eq!(wire_error_in_chain(&error), Some(expected));
    }
}

#[tokio::test]
async fn sse_body_preserves_upstream_error_instead_of_emitting_error_event() {
    let response = sse(stream::iter([Err::<EffectBatch, _>(
        std::io::Error::other("upstream failed"),
    )]))
    .into_response();
    let mut body = response.into_body().into_data_stream();
    let error = body.next().await.unwrap().unwrap_err();
    let mut cause: &(dyn Error + 'static) = &error;
    while let Some(source) = cause.source() {
        cause = source;
    }
    assert_eq!(cause.to_string(), "upstream failed");
    assert!(cause.downcast_ref::<std::io::Error>().is_some());
}
