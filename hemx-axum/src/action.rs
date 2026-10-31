//! One route answers both representations of a form action.
//!
//! The browser runtime asks for `application/hemx` effects; a plain form (no
//! JavaScript, or an unsupported browser) asks for a document. The same
//! registry dispatch runs either way, after the same extraction guards, so an
//! application declares only what a native submission should see.

use std::sync::Arc;

use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post, MethodRouter};
use axum::Router;
use hemx_core::Html;

use crate::ledger::{
    refusal, Claim, Ledger, Recorded, Token, HEMX_LEDGER_HEADER, HEMX_TOKEN_HEADER,
};
use crate::{
    runtime_js, runtime_js_route_path, DispatchRejection, HandlerFailure, HandlerRegistry,
    InteractionForm, InteractionRequest, PageResponse, HEMX_CONTENT_TYPE,
};

type Feedback = dyn Fn(&InteractionForm, &str) -> Html + Send + Sync;

/// What a native submission of an action receives.
///
/// Success redirects (303) to a fixed same-origin path, never to a location
/// taken from the request. Invalid input renders `feedback` from the submitted
/// text fields and the validation message, with status 422 and no mutation.
#[derive(Clone)]
pub struct Native {
    location: &'static str,
    feedback: Arc<Feedback>,
}

impl Native {
    /// Redirect successful native submissions to `location`, an absolute path
    /// on this origin such as `"/"`, and answer invalid ones with `feedback`.
    ///
    /// # Panics
    ///
    /// If `location` is not an absolute same-origin path.
    pub fn new(
        location: &'static str,
        feedback: impl Fn(&InteractionForm, &str) -> Html + Send + Sync + 'static,
    ) -> Self {
        let same_origin = location.starts_with('/')
            && !location.starts_with("//")
            && !location.starts_with("/\\")
            && !location.chars().any(|c| c.is_control() || c == '\\');
        assert!(
            same_origin,
            "native redirect location `{location}` must be an absolute path on this origin"
        );
        Self {
            location,
            feedback: Arc::new(feedback),
        }
    }
}

/// A POST route that dispatches `registry` for enhanced and native submissions.
///
/// Enhanced requests receive the handler's effects; native ones receive the
/// declared [`Native`] answer. Cross-site, stale-build and oversized requests
/// are rejected before any handler runs, exactly as for manual dispatch.
pub fn action<S>(registry: HandlerRegistry, native: Native) -> MethodRouter<S>
where
    S: Clone + Send + Sync + 'static,
{
    let registry = Arc::new(registry);
    post(move |headers: HeaderMap, request: InteractionRequest| {
        let registry = Arc::clone(&registry);
        let native = native.clone();
        async move { answer(&registry, &native, wants_effects(&headers), request).await }
    })
}

/// [`action`], deduplicating enhanced submissions by token in `ledger`.
///
/// An enhanced request's first arrival runs the handler as a detached task, so
/// a dropped connection cannot cancel it half-way, and records its complete
/// response. A resend with the same token receives that response (or 425 while
/// it still runs) and never runs the handler again. Responses declare the
/// ledger, which allows the browser runtime one automatic resend. Native
/// submissions and requests without a token are answered exactly like
/// [`action`].
pub fn action_with_ledger<S, L>(
    registry: HandlerRegistry,
    native: Native,
    ledger: L,
) -> MethodRouter<S>
where
    S: Clone + Send + Sync + 'static,
    L: Ledger,
{
    let registry = Arc::new(registry);
    let ledger = Arc::new(ledger);
    post(move |headers: HeaderMap, request: InteractionRequest| {
        let registry = Arc::clone(&registry);
        let ledger = Arc::clone(&ledger);
        let native = native.clone();
        async move {
            let enhanced = wants_effects(&headers);
            let Some(token) = headers.get(HEMX_TOKEN_HEADER).filter(|_| enhanced) else {
                return answer(&registry, &native, enhanced, request).await;
            };
            // Extraction already enforced media type, body limits and Fetch Metadata.
            // Check registry admission before looking up even an existing token.
            if let Err(rejection) = registry.validate_dispatch(request.form()) {
                return rejection.into_response();
            }
            let Some(token) = token.to_str().ok().and_then(Token::parse) else {
                return (StatusCode::BAD_REQUEST, "malformed submission token").into_response();
            };
            let mut response = match ledger.claim(&token, request.form().handle_id).await {
                Claim::New => {
                    let task = {
                        let registry = Arc::clone(&registry);
                        let ledger = Arc::clone(&ledger);
                        let token = token.clone();
                        tokio::spawn(async move {
                            record_action(&*ledger, &token, async move {
                                answer(&registry, &native, true, request).await
                            })
                            .await
                        })
                    };
                    task.await
                        .expect("ledger recording task panicked")
                        .into_response()
                }
                Claim::Done(recorded) => recorded.into_response(),
                claim => refusal(&claim).expect("non-dispatching claim has a refusal"),
            };
            response
                .headers_mut()
                .insert(HEMX_LEDGER_HEADER, HeaderValue::from_static("1"));
            response
        }
    })
}

/// A router serving the bundled browser runtime at its versioned path.
pub fn runtime_route<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new().route(runtime_js_route_path(), get(|| async { runtime_js() }))
}

fn wants_effects(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::ACCEPT)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|part| part.split(';').next())
        .any(|media| media.trim().eq_ignore_ascii_case(HEMX_CONTENT_TYPE))
}

async fn answer(
    registry: &HandlerRegistry,
    native: &Native,
    enhanced: bool,
    request: InteractionRequest,
) -> Response {
    // Feedback may need the submitted text, never re-uploaded files.
    let input = (!enhanced)
        .then(|| InteractionForm::new(request.form().handle_id, request.form().fields().to_vec()));
    let result = registry.dispatch_async(request.form).await;
    let Some(input) = input else {
        return result.into_response();
    };
    match result {
        Ok(_) => Redirect::to(native.location).into_response(),
        Err(DispatchRejection::InvalidForm { message, .. }) => feedback(native, &input, &message),
        // Failure effects are for the browser runtime; a document cannot apply them.
        Err(DispatchRejection::HandlerError(HandlerFailure::Effects(_))) => {
            feedback(native, &input, "the action could not be completed")
        }
        Err(rejection) => rejection.into_response(),
    }
}

pub(super) async fn record_action(
    ledger: &impl Ledger,
    token: &Token,
    response: impl std::future::Future<Output = Response> + Send + 'static,
) -> Recorded {
    // The detached task supervises dispatch and body collection in a child task,
    // so a panic becomes a terminal record even if the client has disconnected.
    let result = tokio::spawn(async move { Recorded::read(response.await).await }).await;
    let recorded = match result {
        Ok(Ok(recorded)) => recorded,
        Ok(Err(_)) | Err(_) => Recorded {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            headers: vec![(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/plain; charset=utf-8"),
            )],
            body: axum::body::Bytes::from_static(b"the action failed"),
        },
    };
    ledger.record(token, recorded).await
}

fn feedback(native: &Native, input: &InteractionForm, message: &str) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        PageResponse::full((native.feedback)(input, message)),
    )
        .into_response()
}
