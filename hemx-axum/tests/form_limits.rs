use axum::{
    body::{to_bytes, Body},
    extract::{DefaultBodyLimit, State},
    http::{Request, StatusCode},
    routing::post,
    Router,
};
use hemx_axum::InteractionRequest;
use std::{
    convert::Infallible,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tower::ServiceExt;

async fn accept(State(calls): State<Arc<AtomicUsize>>, request: InteractionRequest) -> StatusCode {
    assert_eq!(request.form().handle_id, 7);
    assert_eq!(request.form().file("upload").unwrap().bytes, b"ok");
    calls.fetch_add(1, Ordering::SeqCst);
    StatusCode::NO_CONTENT
}

fn multipart(value: &str) -> String {
    format!(
        "--X\r\nContent-Disposition: form-data; name=\"__h\"\r\n\r\n7\r\n\
         --X\r\nContent-Disposition: form-data; name=\"upload\"; filename=\"probe.txt\"\r\n\
         Content-Type: text/plain\r\n\r\n{value}\r\n--X--\r\n"
    )
}

async fn request(body: String, streamed: bool) -> (StatusCode, usize, String) {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/upload", post(accept))
        .layer(DefaultBodyLimit::max(256))
        .with_state(calls.clone());
    let body = if streamed {
        let chunks: Vec<_> = body
            .as_bytes()
            .chunks(64)
            .map(|chunk| Ok::<_, Infallible>(chunk.to_vec()))
            .collect();
        Body::from_stream(futures_util::stream::iter(chunks))
    } else {
        Body::from(body)
    };
    let response = app
        .oneshot(
            Request::post("/upload")
                .header("content-type", "multipart/form-data; boundary=X")
                .body(body)
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let message = to_bytes(response.into_body(), 4096).await.unwrap();
    (
        status,
        calls.load(Ordering::SeqCst),
        String::from_utf8(message.to_vec()).unwrap(),
    )
}

#[tokio::test]
async fn multipart_preserves_host_limits_for_buffered_and_streamed_bodies() {
    for streamed in [false, true] {
        let (status, calls, message) = request(multipart(&"x".repeat(1024)), streamed).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{message}");
        assert_eq!(calls, 0, "oversized input reached the mutation handler");
        assert_eq!(message, "hemx interaction body exceeds the host limit");

        let (status, calls, _) = request(multipart("ok"), streamed).await;
        assert_eq!((status, calls), (StatusCode::NO_CONTENT, 1));

        let (status, calls, message) = request("not a multipart body".into(), streamed).await;
        assert_eq!((status, calls), (StatusCode::BAD_REQUEST, 0));
        assert_eq!(message, "invalid hemx form body");
    }
}
