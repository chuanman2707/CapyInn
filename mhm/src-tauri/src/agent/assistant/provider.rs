use crate::{
    agent::secrets::redact_agent_secret_markers,
    app_error::{codes, CommandError, CommandResult},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

const PROVIDER_TIMEOUT_SECONDS: u64 = 30;
const MAX_PROVIDER_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_OUTPUT_TOKENS: u16 = 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawToolCall {
    pub id: String,
    #[serde(rename = "type", default = "default_tool_call_type")]
    pub call_type: String,
    pub function: RawToolCallFunction,
}

fn default_tool_call_type() -> String {
    "function".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawToolCallFunction {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<RawToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn assistant_tool_calls(tool_calls: Vec<RawToolCall>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: None,
            tool_calls: Some(tool_calls),
            tool_call_id: None,
        }
    }

    pub fn tool_result(tool_call_id: impl Into<String>, output: &Value) -> Self {
        Self {
            role: "tool".to_string(),
            content: Some(output.to_string()),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AssistantToolSchema {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AssistantToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AssistantProviderTurn {
    ToolCalls(Vec<AssistantToolCall>),
    FinalText(String),
}

#[derive(Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatCompletionChoice>,
}

#[derive(Deserialize)]
struct ChatCompletionChoice {
    message: ChatCompletionMessage,
}

#[derive(Deserialize)]
struct ChatCompletionMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<RawToolCall>>,
}

pub fn build_assistant_provider_client() -> CommandResult<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(PROVIDER_TIMEOUT_SECONDS))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| {
            CommandError::system(
                codes::AGENT_PROVIDER_REQUEST_FAILED,
                format!(
                    "Không dựng được kết nối tới máy chủ AI: {}",
                    redact_agent_secret_markers(&error.to_string())
                ),
            )
        })
}

pub struct AssistantProviderClient {
    client: reqwest::Client,
}

impl AssistantProviderClient {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    pub async fn call(
        &self,
        base_url: &str,
        api_key: &str,
        model: &str,
        messages: &[ChatMessage],
        tools: &[AssistantToolSchema],
    ) -> CommandResult<AssistantProviderTurn> {
        let body = serde_json::json!({
            "model": model,
            "messages": messages,
            "tools": tools
                .iter()
                .map(|tool| serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    }
                }))
                .collect::<Vec<_>>(),
            "temperature": 0.1,
            "max_tokens": MAX_OUTPUT_TOKENS,
            "stream": false,
        });

        let response = self
            .client
            .post(base_url)
            .bearer_auth(api_key)
            .json(&body)
            .send()
            .await
            .map_err(|error| {
                provider_failure(format!(
                    "Không gọi được máy chủ AI: {}",
                    redact_agent_secret_markers(&error.to_string())
                ))
            })?;

        let status = response.status();
        let raw = response.text().await.map_err(|error| {
            provider_failure(format!(
                "Không đọc được trả lời từ máy chủ AI: {}",
                redact_agent_secret_markers(&error.to_string())
            ))
        })?;

        if raw.len() > MAX_PROVIDER_RESPONSE_BYTES {
            return Err(provider_failure("Máy chủ AI trả về quá dài.".to_string()));
        }

        if !status.is_success() {
            return Err(map_non_success(status, &raw));
        }

        let parsed: ChatCompletionResponse = serde_json::from_str(&raw).map_err(|_| {
            provider_failure("Máy chủ AI trả về định dạng không hỗ trợ.".to_string())
        })?;

        let message = parsed
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| {
                provider_failure("Máy chủ AI trả về định dạng không hỗ trợ.".to_string())
            })?
            .message;

        if let Some(raw_calls) = message.tool_calls.filter(|calls| !calls.is_empty()) {
            let mut calls = Vec::with_capacity(raw_calls.len());
            for raw_call in raw_calls {
                // Đối số tool là một chuỗi JSON. Chuỗi rỗng nghĩa là tool không
                // cần tham số nào, không phải lỗi.
                let arguments = if raw_call.function.arguments.trim().is_empty() {
                    Value::Object(serde_json::Map::new())
                } else {
                    serde_json::from_str(&raw_call.function.arguments).map_err(|_| {
                        provider_failure(
                            "Máy chủ AI gửi đối số công cụ không đọc được.".to_string(),
                        )
                    })?
                };

                calls.push(AssistantToolCall {
                    id: raw_call.id,
                    name: raw_call.function.name,
                    arguments,
                });
            }
            return Ok(AssistantProviderTurn::ToolCalls(calls));
        }

        match message.content {
            Some(content) if !content.trim().is_empty() => {
                Ok(AssistantProviderTurn::FinalText(content))
            }
            _ => Err(provider_failure(
                "Máy chủ AI trả về định dạng không hỗ trợ.".to_string(),
            )),
        }
    }
}

fn map_non_success(status: reqwest::StatusCode, raw: &str) -> CommandError {
    let lowered = raw.to_ascii_lowercase();
    let mentions_tools = lowered.contains("function calling")
        || lowered.contains("function_call")
        || lowered.contains("does not support tool")
        || lowered.contains("tools are not supported");

    if status == reqwest::StatusCode::BAD_REQUEST && mentions_tools {
        return CommandError::user(
            codes::AGENT_MODEL_NO_TOOL_SUPPORT,
            redact_agent_secret_markers(
                "Model này không gọi được công cụ. Chọn model khác, ví dụ deepseek-chat.",
            ),
        );
    }

    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return provider_failure("Máy chủ AI từ chối khoá API. Kiểm tra lại khoá.".to_string());
    }

    provider_failure(format!("Máy chủ AI báo lỗi {}.", status.as_u16()))
}

fn provider_failure(message: String) -> CommandError {
    CommandError::system(
        codes::AGENT_PROVIDER_REQUEST_FAILED,
        redact_agent_secret_markers(&message),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::post, Json, Router};
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    async fn spawn_provider(router: Router) -> String {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to bind test provider");
        let addr = listener
            .local_addr()
            .expect("failed to read test provider address");

        tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("test provider failed");
        });

        format!("http://{addr}/v1/chat/completions")
    }

    fn client() -> AssistantProviderClient {
        AssistantProviderClient::new(build_assistant_provider_client().expect("client"))
    }

    fn sample_tools() -> Vec<AssistantToolSchema> {
        vec![AssistantToolSchema {
            name: "list_rooms_now",
            description: "Trạng thái phòng",
            parameters: serde_json::json!({ "type": "object", "properties": {} }),
        }]
    }

    #[tokio::test]
    async fn parses_a_final_text_answer() {
        let endpoint = spawn_provider(Router::new().route(
            "/v1/chat/completions",
            post(|| async {
                Json(serde_json::json!({
                    "choices": [{ "message": { "role": "assistant", "content": "Còn 3 phòng trống." } }]
                }))
            }),
        ))
        .await;

        let turn = client()
            .call(
                &endpoint,
                "sk-test",
                "deepseek-chat",
                &[ChatMessage::user("phòng nào trống")],
                &sample_tools(),
            )
            .await
            .expect("provider should answer");

        assert_eq!(
            turn,
            AssistantProviderTurn::FinalText("Còn 3 phòng trống.".to_string())
        );
    }

    #[tokio::test]
    async fn parses_tool_calls_and_decodes_json_arguments() {
        let endpoint = spawn_provider(Router::new().route(
            "/v1/chat/completions",
            post(|| async {
                Json(serde_json::json!({
                    "choices": [{
                        "message": {
                            "role": "assistant",
                            "content": null,
                            "tool_calls": [{
                                "id": "call_1",
                                "type": "function",
                                "function": {
                                    "name": "get_stay_charges",
                                    "arguments": "{\"room_number\":\"201\"}"
                                }
                            }]
                        }
                    }]
                }))
            }),
        ))
        .await;

        let turn = client()
            .call(
                &endpoint,
                "sk-test",
                "deepseek-chat",
                &[ChatMessage::user("phòng 201 nợ bao nhiêu")],
                &sample_tools(),
            )
            .await
            .expect("provider should answer");

        match turn {
            AssistantProviderTurn::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].id, "call_1");
                assert_eq!(calls[0].name, "get_stay_charges");
                assert_eq!(calls[0].arguments["room_number"], "201");
            }
            other => panic!("mong đợi tool call, nhận {other:?}"),
        }
    }

    #[tokio::test]
    async fn sends_the_api_key_as_a_bearer_header_and_never_in_the_body() {
        let captured = Arc::new(Mutex::new(None::<(Option<String>, serde_json::Value)>));
        let sink = Arc::clone(&captured);

        let endpoint = spawn_provider(Router::new().route(
            "/v1/chat/completions",
            post(
                move |headers: axum::http::HeaderMap, Json(body): Json<serde_json::Value>| {
                    let sink = Arc::clone(&sink);
                    async move {
                        let auth = headers
                            .get("authorization")
                            .and_then(|value| value.to_str().ok())
                            .map(str::to_string);
                        *sink.lock().expect("lock") = Some((auth, body));
                        Json(serde_json::json!({
                            "choices": [{ "message": { "role": "assistant", "content": "ok" } }]
                        }))
                    }
                },
            ),
        ))
        .await;

        client()
            .call(
                &endpoint,
                "sk-secret-value",
                "deepseek-chat",
                &[ChatMessage::user("chào")],
                &sample_tools(),
            )
            .await
            .expect("provider should answer");

        let (auth, body) = captured.lock().expect("lock").clone().expect("captured");
        assert_eq!(auth.as_deref(), Some("Bearer sk-secret-value"));
        assert!(!body.to_string().contains("sk-secret-value"));
        assert_eq!(body["model"], "deepseek-chat");
        assert_eq!(body["stream"], false);
        assert_eq!(body["tools"][0]["function"]["name"], "list_rooms_now");
    }

    #[tokio::test]
    async fn maps_a_model_without_tool_support_to_its_own_code() {
        let endpoint = spawn_provider(Router::new().route(
            "/v1/chat/completions",
            post(|| async {
                (
                    axum::http::StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": { "message": "Model does not support Function Calling" }
                    })),
                )
            }),
        ))
        .await;

        let error = client()
            .call(
                &endpoint,
                "sk-test",
                "deepseek-reasoner",
                &[ChatMessage::user("chào")],
                &sample_tools(),
            )
            .await
            .expect_err("phải báo lỗi");

        assert_eq!(error.code, codes::AGENT_MODEL_NO_TOOL_SUPPORT);
    }

    #[tokio::test]
    async fn maps_an_unauthorised_provider_to_a_request_failure() {
        let endpoint = spawn_provider(Router::new().route(
            "/v1/chat/completions",
            post(|| async { (axum::http::StatusCode::UNAUTHORIZED, "nope") }),
        ))
        .await;

        let error = client()
            .call(
                &endpoint,
                "sk-wrong",
                "deepseek-chat",
                &[ChatMessage::user("chào")],
                &sample_tools(),
            )
            .await
            .expect_err("phải báo lỗi");

        assert_eq!(error.code, codes::AGENT_PROVIDER_REQUEST_FAILED);
        assert!(error.message.contains("khoá"));
    }

    #[tokio::test]
    async fn never_leaks_the_api_key_into_an_error_message() {
        let endpoint = spawn_provider(Router::new().route(
            "/v1/chat/completions",
            post(|| async { (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "boom") }),
        ))
        .await;

        let error = client()
            .call(
                &endpoint,
                "sk-super-secret",
                "deepseek-chat",
                &[ChatMessage::user("chào")],
                &sample_tools(),
            )
            .await
            .expect_err("phải báo lỗi");

        assert!(!error.message.contains("sk-super-secret"));
    }

    #[tokio::test]
    async fn rejects_an_unreachable_provider() {
        // Cổng 1 trên loopback không có ai nghe.
        let error = client()
            .call(
                "http://127.0.0.1:1/v1/chat/completions",
                "sk-test",
                "deepseek-chat",
                &[ChatMessage::user("chào")],
                &sample_tools(),
            )
            .await
            .expect_err("phải báo lỗi");

        assert_eq!(error.code, codes::AGENT_PROVIDER_REQUEST_FAILED);
    }

    #[tokio::test]
    async fn rejects_a_malformed_response() {
        let endpoint = spawn_provider(Router::new().route(
            "/v1/chat/completions",
            post(|| async { Json(serde_json::json!({ "choices": [] })) }),
        ))
        .await;

        let error = client()
            .call(
                &endpoint,
                "sk-test",
                "deepseek-chat",
                &[ChatMessage::user("chào")],
                &sample_tools(),
            )
            .await
            .expect_err("phải báo lỗi");

        assert_eq!(error.code, codes::AGENT_PROVIDER_REQUEST_FAILED);
    }
}
