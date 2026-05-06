use crate::{
    agent::{
        secrets::{redact_agent_secret_markers, AgentSecretKind, AgentSecretStore},
        tools::ceo_read::CeoReadToolSchema,
    },
    app_error::{codes, CommandError, CommandResult},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    sync::{Mutex, PoisonError},
    time::Duration,
};

const OPENAI_RESPONSES_URL: &str = "https://api.openai.com/v1/responses";
const OPENAI_TIMEOUT_SECONDS: u64 = 30;
const OPENAI_MAX_RESPONSE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum ProviderTurn {
    ToolCalls {
        calls: Vec<ProviderToolCall>,
        response_items: Vec<Value>,
    },
    FinalText(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderToolCall {
    pub call_id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderToolOutput {
    pub call_id: String,
    pub output: Value,
}

#[derive(Debug, Clone)]
pub struct ProviderRequest {
    pub model: String,
    pub system: String,
    pub user: String,
    pub tools: Vec<CeoReadToolSchema>,
    pub response_items: Vec<Value>,
    pub tool_outputs: Vec<ProviderToolOutput>,
}

impl ProviderRequest {
    pub fn new(
        model: impl Into<String>,
        system: impl Into<String>,
        user: impl Into<String>,
        tools: Vec<CeoReadToolSchema>,
    ) -> Self {
        Self {
            model: model.into(),
            system: system.into(),
            user: user.into(),
            tools,
            response_items: Vec::new(),
            tool_outputs: Vec::new(),
        }
    }

    pub fn with_response_items(mut self, response_items: Vec<Value>) -> Self {
        self.response_items = response_items;
        self
    }

    pub fn with_tool_outputs(mut self, tool_outputs: Vec<ProviderToolOutput>) -> Self {
        self.tool_outputs = tool_outputs;
        self
    }
}

pub trait AiProvider: Send + Sync {
    fn create_turn<'a>(
        &'a self,
        request: ProviderRequest,
    ) -> Pin<Box<dyn Future<Output = CommandResult<ProviderTurn>> + Send + 'a>>;
}

#[derive(Debug, Serialize)]
pub struct OpenAiResponsesRequest {
    pub model: String,
    pub instructions: String,
    pub input: Value,
    pub tools: Vec<Value>,
    pub store: Option<bool>,
}

impl OpenAiResponsesRequest {
    pub fn new(model: &str, system: &str, user: &str, tools: &[CeoReadToolSchema]) -> Self {
        Self {
            model: model.to_string(),
            instructions: system.to_string(),
            input: json!(user),
            tools: function_tools(tools),
            store: Some(false),
        }
    }

    fn from_provider_request(request: &ProviderRequest) -> CommandResult<Self> {
        let mut dto = Self::new(
            &request.model,
            &request.system,
            &request.user,
            &request.tools,
        );

        if !request.tool_outputs.is_empty() {
            if request.response_items.is_empty() {
                return Err(scrub_provider_error(
                    "provider continuation missing prior response items",
                ));
            }

            let mut input =
                Vec::with_capacity(request.response_items.len() + request.tool_outputs.len() + 1);
            input.push(json!({
                "role": "user",
                "content": request.user,
            }));
            input.extend(request.response_items.iter().cloned());
            input.extend(request.tool_outputs.iter().map(|output| {
                json!({
                    "type": "function_call_output",
                    "call_id": output.call_id,
                    "output": output.output.to_string(),
                })
            }));
            dto.input = Value::Array(input);
        }

        Ok(dto)
    }
}

pub struct OpenAiProvider<S: AgentSecretStore> {
    secrets: S,
    client: reqwest::Client,
    endpoint: String,
}

impl<S: AgentSecretStore> OpenAiProvider<S> {
    pub fn new(secrets: S) -> CommandResult<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(OPENAI_TIMEOUT_SECONDS))
            .build()
            .map_err(|error| scrub_provider_error(&error.to_string()))?;

        Ok(Self {
            secrets,
            client,
            endpoint: OPENAI_RESPONSES_URL.to_string(),
        })
    }

    pub fn with_client(secrets: S, client: reqwest::Client, endpoint: impl Into<String>) -> Self {
        Self {
            secrets,
            client,
            endpoint: endpoint.into(),
        }
    }

    async fn create_turn_inner(&self, request: ProviderRequest) -> CommandResult<ProviderTurn> {
        let api_key = self
            .secrets
            .get_secret(AgentSecretKind::OpenAiApiKey)?
            .ok_or_else(|| {
                CommandError::user(
                    codes::AGENT_SECRET_MISSING,
                    "Missing OpenAI API key for CEO Telegram Chat.",
                )
            })?;
        let body = OpenAiResponsesRequest::from_provider_request(&request)?;

        let response = self
            .client
            .post(&self.endpoint)
            .bearer_auth(api_key)
            .json(&body)
            .send()
            .await
            .map_err(|error| scrub_provider_error(&error.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            return Err(scrub_provider_error(&format!(
                "provider returned HTTP {status}"
            )));
        }

        let bytes = limited_response_bytes(response).await?;
        parse_provider_turn(&bytes)
    }
}

impl<S: AgentSecretStore> AiProvider for OpenAiProvider<S> {
    fn create_turn<'a>(
        &'a self,
        request: ProviderRequest,
    ) -> Pin<Box<dyn Future<Output = CommandResult<ProviderTurn>> + Send + 'a>> {
        Box::pin(async move { self.create_turn_inner(request).await })
    }
}

#[derive(Default)]
pub struct FakeAiProvider {
    turns: Mutex<VecDeque<Result<ProviderTurn, CommandError>>>,
}

impl FakeAiProvider {
    pub fn push_turn(&self, turn: Result<ProviderTurn, CommandError>) {
        self.turns
            .lock()
            .expect("fake provider queue lock")
            .push_back(turn);
    }
}

impl AiProvider for FakeAiProvider {
    fn create_turn<'a>(
        &'a self,
        _request: ProviderRequest,
    ) -> Pin<Box<dyn Future<Output = CommandResult<ProviderTurn>> + Send + 'a>> {
        Box::pin(async move {
            self.turns
                .lock()
                .map_err(provider_lock_error)?
                .pop_front()
                .unwrap_or_else(|| Err(scrub_provider_error("fake provider queue is empty")))
        })
    }
}

pub fn scrub_provider_error(message: &str) -> CommandError {
    CommandError::system(
        codes::AGENT_PROVIDER_REQUEST_FAILED,
        redact_agent_secret_markers(message),
    )
}

fn function_tools(tools: &[CeoReadToolSchema]) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
                "strict": true,
            })
        })
        .collect()
}

async fn limited_response_bytes(response: reqwest::Response) -> CommandResult<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > OPENAI_MAX_RESPONSE_BYTES)
    {
        return Err(scrub_provider_error(
            "provider response exceeded size limit",
        ));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|error| scrub_provider_error(&error.to_string()))?;
    if bytes.len() as u64 > OPENAI_MAX_RESPONSE_BYTES {
        return Err(scrub_provider_error(
            "provider response exceeded size limit",
        ));
    }

    Ok(bytes.to_vec())
}

fn parse_provider_turn(bytes: &[u8]) -> CommandResult<ProviderTurn> {
    let response: OpenAiResponsesResponse = serde_json::from_slice(bytes)
        .map_err(|error| scrub_provider_error(&format!("invalid provider response: {error}")))?;
    let mut tool_calls = Vec::new();
    let mut final_text = None;

    for output in &response.output {
        match output.get("type").and_then(Value::as_str) {
            Some("function_call") => tool_calls.push(parse_tool_call(output.clone())?),
            Some("message") => {
                if final_text.is_none() {
                    final_text = parse_message_text(output);
                }
            }
            _ => {}
        }
    }

    if !tool_calls.is_empty() {
        return Ok(ProviderTurn::ToolCalls {
            calls: tool_calls,
            response_items: response.output,
        });
    }

    final_text
        .filter(|text| !text.trim().is_empty())
        .map(ProviderTurn::FinalText)
        .ok_or_else(|| scrub_provider_error("provider response did not contain usable output"))
}

fn parse_tool_call(raw: Value) -> CommandResult<ProviderToolCall> {
    let call_id = string_field(&raw, "call_id")
        .or_else(|| string_field(&raw, "id"))
        .ok_or_else(|| scrub_provider_error("provider function call missing call_id"))?;
    let name = string_field(&raw, "name")
        .ok_or_else(|| scrub_provider_error("provider function call missing name"))?;
    let arguments = match raw.get("arguments") {
        Some(Value::String(arguments)) => serde_json::from_str(arguments).map_err(|error| {
            scrub_provider_error(&format!(
                "provider function call has invalid arguments: {error}"
            ))
        })?,
        Some(Value::Object(_)) => raw["arguments"].clone(),
        Some(Value::Null) | None => json!({}),
        _ => {
            return Err(scrub_provider_error(
                "provider function call arguments had unsupported shape",
            ))
        }
    };

    Ok(ProviderToolCall {
        call_id,
        name,
        arguments,
    })
}

fn parse_message_text(raw: &Value) -> Option<String> {
    raw.get("content")?.as_array()?.iter().find_map(|content| {
        let content_type = content.get("type").and_then(Value::as_str);
        if content_type == Some("output_text") || content.get("text").is_some() {
            content
                .get("text")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        } else {
            None
        }
    })
}

fn string_field(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn provider_lock_error<T>(error: PoisonError<T>) -> CommandError {
    scrub_provider_error(&format!("provider queue lock failed: {error}"))
}

#[derive(Debug, Deserialize)]
struct OpenAiResponsesResponse {
    output: Vec<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tools::ceo_read::CeoReadToolSchema;

    #[test]
    fn openai_tool_request_contains_only_function_tools() {
        let tools = vec![CeoReadToolSchema {
            name: "get_hotel_status".to_string(),
            description: "Read status".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false,
            }),
        }];

        let request = OpenAiResponsesRequest::new("gpt-5", "system", "user", &tools);

        assert_eq!(request.tools[0]["type"], "function");
        assert_eq!(request.tools[0]["name"], "get_hotel_status");
        assert!(request.store.is_none_or(|store| !store));
    }

    #[test]
    fn provider_error_scrubbing_removes_secret_markers() {
        let err = scrub_provider_error("request failed with sk-secret and telegram_bot_token=abc");

        assert!(!err.message.contains("sk-secret"));
        assert!(!err.message.contains("abc"));
    }

    #[test]
    fn parse_tool_call_turn_preserves_returned_response_items() {
        let reasoning_item = serde_json::json!({
            "type": "reasoning",
            "id": "rs_123",
            "summary": [{"type": "summary_text", "text": "Need hotel status."}],
        });
        let function_call_item = serde_json::json!({
            "type": "function_call",
            "id": "fc_123",
            "call_id": "call_123",
            "name": "get_hotel_status",
            "arguments": "{}",
        });
        let response = serde_json::json!({
            "output": [reasoning_item.clone(), function_call_item.clone()],
        });

        let turn = parse_provider_turn(response.to_string().as_bytes()).expect("parse turn");

        match turn {
            ProviderTurn::ToolCalls {
                calls,
                response_items,
            } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].call_id, "call_123");
                assert_eq!(response_items, vec![reasoning_item, function_call_item]);
            }
            ProviderTurn::FinalText(_) => panic!("expected tool call turn"),
        }
    }

    #[test]
    fn follow_up_request_includes_prior_response_items_before_tool_outputs() {
        let prior_response_item = serde_json::json!({
            "type": "function_call",
            "id": "fc_123",
            "call_id": "call_123",
            "name": "get_hotel_status",
            "arguments": "{}",
        });
        let request = ProviderRequest::new("gpt-5", "system", "user", Vec::new())
            .with_response_items(vec![prior_response_item.clone()])
            .with_tool_outputs(vec![ProviderToolOutput {
                call_id: "call_123".to_string(),
                output: serde_json::json!({"ok": true}),
            }]);

        let openai_request =
            OpenAiResponsesRequest::from_provider_request(&request).expect("request DTO");

        assert_eq!(openai_request.store, Some(false));
        assert_eq!(openai_request.input[0]["role"], "user");
        assert_eq!(openai_request.input[1], prior_response_item);
        assert_eq!(openai_request.input[2]["type"], "function_call_output");
        assert_eq!(openai_request.input[2]["call_id"], "call_123");
    }

    #[test]
    fn follow_up_with_tool_outputs_without_response_items_is_rejected() {
        let request = ProviderRequest::new("gpt-5", "system", "user", Vec::new())
            .with_tool_outputs(vec![ProviderToolOutput {
                call_id: "call_123".to_string(),
                output: serde_json::json!({"ok": true}),
            }]);

        let error = OpenAiResponsesRequest::from_provider_request(&request)
            .expect_err("tool output continuation must include prior response context");

        assert_eq!(error.code, codes::AGENT_PROVIDER_REQUEST_FAILED);
    }
}
