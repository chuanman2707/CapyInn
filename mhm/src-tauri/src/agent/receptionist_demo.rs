use crate::app_error::{codes, CommandError, CommandResult};
use serde::{Deserialize, Serialize};

pub const DEFAULT_LOCAL_RECEPTIONIST_ENDPOINT: &str = "http://127.0.0.1:8080/v1/chat/completions";
pub const DEFAULT_LOCAL_RECEPTIONIST_MODEL: &str = "capyinn-gemma4-e2b-q5km";
const MAX_MESSAGE_CHARS: usize = 2_000;
const MAX_MODEL_CHARS: usize = 128;
const MAX_ENDPOINT_CHARS: usize = 2_048;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct LocalReceptionistChatRequest {
    pub endpoint: String,
    pub model: String,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct LocalReceptionistChatResponse {
    pub answer: String,
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValidatedLocalReceptionistRequest {
    pub(crate) endpoint: String,
    pub(crate) model: String,
    pub(crate) message: String,
}

pub(crate) fn validate_request(
    request: LocalReceptionistChatRequest,
) -> CommandResult<ValidatedLocalReceptionistRequest> {
    let endpoint = request.endpoint.trim();
    let model = request.model.trim();
    let message = request.message.trim();

    validate_local_http_endpoint(endpoint)?;

    if model.is_empty() {
        return Err(validation_error("Model is required"));
    }
    if model.chars().count() > MAX_MODEL_CHARS {
        return Err(validation_error("Model is too long"));
    }
    if model.chars().any(char::is_control) {
        return Err(validation_error("Model contains invalid characters"));
    }

    if message.is_empty() {
        return Err(validation_error("Message is required"));
    }
    if message.chars().count() > MAX_MESSAGE_CHARS {
        return Err(validation_error("Message is too long"));
    }

    Ok(ValidatedLocalReceptionistRequest {
        endpoint: endpoint.to_string(),
        model: model.to_string(),
        message: message.to_string(),
    })
}

fn validate_local_http_endpoint(endpoint: &str) -> CommandResult<()> {
    if endpoint.is_empty() {
        return Err(validation_error("Endpoint is required"));
    }
    if endpoint.chars().count() > MAX_ENDPOINT_CHARS {
        return Err(validation_error("Endpoint is too long"));
    }

    let url = reqwest::Url::parse(endpoint).map_err(|_| validation_error("Endpoint is invalid"))?;
    if url.scheme() != "http" {
        return Err(validation_error("Endpoint must use local HTTP"));
    }

    match url.host_str() {
        Some("127.0.0.1" | "localhost") => Ok(()),
        _ => Err(validation_error("Endpoint must be local")),
    }
}

fn validation_error(message: &'static str) -> CommandError {
    CommandError::user(codes::VALIDATION_INVALID_INPUT, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_request() -> LocalReceptionistChatRequest {
        LocalReceptionistChatRequest {
            endpoint: DEFAULT_LOCAL_RECEPTIONIST_ENDPOINT.to_string(),
            model: DEFAULT_LOCAL_RECEPTIONIST_MODEL.to_string(),
            message: "Do you have hourly rooms?".to_string(),
        }
    }

    fn assert_validation_error(request: LocalReceptionistChatRequest) {
        let error = validate_request(request).expect_err("request must fail validation");
        assert_eq!(error.code, codes::VALIDATION_INVALID_INPUT);
        assert!(error.support_id.is_none());
    }

    #[test]
    fn validate_request_accepts_localhost_http_endpoint() {
        let mut request = valid_request();
        request.endpoint = "http://localhost:8080/v1/chat/completions".to_string();

        let validated = validate_request(request).expect("request should pass validation");

        assert_eq!(
            validated.endpoint,
            "http://localhost:8080/v1/chat/completions"
        );
    }

    #[test]
    fn validate_request_rejects_remote_endpoint() {
        let mut request = valid_request();
        request.endpoint = "https://example.com/v1/chat/completions".to_string();

        assert_validation_error(request);
    }

    #[test]
    fn validate_request_rejects_empty_message() {
        let mut request = valid_request();
        request.message = "   ".to_string();

        assert_validation_error(request);
    }

    #[test]
    fn validate_request_rejects_too_long_message() {
        let mut request = valid_request();
        request.message = "a".repeat(MAX_MESSAGE_CHARS + 1);

        assert_validation_error(request);
    }

    #[test]
    fn validate_request_rejects_empty_model() {
        let mut request = valid_request();
        request.model = "   ".to_string();

        assert_validation_error(request);
    }

    #[test]
    fn validate_request_rejects_control_characters_in_model() {
        let mut request = valid_request();
        request.model = "capyinn\nmodel".to_string();

        assert_validation_error(request);
    }

    #[test]
    fn validate_request_rejects_too_long_endpoint() {
        let mut request = valid_request();
        request.endpoint = format!("http://127.0.0.1:8080/{}", "a".repeat(MAX_ENDPOINT_CHARS));

        assert_validation_error(request);
    }
}
