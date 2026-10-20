use axum::{
    body::Body,
    extract::{DefaultBodyLimit, State},
    http::{Request, StatusCode},
    routing::post,
    Extension, Router,
};
use futures_util::{stream, FutureExt, StreamExt};
use hemx_axum::{InteractionForm, InteractionRequest, MultipartLimits};
use std::{
    convert::Infallible,
    sync::{Arc, Mutex},
};
use tower::ServiceExt;

type Received = Arc<Mutex<Vec<InteractionForm>>>;

async fn accept(State(received): State<Received>, request: InteractionRequest) -> StatusCode {
    received.lock().unwrap().push(request.form().clone());
    StatusCode::NO_CONTENT
}

fn app(limits: Option<MultipartLimits>) -> (Router, Received) {
    let received = Received::default();
    let mut app = Router::new()
        .route("/upload", post(accept))
        .with_state(received.clone());
    if let Some(limits) = limits {
        app = app.layer(Extension(limits));
    }
    (app, received)
}

// Each pair is a Content-Disposition parameter string and raw part data.
// The required __h field costs four text bytes (three for the name, one value).
fn multipart(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut body = b"--X\r\nContent-Disposition: form-data; name=\"__h\"\r\n\r\n7\r\n".to_vec();
    for (parameters, bytes) in parts {
        body.extend_from_slice(b"--X\r\nContent-Disposition: form-data");
        if !parameters.is_empty() {
            body.extend_from_slice(b"; ");
            body.extend_from_slice(parameters.as_bytes());
        }
        body.extend_from_slice(b"\r\n\r\n");
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(b"--X--\r\n");
    body
}

fn request(body: Body) -> Request<Body> {
    Request::post("/upload")
        .header("content-type", "multipart/form-data; boundary=X")
        .body(body)
        .unwrap()
}

#[tokio::test]
async fn text_budget_sums_repeated_names_and_utf8_values() {
    // __h=7 (4), a=ab (3), a=cd (3), é=é (4): fourteen UTF-8 bytes.
    let body = multipart(&[
        ("name=\"a\"", b"ab"),
        ("name=\"a\"", b"cd"),
        ("name=\"é\"", "é".as_bytes()),
    ]);
    for (text_bytes, expected) in [
        (13, StatusCode::PAYLOAD_TOO_LARGE),
        (14, StatusCode::NO_CONTENT),
    ] {
        let (app, received) = app(Some(MultipartLimits {
            text_bytes,
            file_bytes: 0,
        }));
        let response = app
            .oneshot(request(Body::from(body.clone())))
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        let forms = received.lock().unwrap();
        assert_eq!(forms.len(), usize::from(expected.is_success()));
        if let Some(form) = forms.first() {
            assert_eq!(form.values("a").collect::<Vec<_>>(), ["ab", "cd"]);
            assert_eq!(form.value("é"), Some("é"));
        }
    }
}

#[tokio::test]
async fn file_budget_is_cumulative_and_file_names_cost_text_bytes() {
    let body = multipart(&[
        ("name=\"a\"; filename=\"one.bin\"", b"abc"),
        ("name=\"b\"; filename=\"\"", b"\xff\x00"),
    ]);
    for (text_bytes, file_bytes, expected) in [
        (6, 4, StatusCode::PAYLOAD_TOO_LARGE),
        (5, 5, StatusCode::PAYLOAD_TOO_LARGE),
        (6, 5, StatusCode::NO_CONTENT),
    ] {
        let (app, received) = app(Some(MultipartLimits {
            text_bytes,
            file_bytes,
        }));
        let response = app
            .oneshot(request(Body::from(body.clone())))
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        let forms = received.lock().unwrap();
        assert_eq!(forms.len(), usize::from(expected.is_success()));
        if let Some(form) = forms.first() {
            assert_eq!(form.files().len(), 2);
            assert_eq!(form.file("a").unwrap().bytes, b"abc");
            assert_eq!(form.file("b").unwrap().bytes, b"\xff\x00");
        }
    }
}

#[tokio::test]
async fn unnamed_parts_are_counted_but_not_stored() {
    for (parameters, text_bytes, file_bytes, expected) in [
        ("", 6, 0, StatusCode::PAYLOAD_TOO_LARGE),
        ("", 7, 0, StatusCode::NO_CONTENT),
        ("filename=\"ignored\"", 4, 2, StatusCode::PAYLOAD_TOO_LARGE),
        ("filename=\"ignored\"", 4, 3, StatusCode::NO_CONTENT),
    ] {
        let (app, received) = app(Some(MultipartLimits {
            text_bytes,
            file_bytes,
        }));
        let response = app
            .oneshot(request(Body::from(multipart(&[(parameters, b"abc")]))))
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        let forms = received.lock().unwrap();
        assert_eq!(forms.len(), usize::from(expected.is_success()));
        if let Some(form) = forms.first() {
            assert_eq!(form.fields(), &[("__h".into(), "7".into())]);
            assert!(form.files().is_empty());
        }
    }
}

#[tokio::test]
async fn oversized_text_and_files_reject_without_waiting_for_eof() {
    for parameters in ["name=\"a\"", "name=\"a\"; filename=\"clip.glb\""] {
        let (app, received) = app(Some(MultipartLimits {
            text_bytes: 8,
            file_bytes: 3,
        }));
        let mut prefix = multipart(&[(parameters, &[b'x'; 128])]);
        prefix.truncate(prefix.len() - b"\r\n--X--\r\n".len());
        let body =
            Body::from_stream(stream::iter([Ok::<_, Infallible>(prefix)]).chain(stream::pending()));
        let response = app
            .oneshot(request(body))
            .now_or_never()
            .expect("limit rejection waited for another chunk or EOF")
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert!(received.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn limits_are_opt_in_and_do_not_apply_to_urlencoded_forms() {
    let (unconfigured, received) = app(None);
    let response = unconfigured
        .oneshot(request(Body::from(multipart(&[(
            "name=\"a\"",
            &[b'x'; 1024],
        )]))))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(received.lock().unwrap()[0].value("a").unwrap().len(), 1024);

    let (configured, received) = app(Some(MultipartLimits {
        text_bytes: 0,
        file_bytes: 0,
    }));
    let response = configured
        .oneshot(
            Request::post("/upload")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("__h=7&a=accepted"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(received.lock().unwrap()[0].value("a"), Some("accepted"));
}

#[tokio::test]
async fn whole_body_limit_still_applies_with_unbounded_category_budgets() {
    for streamed in [false, true] {
        let (app, received) = app(Some(MultipartLimits {
            text_bytes: usize::MAX,
            file_bytes: usize::MAX,
        }));
        let body = multipart(&[("name=\"a\"; filename=\"clip.glb\"", &[b'x'; 128])]);
        let body = if streamed {
            Body::from_stream(stream::iter(
                body.chunks(16)
                    .map(|chunk| Ok::<_, Infallible>(chunk.to_vec()))
                    .collect::<Vec<_>>(),
            ))
        } else {
            Body::from(body)
        };
        let response = app
            .layer(DefaultBodyLimit::max(64))
            .oneshot(request(body))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert!(received.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn zero_is_a_real_limit_and_headers_are_not_payload() {
    for (text_bytes, expected) in [
        (0, StatusCode::PAYLOAD_TOO_LARGE),
        (4, StatusCode::NO_CONTENT),
    ] {
        let (app, _) = app(Some(MultipartLimits {
            text_bytes,
            file_bytes: 0,
        }));
        let response = app
            .oneshot(request(Body::from(multipart(&[]))))
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    let (app, received) = app(Some(MultipartLimits {
        text_bytes: 5,
        file_bytes: 0,
    }));
    let parameters = format!(
        "name=\"a\"; filename=\"{}\"\r\nContent-Type: application/octet-stream",
        "x".repeat(256)
    );
    let response = app
        .oneshot(request(Body::from(multipart(&[(&parameters, b"")]))))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(received.lock().unwrap()[0]
        .file("a")
        .unwrap()
        .bytes
        .is_empty());
}

#[tokio::test]
async fn text_stays_utf8_and_file_bytes_stay_opaque() {
    let (app, received) = app(Some(MultipartLimits {
        text_bytes: 6,
        file_bytes: 0,
    }));
    let response = app
        .oneshot(request(Body::from(multipart(&[("name=\"a\"", b"\xff")]))))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(received.lock().unwrap().is_empty());
}
