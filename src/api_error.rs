use actix_web::{
    HttpResponse, ResponseError,
    http::{StatusCode, header::ContentType},
};
use std::fmt;

use crate::models::ErrorResponse;

pub type ApiResult<T> = Result<T, ApiError>;

#[derive(Debug, Clone)]
pub struct ApiError {
    status: StatusCode,
    code: String,
    details: Option<String>,
}

impl ApiError {
    pub fn new(status: StatusCode, code: impl Into<String>, details: Option<String>) -> Self {
        Self {
            status,
            code: code.into(),
            details,
        }
    }

    pub fn bad_request(code: impl Into<String>, details: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, Some(details.into()))
    }

    pub fn forbidden(code: impl Into<String>, details: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, code, Some(details.into()))
    }

    pub fn not_found(code: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, code, None)
    }

    pub fn database() -> Self {
        crate::observability::capture_internal_failure("database operation failed");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "db_error", None)
    }

    pub fn database_with_source(error: &dyn fmt::Display) -> Self {
        crate::observability::capture_internal_error("database operation failed", error);
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "db_error", None)
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.details {
            Some(details) => write!(f, "{}: {}", self.code, details),
            None => write!(f, "{}", self.code),
        }
    }
}

impl ResponseError for ApiError {
    fn status_code(&self) -> StatusCode {
        self.status
    }

    fn error_response(&self) -> HttpResponse {
        HttpResponse::build(self.status)
            .insert_header(ContentType::json())
            .json(ErrorResponse {
                error: self.code.clone(),
                details: self.details.clone(),
            })
    }
}

impl From<ApiError> for HttpResponse {
    fn from(error: ApiError) -> Self {
        error.error_response()
    }
}
