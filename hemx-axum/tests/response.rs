use axum::extract::{DefaultBodyLimit, State};
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::response::IntoResponse;
use axum::{
    body::{to_bytes, Body, Bytes},
    routing::post,
    Router,
};
use hemx_axum::{
    interactions, runtime_js, runtime_js_hash, runtime_js_path, runtime_js_route_path,
    runtime_js_script_src, runtime_js_source, DispatchRejection, EffectResponse, Form,
    HandlerErrorContext, HandlerFailure, InteractionForm, InteractionFormRejection,
    InteractionRequest, IntoHandlerFailure, PageMode, PageRequest, PageResponse, HEMX_CONTENT_TYPE,
    HEMX_FINGERPRINT_HEADER, HEMX_PARTIAL_HEADER, HEMX_RUNTIME_CONTENT_TYPE, HEMX_TITLE_HEADER,
};
use hemx_core::{push, BuildFingerprint, Effect, Handle, IntoEffect, SafeHtml, Slot};
use scraper::{Html, Selector};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Mutex;
use tower::ServiceExt;

static MUTATION_CALLS: AtomicUsize = AtomicUsize::new(0);
static BOUNDARY_TEST_LOCK: Mutex<()> = Mutex::const_new(());

async fn bounded_mutation(_: InteractionRequest) -> StatusCode {
    MUTATION_CALLS.fetch_add(1, Ordering::SeqCst);
    StatusCode::NO_CONTENT
}

async fn multipart_mutation(request: InteractionRequest) -> StatusCode {
    let form = request.form();
    assert_eq!(form.handle_id, 7);
    assert_eq!(form.value("title"), Some("report"));
    assert_eq!(form.files().len(), 1);
    let upload = form.file("upload").expect("uploaded file");
    assert_eq!(upload.file_name.as_deref(), Some("report.txt"));
    assert_eq!(upload.content_type.as_deref(), Some("text/plain"));
    assert_eq!(upload.bytes, b"hello");
    StatusCode::NO_CONTENT
}

#[tokio::test]
async fn interaction_boundary_honors_media_type_and_host_body_limit() {
    let _guard = BOUNDARY_TEST_LOCK.lock().await;
    MUTATION_CALLS.store(0, Ordering::SeqCst);
    let app = Router::new()
        .route("/mutate", post(bounded_mutation))
        .layer(DefaultBodyLimit::max(32));

    let missing_content_type = app
        .clone()
        .oneshot(Request::post("/mutate").body(Body::from("__h=1")).unwrap())
        .await
        .unwrap();
    assert_eq!(
        missing_content_type.status(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    assert_eq!(MUTATION_CALLS.load(Ordering::SeqCst), 0);

    let unsupported = app
        .clone()
        .oneshot(
            Request::post("/mutate")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"__h":"1"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unsupported.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(MUTATION_CALLS.load(Ordering::SeqCst), 0);

    let oversized = app
        .clone()
        .oneshot(
            Request::post("/mutate")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!("__h=1&value={}", "x".repeat(64))))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(MUTATION_CALLS.load(Ordering::SeqCst), 0);

    let accepted = app
        .oneshot(
            Request::post("/mutate")
                .header(
                    header::CONTENT_TYPE,
                    "Application/X-Www-Form-Urlencoded; charset=UTF-8",
                )
                .body(Body::from("__h=1"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::NO_CONTENT);
    assert_eq!(MUTATION_CALLS.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn interaction_boundary_extracts_multipart_fields_and_files() {
    let _guard = BOUNDARY_TEST_LOCK.lock().await;
    let boundary = "hemx-boundary";
    let body = concat!(
        "--hemx-boundary\r\n",
        "Content-Disposition: form-data; name=\"__h\"\r\n\r\n",
        "7\r\n",
        "--hemx-boundary\r\n",
        "Content-Disposition: form-data; name=\"title\"\r\n\r\n",
        "report\r\n",
        "--hemx-boundary\r\n",
        "Content-Disposition: form-data; name=\"upload\"; filename=\"report.txt\"\r\n",
        "Content-Type: text/plain\r\n\r\n",
        "hello\r\n",
        "--hemx-boundary--\r\n"
    );
    let response = Router::new()
        .route("/upload", post(multipart_mutation))
        .oneshot(
            Request::post("/upload")
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn interaction_boundary_skips_unnamed_parts_and_rejects_invalid_multipart() {
    let _guard = BOUNDARY_TEST_LOCK.lock().await;
    let app = Router::new().route("/mutate", post(bounded_mutation));
    let missing_boundary = app
        .clone()
        .oneshot(
            Request::post("/mutate")
                .header(header::CONTENT_TYPE, "multipart/form-data")
                .body(Body::from("invalid"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing_boundary.status(), StatusCode::BAD_REQUEST);

    let accepted = concat!(
        "--b\r\n",
        "Content-Disposition: form-data; filename=\"ignored.txt\"\r\n\r\n",
        "ignored\r\n",
        "--b\r\n",
        "Content-Disposition: form-data; name=\"__h\"\r\n\r\n",
        "1\r\n",
        "--b--\r\n"
    );
    let response = app
        .clone()
        .oneshot(
            Request::post("/mutate")
                .header(header::CONTENT_TYPE, "multipart/form-data; boundary=b")
                .body(Body::from(accepted))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let invalid_utf8 =
        b"--b\r\nContent-Disposition: form-data; name=\"__h\"\r\n\r\n\xff\r\n--b--\r\n";
    let response = app
        .clone()
        .oneshot(
            Request::post("/mutate")
                .header(header::CONTENT_TYPE, "multipart/form-data; boundary=b")
                .body(Body::from(invalid_utf8.as_slice()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let stream = futures_util::stream::iter([
        Ok::<_, std::io::Error>(Bytes::from_static(
            b"--b\r\nContent-Disposition: form-data; name=\"__h\"\r\n\r\n",
        )),
        Err(std::io::Error::other("stream failed")),
    ]);
    let response = app
        .oneshot(
            Request::post("/mutate")
                .header(header::CONTENT_TYPE, "multipart/form-data; boundary=b")
                .body(Body::from_stream(stream))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

fn selector(value: &str) -> Selector {
    Selector::parse(value).expect("test selector parses")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProjectId(u32);

impl std::str::FromStr for ProjectId {
    type Err = std::num::ParseIntError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        value.parse().map(ProjectId)
    }
}

struct OpenProject {
    project_id: ProjectId,
}

impl hemx_core::FromForm for OpenProject {
    fn from_form_fields(fields: &[(String, String)]) -> Result<Self, hemx_core::FormError> {
        let Some(value) = fields
            .iter()
            .find_map(|(name, value)| (name == "project_id").then_some(value.as_str()))
        else {
            return Err(hemx_core::FormError::new("missing form field `project_id`"));
        };
        Ok(Self {
            project_id: value
                .parse()
                .map_err(|_| hemx_core::FormError::new("invalid form field `project_id`"))?,
        })
    }
}

#[test]
fn page_mode_detects_only_explicit_partial_header_values() {
    for value in [
        None,
        Some("false"),
        Some("0"),
        Some("TRUE"),
        Some("invalid"),
    ] {
        let mut headers = HeaderMap::new();
        if let Some(value) = value {
            headers.insert(HEMX_PARTIAL_HEADER, value.parse().unwrap());
        }
        assert_eq!(PageMode::from_headers(&headers), PageMode::Full);
    }
    for value in ["true", "1"] {
        let mut headers = HeaderMap::new();
        headers.insert(HEMX_PARTIAL_HEADER, value.parse().unwrap());
        assert_eq!(PageMode::from_headers(&headers), PageMode::Partial);
    }
}

#[test]
fn page_request_wraps_full_pages_and_leaves_partials_unwrapped() {
    assert!(!PageRequest {
        mode: PageMode::Full
    }
    .is_partial());
    assert!(PageRequest {
        mode: PageMode::Partial
    }
    .is_partial());
    let full = PageRequest {
        mode: PageMode::Full,
    }
    .page("<main data-page=\"docs\">Docs</main>", |content| {
        format!("<html><body data-shell=\"docs\">{content}</body></html>")
    });
    assert_eq!(full.mode, PageMode::Full);
    let full_document = Html::parse_document(&full.html);
    assert_eq!(
        full_document
            .select(&selector(
                "body[data-shell=\"docs\"] main[data-page=\"docs\"]"
            ))
            .count(),
        1
    );

    let partial = PageRequest {
        mode: PageMode::Partial,
    }
    .page("<main data-page=\"docs\">Docs</main>", |content| {
        format!("<html><body data-shell=\"docs\">{content}</body></html>")
    });
    assert_eq!(partial.mode, PageMode::Partial);
    let partial_fragment = Html::parse_fragment(&partial.html);
    assert_eq!(
        partial_fragment
            .select(&selector("main[data-page=\"docs\"]"))
            .count(),
        1
    );
    assert_eq!(
        partial_fragment
            .select(&selector("body[data-shell=\"docs\"]"))
            .count(),
        0
    );
}

#[test]
fn page_request_wraps_safe_html_full_pages_and_leaves_partials_unwrapped() {
    let full = PageRequest {
        mode: PageMode::Full,
    }
    .page_html(
        SafeHtml::trusted("<main data-page=\"docs\">Docs</main>"),
        |content| {
            SafeHtml::trusted(format!(
                "<html><body data-shell=\"docs\">{content}</body></html>"
            ))
        },
    );
    assert_eq!(full.mode, PageMode::Full);
    let full_document = Html::parse_document(&full.html);
    assert_eq!(
        full_document
            .select(&selector(
                "body[data-shell=\"docs\"] main[data-page=\"docs\"]"
            ))
            .count(),
        1
    );

    let partial = PageRequest {
        mode: PageMode::Partial,
    }
    .page_html(
        SafeHtml::trusted("<main data-page=\"docs\">Docs</main>"),
        |content| {
            SafeHtml::trusted(format!(
                "<html><body data-shell=\"docs\">{content}</body></html>"
            ))
        },
    );
    assert_eq!(partial.mode, PageMode::Partial);
    let partial_fragment = Html::parse_fragment(&partial.html);
    assert_eq!(
        partial_fragment
            .select(&selector("main[data-page=\"docs\"]"))
            .count(),
        1
    );
    assert_eq!(
        partial_fragment
            .select(&selector("body[data-shell=\"docs\"]"))
            .count(),
        0
    );
}

#[test]
fn page_response_constructors_preserve_mode_and_optional_fingerprint() {
    let full = PageResponse::full("<html>Full</html>");
    assert_eq!(full.mode, PageMode::Full);
    assert_eq!(full.html, "<html>Full</html>");
    assert_eq!(full.fingerprint, None);

    let partial = PageResponse::partial("<main>Partial</main>");
    assert_eq!(partial.mode, PageMode::Partial);
    assert_eq!(partial.html, "<main>Partial</main>");
    assert_eq!(partial.title, None);
    assert_eq!(partial.fingerprint, None);

    let fingerprint = BuildFingerprint(42);
    assert_eq!(
        partial.fingerprint(fingerprint).fingerprint,
        Some(fingerprint)
    );
}

#[tokio::test]
async fn full_page_response_preserves_html_and_fingerprint_without_partial_headers() {
    let response = PageResponse::full("<html>Full</html>")
        .fingerprint(BuildFingerprint(42))
        .into_response();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/html; charset=utf-8"
    );
    assert_eq!(response.headers()[HEMX_FINGERPRINT_HEADER], "42");
    assert!(!response.headers().contains_key(HEMX_PARTIAL_HEADER));
    assert!(!response.headers().contains_key(HEMX_TITLE_HEADER));
    assert_eq!(
        to_bytes(response.into_body(), 1024).await.unwrap().as_ref(),
        b"<html>Full</html>"
    );
}

#[test]
fn partial_page_response_sets_partial_and_title_headers() {
    let response = PageResponse::partial("<main>Docs</main>")
        .title("Docs")
        .into_response();

    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/html; charset=utf-8"
    );
    assert_eq!(response.headers()[HEMX_PARTIAL_HEADER], "true");
    assert_eq!(response.headers()[HEMX_TITLE_HEADER], "Docs");
}

#[test]
fn effect_response_is_wire_batch_with_fingerprint_header() {
    let response = EffectResponse::new(push("/docs"), BuildFingerprint(99)).into_response();

    assert_eq!(response.headers()[header::CONTENT_TYPE], HEMX_CONTENT_TYPE);
    assert_eq!(response.headers()[HEMX_FINGERPRINT_HEADER], "99");
}

#[test]
fn interaction_form_parses_handle_and_fields() {
    let form =
        InteractionForm::parse_urlencoded(b"__h=42&title=Hello+World&tag=a&tag=b%2Fc").unwrap();

    assert_eq!(form.handle_id, 42);
    assert_eq!(form.value("title"), Some("Hello World"));
    assert_eq!(form.values("tag").collect::<Vec<_>>(), ["a", "b/c"]);
}

#[test]
fn interaction_form_parses_typed_values() {
    let form =
        InteractionForm::parse_urlencoded(b"__h=42&count=7&bad=nope").expect("form should parse");

    assert_eq!(form.parse::<u32>("count"), Some(7));
    assert_eq!(form.parse::<u32>("bad"), None);
    assert_eq!(form.parse::<u32>("missing"), None);
}

#[test]
fn interaction_form_requires_numeric_handle() {
    assert_eq!(
        InteractionForm::parse_urlencoded(b"title=Hello").unwrap_err(),
        InteractionFormRejection::MissingHandle
    );
    assert_eq!(
        InteractionForm::parse_urlencoded(b"__h=nope").unwrap_err(),
        InteractionFormRejection::InvalidHandle
    );
}

#[test]
fn interaction_form_preserves_hidden_csrf_fields_for_extractors() {
    let form = InteractionForm::parse_urlencoded(b"__h=42&csrf_token=abc123&title=Hello").unwrap();

    assert_eq!(form.handle_id, 42);
    assert_eq!(form.value("csrf_token"), Some("abc123"));
    assert!(form.fields().iter().any(|(name, _)| name == "csrf_token"));
}

#[test]
fn interaction_request_dispatches_with_concise_handlers_helper() {
    let request = InteractionRequest::from(InteractionForm::for_handle(
        Handle::<()>::new(7),
        Vec::new(),
    ));
    let response = request
        .dispatch(
            interactions(BuildFingerprint(4))
                .on(Handle::<()>::new(7), |_| Slot::<String>::new(3).text("ok")),
        )
        .unwrap();

    assert_eq!(response.batch.fingerprint, BuildFingerprint(4));
    assert_eq!(response.batch.ops, vec![Slot::<String>::new(3).text("ok")]);
}

#[test]
fn state_interactions_starts_stateful_wiring_without_nested_closures() {
    fn ping(prefix: String) -> impl IntoEffect {
        Slot::<String>::new(3).text(format!("{prefix}: ping"))
    }

    fn open(prefix: String, input: OpenProject) -> impl IntoEffect {
        Slot::<String>::new(3).text(format!("{prefix}: {}", input.project_id.0))
    }

    let registry = || {
        hemx_axum::state_interactions(BuildFingerprint(4), "project".to_owned())
            .on_state(Handle::<()>::new(7), ping)
            .on(Handle::<()>::new(8), open)
            .into_registry()
    };

    let ping_response = InteractionRequest::from(InteractionForm::for_handle(
        Handle::<()>::new(7),
        Vec::new(),
    ))
    .dispatch(registry())
    .unwrap();
    assert_eq!(
        ping_response.batch.ops,
        vec![Slot::<String>::new(3).text("project: ping")]
    );

    let open_response = InteractionRequest::from(InteractionForm::for_handle(
        Handle::<()>::new(8),
        vec![("project_id".to_owned(), "42".to_owned())],
    ))
    .dispatch(registry())
    .unwrap();
    assert_eq!(
        open_response.batch.ops,
        vec![Slot::<String>::new(3).text("project: 42")]
    );
}

#[test]
fn interaction_form_accessors_preserve_repeated_values_and_decode_diagnostics() {
    let form =
        InteractionForm::parse_urlencoded(b"__h=42&count=7&tag=alpha&tag=beta&empty=").unwrap();

    assert_eq!(form.handle_id, 42);
    assert_eq!(form.value("tag"), Some("alpha"));
    assert_eq!(form.values("tag").collect::<Vec<_>>(), ["alpha", "beta"]);
    assert_eq!(form.parse::<u32>("count"), Some(7));
    assert_eq!(form.parse::<u32>("tag"), None);
    assert_eq!(form.parse::<u32>("missing"), None);
    assert_eq!(form.required("empty"), Ok(""));
    assert_eq!(
        form.required("missing").unwrap_err().message(),
        "missing form field `missing`"
    );
    assert_eq!(form.parse_required::<u32>("count"), Ok(7));
    assert_eq!(
        form.parse_required::<u32>("missing").unwrap_err().message(),
        "missing form field `missing`"
    );
    assert_eq!(
        form.parse_required::<u32>("tag").unwrap_err().message(),
        "invalid form field `tag`"
    );
    assert!(form.files().is_empty());
    assert!(form.file("upload").is_none());
    assert!(form
        .fields()
        .iter()
        .any(|pair| pair == &("count".into(), "7".into())));
}

#[tokio::test]
async fn interaction_form_rejections_return_stable_status_and_diagnostic() {
    for (rejection, status, message) in [
        (
            InteractionFormRejection::UnsupportedMediaType,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "hemx interactions require application/x-www-form-urlencoded or multipart/form-data",
        ),
        (
            InteractionFormRejection::BodyTooLarge,
            StatusCode::PAYLOAD_TOO_LARGE,
            "hemx interaction body exceeds the host limit",
        ),
        (
            InteractionFormRejection::InvalidBody,
            StatusCode::BAD_REQUEST,
            "invalid hemx form body",
        ),
        (
            InteractionFormRejection::MissingHandle,
            StatusCode::BAD_REQUEST,
            "missing __h hemx handle field",
        ),
        (
            InteractionFormRejection::InvalidHandle,
            StatusCode::BAD_REQUEST,
            "invalid __h hemx handle field",
        ),
    ] {
        let response = rejection.into_response();
        assert_eq!(response.status(), status);
        assert_eq!(
            to_bytes(response.into_body(), 1024).await.unwrap().as_ref(),
            message.as_bytes()
        );
    }
}

#[test]
fn interaction_request_dispatches_typed_form_inputs() {
    let request = InteractionRequest::from(InteractionForm::for_handle(
        Handle::<()>::new(8),
        vec![("project_id".to_owned(), "42".to_owned())],
    ));
    let response = request
        .dispatch(
            interactions(BuildFingerprint(4))
                .on_form(Handle::<()>::new(8), |input: OpenProject| {
                    Slot::<String>::new(3).text(input.project_id.0)
                }),
        )
        .unwrap();

    assert_eq!(response.batch.fingerprint, BuildFingerprint(4));
    assert_eq!(response.batch.ops, vec![Slot::<String>::new(3).text(42)]);
}

#[test]
fn interaction_request_rejects_invalid_typed_form_inputs() {
    let request = InteractionRequest::from(InteractionForm::for_handle(
        Handle::<()>::new(8),
        vec![("project_id".to_owned(), "nope".to_owned())],
    ));
    let rejection = request
        .dispatch(
            interactions(BuildFingerprint(4))
                .on_form(Handle::<()>::new(8), |input: OpenProject| {
                    Slot::<String>::new(3).text(input.project_id.0)
                }),
        )
        .unwrap_err();

    assert!(matches!(
        rejection,
        DispatchRejection::InvalidForm { handle_id: 8, .. }
    ));
}

#[test]
fn interaction_request_dispatches_typed_state_handlers() {
    fn open_project(multiplier: u32, input: OpenProject) -> impl IntoEffect {
        Slot::<String>::new(3).text(input.project_id.0 * multiplier)
    }

    let request = InteractionRequest::from(InteractionForm::for_handle(
        Handle::<()>::new(9),
        vec![("project_id".to_owned(), "21".to_owned())],
    ));
    let response = request
        .dispatch(
            interactions(BuildFingerprint(4))
                .with_state(2_u32)
                .on(Handle::<()>::new(9), open_project),
        )
        .unwrap();

    assert_eq!(response.batch.ops, vec![Slot::<String>::new(3).text(42)]);
}

#[derive(Debug)]
struct HandlerBoom;

impl std::fmt::Display for HandlerBoom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("database unavailable")
    }
}

impl std::error::Error for HandlerBoom {}

impl IntoHandlerFailure for HandlerBoom {
    fn into_handler_failure(self, _context: HandlerErrorContext) -> HandlerFailure {
        HandlerFailure::internal(self.to_string())
    }
}

#[derive(Debug)]
struct UiFailure;

impl IntoHandlerFailure for UiFailure {
    fn into_handler_failure(self, context: HandlerErrorContext) -> HandlerFailure {
        HandlerFailure::effects(Slot::<String>::new(3).text("try again"), context)
    }
}

#[tokio::test]
async fn interaction_request_dispatches_async_typed_state_extractors() {
    async fn open_project(
        State(multiplier): State<u32>,
        Form(input): Form<OpenProject>,
    ) -> impl IntoEffect {
        Slot::<String>::new(3).text(input.project_id.0 * multiplier)
    }

    let request = InteractionRequest::from(InteractionForm::for_handle(
        Handle::<()>::new(10),
        vec![("project_id".to_owned(), "21".to_owned())],
    ));
    let response = request
        .dispatch_async(
            interactions(BuildFingerprint(4))
                .with_state(2_u32)
                .on_async(Handle::<()>::new(10), open_project)
                .into_registry(),
        )
        .await
        .unwrap();

    assert_eq!(response.batch.ops, vec![Slot::<String>::new(3).text(42)]);
}

#[tokio::test]
async fn interaction_request_reports_async_result_handler_errors() {
    async fn open_project(
        State(_multiplier): State<u32>,
        Form(_input): Form<OpenProject>,
    ) -> Result<impl IntoEffect, HandlerBoom> {
        Err::<(), _>(HandlerBoom)
    }

    let request = InteractionRequest::from(InteractionForm::for_handle(
        Handle::<()>::new(11),
        vec![("project_id".to_owned(), "21".to_owned())],
    ));
    let error = request
        .dispatch_async(
            interactions(BuildFingerprint(4))
                .with_state(2_u32)
                .on_async_result(Handle::<()>::new(11), open_project)
                .into_registry(),
        )
        .await
        .unwrap_err();

    assert_eq!(
        error,
        DispatchRejection::HandlerError(HandlerFailure::Response {
            status: axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            message: "database unavailable".to_owned(),
        })
    );
}

#[tokio::test]
async fn interaction_request_maps_async_result_errors_to_effects() {
    async fn open_project(
        State(_multiplier): State<u32>,
        Form(_input): Form<OpenProject>,
    ) -> Result<impl IntoEffect, UiFailure> {
        Err::<(), _>(UiFailure)
    }

    let request = InteractionRequest::from(InteractionForm::for_handle(
        Handle::<()>::new(12),
        vec![("project_id".to_owned(), "21".to_owned())],
    ));
    let response = request
        .dispatch_async(
            interactions(BuildFingerprint(4))
                .with_state(2_u32)
                .on_async_result(Handle::<()>::new(12), open_project)
                .into_registry(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.batch.ops,
        vec![Slot::<String>::new(3).text("try again")]
    );
}

#[test]
fn interaction_request_maps_sync_result_errors_to_effects() {
    fn open_project(
        State(_multiplier): State<u32>,
        Form(_input): Form<OpenProject>,
    ) -> Result<impl IntoEffect, UiFailure> {
        Err::<(), _>(UiFailure)
    }

    let request = InteractionRequest::from(InteractionForm::for_handle(
        Handle::<()>::new(13),
        vec![("project_id".to_owned(), "21".to_owned())],
    ));
    let response = request
        .dispatch(
            interactions(BuildFingerprint(4))
                .with_state(2_u32)
                .on_result(Handle::<()>::new(13), open_project)
                .into_registry(),
        )
        .unwrap();

    assert_eq!(
        response.batch.ops,
        vec![Slot::<String>::new(3).text("try again")]
    );
}

#[test]
fn interactions_dispatch_by_checked_handle() {
    let title = Slot::<String>::new(7);
    let create = Handle::<()>::new(42);
    let request = InteractionRequest::from(InteractionForm::for_handle(
        create,
        [("title".into(), "Hello".into())],
    ));

    let response = request
        .dispatch(interactions(BuildFingerprint(123)).on(create, move |form| {
            title.text(form.value("title").unwrap_or(""))
        }))
        .unwrap();

    assert_eq!(response.batch.fingerprint, BuildFingerprint(123));
    assert_eq!(response.batch.ops, vec![title.text("Hello")]);
}

#[tokio::test]
async fn registry_contains_registered_sync_and_async_handles_and_async_falls_back_to_sync() {
    let sync = Handle::<()>::new(21);
    let asynchronous = Handle::<()>::new(22);
    let registry = interactions(BuildFingerprint(55))
        .on(sync, |_| Slot::<String>::new(1).text("sync"))
        .on_async(asynchronous, |_| async {
            Slot::<String>::new(1).text("async")
        });

    assert!(registry.contains(sync.id().id));
    assert!(registry.contains(asynchronous.id().id));
    assert!(!registry.contains(99));

    let sync_response = InteractionRequest::from(InteractionForm::for_handle(sync, []))
        .dispatch_async(registry)
        .await
        .unwrap();
    assert_eq!(sync_response.batch.fingerprint, BuildFingerprint(55));
    assert_eq!(
        sync_response.batch.ops,
        vec![Slot::<String>::new(1).text("sync")]
    );
}

#[tokio::test]
async fn state_result_registration_paths_preserve_state_form_and_failures() {
    let sync = Handle::<()>::new(30);
    let typed = Handle::<()>::new(31);
    let asynchronous = Handle::<()>::new(32);
    let typed_async = Handle::<()>::new(33);
    let typed_plain = Handle::<()>::new(34);
    let typed_state_plain = Handle::<()>::new(37);
    let typed_async_plain = Handle::<()>::new(35);
    let state_async_plain = Handle::<()>::new(36);
    let registry = interactions(BuildFingerprint(88))
        .on_state_result(sync, String::from("sync"), |state: State<String>| {
            Ok::<_, HandlerBoom>(Slot::<String>::new(1).text(state.0))
        })
        .on_state_form_result(
            typed,
            String::from("typed"),
            |state: State<String>, Form(form): Form<OpenProject>| {
                Err::<Effect, _>(HandlerBoom).inspect_err(|_error| {
                    let _ = (state, form);
                })
            },
        )
        .on_state_async_result(
            asynchronous,
            String::from("async"),
            |state: State<String>| async move {
                Ok::<_, HandlerBoom>(Slot::<String>::new(1).text(state.0))
            },
        )
        .on_form(typed_plain, |Form(form): Form<OpenProject>| {
            Slot::<String>::new(1).text(format!("typed:{}", form.project_id.0))
        })
        .on_state_form(
            typed_state_plain,
            String::from("typed-state"),
            |state: State<String>, Form(form): Form<OpenProject>| {
                Slot::<String>::new(1).text(format!("{}:{}", state.0, form.project_id.0))
            },
        )
        .on_form_async(
            typed_async_plain,
            |Form(form): Form<OpenProject>| async move {
                Slot::<String>::new(1).text(format!("typed-plain-async:{}", form.project_id.0))
            },
        )
        .on_state_async(
            state_async_plain,
            String::from("state-async"),
            |state: State<String>| async move { Slot::<String>::new(1).text(state.0) },
        )
        .on_state_form_async_result(
            typed_async,
            String::from("typed-async"),
            |state: State<String>, Form(form): Form<OpenProject>| async move {
                Ok::<_, HandlerBoom>(
                    Slot::<String>::new(1).text(format!("{}:{}", state.0, form.project_id.0)),
                )
            },
        );

    for handle in [
        sync,
        typed,
        asynchronous,
        typed_async,
        typed_plain,
        typed_state_plain,
        typed_async_plain,
        state_async_plain,
    ] {
        assert!(registry.contains(handle.id().id));
    }
    assert!(registry
        .dispatch(InteractionForm::for_handle(sync, []))
        .is_ok());
    let invalid_form = registry
        .dispatch(InteractionForm::for_handle(
            typed,
            [(String::from("project_id"), String::from("invalid"))],
        ))
        .unwrap_err();
    assert_eq!(
        invalid_form,
        DispatchRejection::InvalidForm {
            handle_id: typed.id().id,
            message: "invalid form field `project_id`".into(),
        }
    );
    let error = registry
        .dispatch(InteractionForm::for_handle(
            typed,
            [(String::from("project_id"), String::from("7"))],
        ))
        .unwrap_err();
    assert_eq!(
        error,
        DispatchRejection::HandlerError(HandlerFailure::internal("database unavailable"))
    );
    assert!(registry
        .dispatch_async(InteractionForm::for_handle(asynchronous, []))
        .await
        .is_ok());
    let invalid_plain = registry
        .dispatch(InteractionForm::for_handle(
            typed_plain,
            [(String::from("project_id"), String::from("invalid"))],
        ))
        .unwrap_err();
    assert!(matches!(
        invalid_plain,
        DispatchRejection::InvalidForm { .. }
    ));
    let invalid_state_plain = registry
        .dispatch(InteractionForm::for_handle(
            typed_state_plain,
            [(String::from("project_id"), String::from("invalid"))],
        ))
        .unwrap_err();
    assert!(matches!(
        invalid_state_plain,
        DispatchRejection::InvalidForm { .. }
    ));
    assert!(registry
        .dispatch_async(InteractionForm::for_handle(
            typed_async_plain,
            [(String::from("project_id"), String::from("8"))],
        ))
        .await
        .is_ok());
    assert!(registry
        .dispatch_async(InteractionForm::for_handle(state_async_plain, []))
        .await
        .is_ok());
    let response = registry
        .dispatch_async(InteractionForm::for_handle(
            typed_async,
            [(String::from("project_id"), String::from("9"))],
        ))
        .await
        .unwrap();
    assert_eq!(
        response.batch.ops,
        vec![Slot::<String>::new(1).text("typed-async:9")]
    );
}

#[test]
fn interactions_reject_unknown_handle_ids() {
    let request = InteractionRequest::from(InteractionForm::new(9, []));

    assert_eq!(
        request
            .dispatch(interactions(BuildFingerprint(123)))
            .unwrap_err(),
        DispatchRejection::UnknownHandle(9)
    );
}

#[tokio::test]
async fn effect_and_dispatch_responses_preserve_status_wire_and_diagnostics() {
    let batch = Slot::<String>::new(3)
        .text("saved")
        .into_batch(BuildFingerprint(77));
    let response = EffectResponse {
        batch: batch.clone(),
    }
    .into_response();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], HEMX_CONTENT_TYPE);
    assert_eq!(response.headers()[HEMX_FINGERPRINT_HEADER], "77");
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    assert_eq!(hemx_core::EffectBatch::from_wire(&body), Ok(batch.clone()));

    for (rejection, status, message) in [
        (
            DispatchRejection::UnknownHandle(9),
            StatusCode::NOT_FOUND,
            "unknown hemx handle id 9",
        ),
        (
            DispatchRejection::InvalidForm {
                handle_id: 9,
                message: "missing title".into(),
            },
            StatusCode::BAD_REQUEST,
            "invalid hemx form for handle id 9: missing title",
        ),
        (
            DispatchRejection::HandlerError(HandlerFailure::Response {
                status: StatusCode::CONFLICT,
                message: "stale".into(),
            }),
            StatusCode::CONFLICT,
            "stale",
        ),
    ] {
        let response = rejection.into_response();
        assert_eq!(response.status(), status);
        assert_eq!(
            to_bytes(response.into_body(), 4096).await.unwrap().as_ref(),
            message.as_bytes()
        );
    }

    let response = DispatchRejection::HandlerError(HandlerFailure::Effects(batch)).into_response();
    assert_eq!(response.headers()[header::CONTENT_TYPE], HEMX_CONTENT_TYPE);
}

#[tokio::test]
async fn runtime_js_response_serves_embedded_runtime() {
    let response = runtime_js().into_response();

    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        HEMX_RUNTIME_CONTENT_TYPE
    );
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );
    assert_eq!(
        response.headers()[header::ETAG],
        format!("\"{}\"", runtime_js_hash())
    );
    assert_eq!(
        response.headers()[header::CONTENT_LENGTH],
        runtime_js_source().len().to_string()
    );
    assert_eq!(
        to_bytes(response.into_body(), runtime_js_source().len() + 1)
            .await
            .unwrap()
            .as_ref(),
        runtime_js_source().as_bytes()
    );
}

#[test]
fn runtime_js_path_is_content_hashed() {
    let path = runtime_js_path();

    assert!(path.starts_with("/hemx."));
    assert!(path.ends_with(".js"));
    assert!(path.contains(runtime_js_hash()));
    assert_eq!(runtime_js_script_src(), path);
    assert_eq!(runtime_js_route_path(), path);
    assert_eq!(runtime_js_hash().len(), 64);
}
