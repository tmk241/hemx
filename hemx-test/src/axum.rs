use crate::{handle_form_body, inspect_html_document, inspect_html_fragment, try_inspect_wire};
use axum::body::{to_bytes, Body};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode};
use axum::Router;
use hemx_core::{Handle, WireError, HEMX_CONTENT_TYPE};
use std::error::Error;
use std::fmt;
use tower::ServiceExt;

const HTML_CONTENT_TYPE: &str = "text/html";
const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded";
const DEFAULT_BODY_LIMIT: usize = 2 * 1024 * 1024;

/// Build a request against a real Axum [`Router`].
pub fn request(method: Method, uri: impl Into<String>) -> RouterRequest {
    RouterRequest {
        method,
        uri: uri.into(),
        headers: HeaderMap::new(),
        body: Vec::new(),
        body_limit: DEFAULT_BODY_LIMIT,
    }
}

/// Build a GET request against a real Axum [`Router`].
pub fn get(uri: impl Into<String>) -> RouterRequest {
    request(Method::GET, uri)
}

/// Build a POST request against a real Axum [`Router`].
pub fn post(uri: impl Into<String>) -> RouterRequest {
    request(Method::POST, uri)
}

/// An owned request builder for exercising a real Axum router in process.
#[derive(Clone, Debug)]
pub struct RouterRequest {
    method: Method,
    uri: String,
    headers: HeaderMap,
    body: Vec<u8>,
    body_limit: usize,
}

impl RouterRequest {
    pub fn header(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.headers.insert(name, value);
        self
    }

    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    /// Set a URL-encoded Hemx interaction form body from a typed handle.
    pub fn form<I>(mut self, handle: Handle<I>, fields: &[(&str, &str)]) -> Self {
        self.headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static(FORM_CONTENT_TYPE),
        );
        self.body = handle_form_body(handle, fields).into_bytes();
        self
    }

    /// Bound the buffered response body. The default is 2 MiB.
    pub fn body_limit(mut self, bytes: usize) -> Self {
        self.body_limit = bytes;
        self
    }

    /// Send this request through a real Axum router and buffer its response.
    pub async fn send(self, router: Router) -> Result<RouterResponse, RouterTestError> {
        let mut request = Request::builder()
            .method(self.method)
            .uri(&self.uri)
            .body(Body::from(self.body))
            .map_err(|error| RouterTestError::Request(error.to_string()))?;
        *request.headers_mut() = self.headers;

        let response = router
            .oneshot(request)
            .await
            .map_err(|error| RouterTestError::Router(error.to_string()))?;
        let (parts, body) = response.into_parts();
        let bytes =
            to_bytes(body, self.body_limit)
                .await
                .map_err(|error| RouterTestError::Body {
                    limit: self.body_limit,
                    message: error.to_string(),
                })?;

        Ok(RouterResponse {
            status: parts.status,
            headers: parts.headers,
            body: bytes.to_vec(),
        })
    }
}

/// An owned Axum response preserving status, headers, and raw body bytes.
#[derive(Clone, Debug)]
pub struct RouterResponse {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl RouterResponse {
    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn body(&self) -> &[u8] {
        &self.body
    }

    pub fn content_type(&self) -> Option<&str> {
        self.headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
    }

    pub fn text(&self) -> Result<&str, RouterTestError> {
        std::str::from_utf8(&self.body).map_err(|error| RouterTestError::Utf8(error.to_string()))
    }

    /// Decode an `application/hemx` response into an effect inspector.
    pub fn effects(&self) -> Result<crate::EffectInspector, RouterTestError> {
        self.expect_content_type(HEMX_CONTENT_TYPE)?;
        try_inspect_wire(&self.body).map_err(RouterTestError::Wire)
    }

    /// Parse a `text/html` response as a complete document.
    pub fn html_document(&self) -> Result<crate::HtmlInspector, RouterTestError> {
        self.expect_content_type(HTML_CONTENT_TYPE)?;
        let source = self.text()?.to_owned();
        Ok(inspect_html_document(source))
    }

    /// Parse a `text/html` response as a fragment.
    pub fn html_fragment(&self) -> Result<crate::HtmlInspector, RouterTestError> {
        self.expect_content_type(HTML_CONTENT_TYPE)?;
        let source = self.text()?.to_owned();
        Ok(inspect_html_fragment(source))
    }

    fn expect_content_type(&self, expected: &'static str) -> Result<(), RouterTestError> {
        let actual = self.content_type().map(str::to_owned);
        let media_type = actual
            .as_deref()
            .and_then(|value| value.split(';').next())
            .map(str::trim);
        if media_type == Some(expected) {
            Ok(())
        } else {
            Err(RouterTestError::ContentType { expected, actual })
        }
    }
}

/// A request, router, body, content-type, UTF-8, or Hemx wire failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RouterTestError {
    Request(String),
    Router(String),
    Body {
        limit: usize,
        message: String,
    },
    ContentType {
        expected: &'static str,
        actual: Option<String>,
    },
    Utf8(String),
    Wire(WireError),
}

impl fmt::Display for RouterTestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Request(message) => {
                write!(formatter, "could not build Axum test request: {message}")
            }
            Self::Router(message) => write!(
                formatter,
                "Axum router failed to serve test request: {message}"
            ),
            Self::Body { limit, message } => write!(
                formatter,
                "could not buffer Axum response body within {limit} bytes: {message}"
            ),
            Self::ContentType { expected, actual } => write!(
                formatter,
                "expected response content type {expected:?}, found {}",
                actual.as_deref().unwrap_or("no content type")
            ),
            Self::Utf8(message) => write!(formatter, "response body is not valid UTF-8: {message}"),
            Self::Wire(error) => write!(formatter, "invalid Hemx effect response: {error:?}"),
        }
    }
}

impl Error for RouterTestError {}
