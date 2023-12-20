use crate::{errors::AiCriticError, DoublingProgressBar};
use async_openai::{
    config::OpenAIConfig,
    error::OpenAIError,
    types::{
        ChatCompletionRequestMessage, ChatCompletionResponseFormat,
        ChatCompletionResponseFormatType, ChatCompletionResponseStream,
        CreateChatCompletionRequest, CreateChatCompletionRequestArgs,
        CreateChatCompletionStreamResponse, FinishReason,
    },
    Client,
};
use async_trait::async_trait;
use color_eyre::eyre::Result;
use futures::StreamExt;
use log::info;
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use tokio::time::timeout;

const MODEL: &str = "gpt-4-1106-preview";
//const MODEL: &str = "gpt-4"; // Try comparing.
const MAX_TOKENS: u16 = 4096;
const TEMPERATURE: f32 = 0.1;
const MAX_RETRIES: usize = 5;
const TIMEOUT_DURATION: std::time::Duration = std::time::Duration::from_secs(30);
// The OpenAI API has a bug where the model will return a stream of spaces and newlines instead of
// the actual text response. Eventually, this stream will exceed the max_tokens limit and the API
// will return a 'Length' stop reason in the response's ChatChoice. But there's no reason to wait
// for the full max_tokens to be exhausted with empty chunks before noticing the abnormal response.
// Instead, we'll allow only MAX_CONSECUTIVE_BLANKS consecutive empty chunks in the response stream.
const MAX_CONSECUTIVE_BLANKS: usize = 300;

#[derive(Debug, PartialEq)]
enum ProcessingOutcome {
    ApiSuccess(String, Option<FinishReason>),
    Retry,
    Done(Value),
}

// Define a trait for client behavior to allow testing without actually calling the OpenAI API.
#[async_trait]
pub trait OpenAIClientTrait {
    async fn create_chat_stream(
        &self,
        request: CreateChatCompletionRequest,
    ) -> Result<ChatCompletionResponseStream, OpenAIError>;
}

// Implement the trait for the real OpenAI Client.
#[async_trait]
impl OpenAIClientTrait for Client<OpenAIConfig> {
    async fn create_chat_stream(
        &self,
        request: CreateChatCompletionRequest,
    ) -> Result<ChatCompletionResponseStream, OpenAIError> {
        self.chat().create_stream(request).await
    }
}

pub struct ChatterJSON {
    client: Box<dyn OpenAIClientTrait + Send + Sync>,
}

#[cfg(test)]
impl ChatterJSON {
    pub fn with_client(client: Box<dyn OpenAIClientTrait + Send + Sync>) -> Self {
        ChatterJSON { client }
    }
}

impl ChatterJSON {
    pub fn new() -> Self {
        ChatterJSON {
            client: Box::new(Client::new()),
        }
    }

    fn create_request(
        msgs: &[ChatCompletionRequestMessage],
    ) -> Result<CreateChatCompletionRequest, color_eyre::eyre::Error> {
        let request = CreateChatCompletionRequestArgs::default()
            .model(MODEL)
            .max_tokens(MAX_TOKENS)
            .temperature(TEMPERATURE)
            .response_format(ChatCompletionResponseFormat {
                r#type: ChatCompletionResponseFormatType::JsonObject,
            })
            .n(1) // Return only one ChatChoice
            .messages(msgs)
            .build()?;
        Ok(request)
    }

    fn check_for_excessive_blanks(consecutive_blanks: &mut usize, content: &str) -> bool {
        *consecutive_blanks = if content.trim().is_empty() {
            *consecutive_blanks + 1
        } else {
            0
        };
        *consecutive_blanks > MAX_CONSECUTIVE_BLANKS
    }

    // Process the chunk, accumulating them into `chunks`. Also, watch for a finish reason to be
    // returned and watch for excessive blank chunks. Return true if the request should be retried.
    fn process_chunk(
        pb: &mut DoublingProgressBar,
        response: CreateChatCompletionStreamResponse,
        chunks: &mut Vec<String>,
        consecutive_blanks: &mut usize,
        last_finish_reason: &mut Option<FinishReason>,
    ) -> bool {
        if response.choices.len() > 1 {
            println!(
                "Expected 1 ChatChoice in response but received {}. Retrying.",
                response.choices.len()
            );
            return true;
        }
        let chat_choice = &response.choices[0];
        if let Some(ref content) = chat_choice.delta.content {
            chunks.push(content.clone());
            if content.trim().is_empty() {
                pb.dec();
            } else {
                pb.inc();
            }
            if Self::check_for_excessive_blanks(consecutive_blanks, content) {
                println!("Retrying due to too many empty chunks returned by the API.");
                return true;
            }
        }
        if let Some(reason) = chat_choice.finish_reason {
            *last_finish_reason = Some(reason);
        }
        false
    }

    // The OpenAI API stream will return chunks, each of which has some text and an optional finish
    // reason. This function collects all of the chunks into a single string and return the combined
    // text and the last finish reason which contains the reason the stream ended.
    async fn collect_chunks(
        &self,
        pb: &mut DoublingProgressBar,
        request: &CreateChatCompletionRequest,
    ) -> Result<ProcessingOutcome> {
        let mut stream = self.client.create_chat_stream(request.clone()).await?;
        let mut chunks = vec![];
        let mut last_finish_reason: Option<FinishReason> = None;

        let mut consecutive_blanks = 0;
        loop {
            match timeout(TIMEOUT_DURATION, stream.next()).await {
                Ok(Some(message)) => {
                    if Self::process_chunk(
                        pb,
                        message?,
                        &mut chunks,
                        &mut consecutive_blanks,
                        &mut last_finish_reason,
                    ) {
                        return Ok(ProcessingOutcome::Retry);
                    }
                }
                Ok(None) => {
                    break; // Stream finished.
                }
                Err(_) => {
                    println!("Request timed out. Retrying...");
                    return Ok(ProcessingOutcome::Retry);
                }
            }
        }
        Ok(ProcessingOutcome::ApiSuccess(
            chunks.join(""),
            last_finish_reason,
        ))
    }

    fn describe_value(value: &Value, indent: usize) {
        match value {
            Value::Object(map) if map.contains_key("lgtm") && map.contains_key("corrections") => {
                log::info!("{}> Found a Correction", "-".repeat(indent));
            }
            Value::Object(map) if map.contains_key("code") => {
                log::info!(
                    "{}> Found a Code (checking the value of map['code']):\n",
                    "-".repeat(indent)
                );
                Self::describe_value(&map["code"], indent + 2);
            }
            Value::Object(map) => {
                log::info!("{}> Found an object in JSON object:\n", "-".repeat(indent));
                log::info!(
                    "{}> [[[\nThe object is:\n{:?}\n]]]",
                    "-".repeat(indent),
                    &map
                );
                for k in map.keys() {
                    log::info!(
                        "{}> It has String key: ``{}``\n(checking the value...)",
                        "-".repeat(indent),
                        k
                    );
                    Self::describe_value(&map[k], indent + 2);
                }
            }
            Value::Array(array) => {
                log::info!("{}> Found array in JSON object:\n", "-".repeat(indent));
                for v in array {
                    Self::describe_value(v, indent + 2);
                }
            }
            Value::String(s) => {
                log::info!(
                    "{}> Found string in JSON object:\n{}",
                    "-".repeat(indent),
                    s
                );
            }
            Value::Number(n) => {
                log::info!("{}> Found number in JSON object: {}", "-".repeat(indent), n);
            }
            Value::Bool(b) => {
                log::info!(
                    "{}> Found boolean in JSON object: {}",
                    "-".repeat(indent),
                    b
                );
            }
            Value::Null => {
                log::info!("{}> Found null in JSON object", "-".repeat(indent));
            }
        }
    }

    // Process the JSON Value returned by the OpenAI API. In some of our System messages, we
    // instruct GPT-4 to generate a code snippet. The API should return an an Object (Map<String,
    // Value>) with a key 'code' and a String value. However, the value is sometimes an Object or
    // other value. This function will parse the known variations and return the correct Object
    // (Map<String, String>) as a Value so that can be parsed by serde into a Code object elsewhere.
    // If it can't find a parsable value, it will return a retry request.
    fn process_code_value(map: &Map<String, Value>) -> Result<ProcessingOutcome> {
        match map.get("code") {
            None => {
                log::info!("The 'code' value is missing. Retrying");
                Ok(ProcessingOutcome::Retry)
            }
            Some(Value::String(_)) => {
                // Ideal: The code value is a String.
                // This is expected if the code object isn't nested:
                //   [Object {"code": String("...")}]
                Ok(ProcessingOutcome::Done(Value::Object(map.clone())))
            }
            Some(Value::Object(m)) => {
                // The code value is an object instead of a String. For example:
                //    [Object {"code": Object("...")}]
                // Sometimes, the API returns the value as an Object that has the code as both the
                // key and the value. Weird! Check for this case and recover.
                if m.len() != 1 {
                    log::info!(
                        "Found an object for the 'code' value with {} keys. Retrying",
                        map.keys().len()
                    );
                    Ok(ProcessingOutcome::Retry)
                } else {
                    let (key, value) = m.iter().next().unwrap();
                    // Sometimes the API returns the code as the key and a comment as the value.
                    log::info!("Found a key / value for the 'code'. Returning the key");
                    log::info!("The Value is:");
                    Self::describe_value(value, 0);
                    Ok(ProcessingOutcome::Done(json!({ "code": key })))
                }
            }
            _ => {
                log::info!("Found an expected type for the 'code' value. Retrying; here it is:");
                Self::describe_value(map.get("code").unwrap(), 0);
                Ok(ProcessingOutcome::Retry)
            }
        }
    }

    // Process the JSON string returned by the OpenAI API when the STOP finish reason is returned.
    // Return it as a Value for further processing.
    fn process_stop(json_str: String) -> Result<ProcessingOutcome> {
        let value: Value = serde_json::from_str(&json_str)?;
        match &value {
            // Code objects need extra processing...
            Value::Object(map) if map.contains_key("code") => Self::process_code_value(map),
            Value::Object(_) => Ok(ProcessingOutcome::Done(value)),
            _ => Err(AiCriticError::UnexpectedJsonStructure { json: value }.into()),
        }
    }

    // Process the string and finish reason from the OpenAI API. Some finish reasons or
    // deserialization errors indicate that the response contained malformed JSON. If so, return a
    // ProcessingOutcome requesting to retry the request. Otherwise, process and return it.
    fn process_api_result(
        &self,
        pb: &mut DoublingProgressBar,
        json_str: String,
        finish_reason: Option<FinishReason>,
    ) -> Result<ProcessingOutcome> {
        match finish_reason {
            Some(FinishReason::Stop) => Self::process_stop(json_str),
            Some(FinishReason::Length) => {
                pb.clone().println("Retrying due to unfinished chat.");
                pb.reset_to_zero();
                Ok(ProcessingOutcome::Retry)
            }
            Some(r) => {
                pb.clone()
                    .println(&format!("Unexpected finish reason: {:?}. Retrying", r));
                pb.reset_to_zero();
                Ok(ProcessingOutcome::Retry)
            }
            None => {
                pb.clone()
                    .println("Missing finish reason. Retrying the request.");
                pb.reset_to_zero();
                Ok(ProcessingOutcome::Retry)
            }
        }
    }

    pub async fn chat(
        &self,
        pb: &mut DoublingProgressBar,
        msgs: &[ChatCompletionRequestMessage],
    ) -> Result<Value> {
        let request = Self::create_request(msgs)?;
        info!("   ==> Request: {:?}", request);

        for i in 1..=MAX_RETRIES {
            match self.collect_chunks(pb, &request).await {
                Ok(ProcessingOutcome::ApiSuccess(json_str, finish_reason)) => {
                    info!("   ==> Response: {}", json_str);
                    match self.process_api_result(pb, json_str, finish_reason)? {
                        ProcessingOutcome::Done(value) => return Ok(value),
                        ProcessingOutcome::Retry => {}
                        ProcessingOutcome::ApiSuccess(_, _) => unreachable!(),
                    }
                }
                Ok(ProcessingOutcome::Retry) => {
                    pb.reset_to_zero();
                }
                Ok(ProcessingOutcome::Done(_)) => unreachable!(),
                Err(e) => {
                    return Err(e);
                }
            };
            info!("Retry attempt: {}", i);
            println!("Retry attempt: {}", i);
        }

        Err(AiCriticError::MaxRetriesExceeded {
            retries: MAX_RETRIES,
        }
        .into())
    }

    // Validate fields from a JSON Value object. Return a list of missing fields as an error. Return
    // any extra fields as a result. If they're the same, the result will be empty.
    pub fn validate_fields(value: &Value, fields: Vec<&str>) -> Result<Vec<String>> {
        // Iterate over fields if it's an object.
        match value.as_object() {
            Some(obj) => {
                let obj_keys: HashSet<_> = obj.keys().cloned().collect();
                let fields_set: HashSet<_> = fields.iter().map(|&k| k.to_string()).collect();

                let missing_keys: Vec<_> = fields_set.difference(&obj_keys).cloned().collect();
                if !missing_keys.is_empty() {
                    return Err(AiCriticError::MissingJsonFields {
                        fields: missing_keys,
                    }
                    .into());
                }
                Ok(obj_keys.difference(&fields_set).cloned().collect())
            }
            None => Err(AiCriticError::NotJsonObject.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DoublingProgressBar;
    use async_openai::types::{
        ChatCompletionRequestUserMessageArgs, ChatCompletionResponseStreamMessage,
        ChatCompletionStreamResponseDelta, CreateChatCompletionStreamResponse, Role,
    };
    use async_openai::types::{CreateChatCompletionRequest, FinishReason};
    use async_trait::async_trait;
    use color_eyre::eyre::Result;
    use futures::stream;
    use mockall::{mock, predicate::*};
    use serde_json::json;

    fn create_message(msg: &str) -> ChatCompletionRequestMessage {
        ChatCompletionRequestUserMessageArgs::default()
            .content(msg)
            .build()
            .unwrap()
            .into()
    }

    fn create_chunk(
        msg: &str,
        finish_reason: Option<FinishReason>,
    ) -> CreateChatCompletionStreamResponse {
        let chat_choice = ChatCompletionResponseStreamMessage {
            index: 0,
            #[allow(deprecated)]
            delta: ChatCompletionStreamResponseDelta {
                content: Some(msg.to_string()),
                role: Some(Role::User),
                tool_calls: None,
                function_call: None, // Deprecated.
            },
            finish_reason,
        };

        CreateChatCompletionStreamResponse {
            id: "1234".to_string(),
            choices: vec![chat_choice],
            created: 12345,
            model: "test_model".to_string(),
            object: "chat.completion.chunk".to_string(),
            system_fingerprint: None,
        }
    }

    mock! {
        pub OpenAIClient {
            async fn create_chat_stream(&self, request: CreateChatCompletionRequest) -> Result<ChatCompletionResponseStream, OpenAIError>;
        }
    }

    #[async_trait]
    impl OpenAIClientTrait for MockOpenAIClient {
        async fn create_chat_stream(
            &self,
            request: CreateChatCompletionRequest,
        ) -> Result<ChatCompletionResponseStream, OpenAIError> {
            self.create_chat_stream(request).await
        }
    }

    fn make_mock(response_chunks: Vec<CreateChatCompletionStreamResponse>) -> MockOpenAIClient {
        let mock_stream = stream::iter(response_chunks.into_iter().map(Ok));

        // Setup the mock
        let mut mock = MockOpenAIClient::new();
        mock.expect_create_chat_stream()
            .returning(move |_| Ok(Box::pin(mock_stream.clone())));
        mock
    }

    ////////////////////////////////////////////////////////////////////////////////////////////////
    // check_for_excessive_blanks() tests
    ////////////////////////////////////////////////////////////////////////////////////////////////
    #[test]
    fn test_check_for_excessive_blanks() {
        let mut blanks = 0;

        assert!(!ChatterJSON::check_for_excessive_blanks(&mut blanks, ""));
        assert_eq!(blanks, 1);

        assert!(!ChatterJSON::check_for_excessive_blanks(&mut blanks, "a"));
        assert_eq!(blanks, 0);

        blanks = MAX_CONSECUTIVE_BLANKS;
        assert!(ChatterJSON::check_for_excessive_blanks(&mut blanks, "\n"));
        assert_eq!(blanks, MAX_CONSECUTIVE_BLANKS + 1);
    }

    ////////////////////////////////////////////////////////////////////////////////////////////////
    // process_stop() tests
    ////////////////////////////////////////////////////////////////////////////////////////////////
    #[test]
    fn test_process_stop_with_code() {
        let json_str = r#"{"code": "print('Hello, World!')"}"#.to_string();
        let result = ChatterJSON::process_stop(json_str).unwrap();
        assert_eq!(
            result,
            ProcessingOutcome::Done(json!({"code": "print('Hello, World!')"}))
        );
    }

    #[test]
    fn test_process_stop_with_invalid_json() {
        let json_str = r#"{"code": "print('Hello, World!')"#.to_string();
        let result = ChatterJSON::process_stop(json_str);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "EOF while parsing a string at line 1 column 32"
        );
    }

    #[test]
    fn test_process_stop_with_object_value() {
        let json_str = r#"{"key": "value"}"#.to_string();
        let result = ChatterJSON::process_stop(json_str).unwrap();
        assert_eq!(result, ProcessingOutcome::Done(json!({"key": "value"})));
    }

    #[test]
    fn test_process_stop_with_unexpected_json_structure() {
        let json_str = r#"["an", "array"]"#.to_string();
        let result = ChatterJSON::process_stop(json_str);
        assert!(result.is_err());

        let error = result.unwrap_err();
        assert!(matches!(
            error.downcast_ref::<AiCriticError>(),
            Some(AiCriticError::UnexpectedJsonStructure { json: _ })
        ));
    }

    ////////////////////////////////////////////////////////////////////////////////////////////////
    // process_api_result() tests
    ////////////////////////////////////////////////////////////////////////////////////////////////

    #[test]
    fn test_process_api_result_with_stop() {
        let mut pb = DoublingProgressBar::new("test_progress_bar").unwrap();
        let json_str = r#"{"code": "print('Hello, World!')"}"#.to_string();
        let finish_reason = Some(FinishReason::Stop);
        let cj = ChatterJSON::new();
        let result = cj
            .process_api_result(&mut pb, json_str, finish_reason)
            .unwrap();
        assert_eq!(
            result,
            ProcessingOutcome::Done(json!({"code": "print('Hello, World!')"}))
        );
    }

    #[test]
    fn test_process_api_result_with_length() {
        let mut pb = DoublingProgressBar::new("test_progress_bar").unwrap(); // Pass the required argument to the function.
        let json_str = r#"{"message": "Hello, World!"}"#.to_string();
        let finish_reason = Some(FinishReason::Length);
        let cj = ChatterJSON::new();
        let result = cj
            .process_api_result(&mut pb, json_str, finish_reason)
            .unwrap();
        assert_eq!(result, ProcessingOutcome::Retry);
    }

    #[test]
    fn test_process_api_result_with_unexpected_reason() {
        let mut pb = DoublingProgressBar::new("test_progress_bar").unwrap();
        let json_str = r#"{"message": "Hello, World!"}"#.to_string();
        let finish_reason = None;
        let cj = ChatterJSON::new();
        let result = cj
            .process_api_result(&mut pb, json_str, finish_reason)
            .unwrap();
        assert_eq!(result, ProcessingOutcome::Retry);
    }

    #[test]
    fn test_process_api_result_without_reason() {
        let mut pb = DoublingProgressBar::new("test_progress_bar").unwrap();
        let json_str = r#"{"message": "Hello, World!"}"#.to_string();
        let finish_reason = None;
        let cj = ChatterJSON::new();
        let result = cj
            .process_api_result(&mut pb, json_str, finish_reason)
            .unwrap();
        assert_eq!(result, ProcessingOutcome::Retry);
    }

    ////////////////////////////////////////////////////////////////////////////////////////////////
    // process_chunk() tests
    ////////////////////////////////////////////////////////////////////////////////////////////////

    #[test]
    fn test_process_chunk() {
        let mut pb = DoublingProgressBar::new("test_progress_bar").unwrap();
        let mut chunks = Vec::new();
        let mut consecutive_blanks = 0;
        let mut last_finish_reason = None;

        let response_chunk = create_chunk("Hello", Some(FinishReason::Stop));
        let retry = ChatterJSON::process_chunk(
            &mut pb,
            response_chunk,
            &mut chunks,
            &mut consecutive_blanks,
            &mut last_finish_reason,
        );
        assert!(!retry);
        assert_eq!(chunks, vec!["Hello"]);
    }

    #[test]
    fn test_process_chunk_consecutive_blanks() {
        let mut pb = DoublingProgressBar::new("test_progress_bar").unwrap();
        let mut chunks = Vec::new();
        let mut consecutive_blanks = 0;
        let mut last_finish_reason = None;

        // Test empty chunk.
        let chunk = create_chunk("", None);
        let retry = ChatterJSON::process_chunk(
            &mut pb,
            chunk,
            &mut chunks,
            &mut consecutive_blanks,
            &mut last_finish_reason,
        );
        assert!(!retry);
        assert_eq!(consecutive_blanks, 1);

        let chunk = create_chunk(" ", None);
        let retry = ChatterJSON::process_chunk(
            &mut pb,
            chunk,
            &mut chunks,
            &mut consecutive_blanks,
            &mut last_finish_reason,
        );
        assert!(!retry);
        assert_eq!(consecutive_blanks, 2);

        let chunk = create_chunk(" \n   ", Some(FinishReason::Stop));
        let retry = ChatterJSON::process_chunk(
            &mut pb,
            chunk,
            &mut chunks,
            &mut consecutive_blanks,
            &mut last_finish_reason,
        );
        assert!(!retry);
        assert_eq!(consecutive_blanks, 3);

        // Test consecutive_blanks reset.
        let chunk = create_chunk("a", None);
        let retry = ChatterJSON::process_chunk(
            &mut pb,
            chunk,
            &mut chunks,
            &mut consecutive_blanks,
            &mut last_finish_reason,
        );
        assert!(!retry);
        assert_eq!(consecutive_blanks, 0);

        // Too many consecutive blanks.
        consecutive_blanks = MAX_CONSECUTIVE_BLANKS;
        let chunk = create_chunk(" ", Some(FinishReason::Stop));
        let retry = ChatterJSON::process_chunk(
            &mut pb,
            chunk,
            &mut chunks,
            &mut consecutive_blanks,
            &mut last_finish_reason,
        );
        assert!(retry);
        assert_eq!(consecutive_blanks, MAX_CONSECUTIVE_BLANKS + 1);
    }

    #[test]
    fn test_process_chunk_finish_reason() {
        let mut pb = DoublingProgressBar::new("test_progress_bar").unwrap();
        let mut chunks = Vec::new();
        let mut consecutive_blanks = 0;
        let mut last_finish_reason = None;

        // Test empty chunk.
        let chunk = create_chunk("foo", Some(FinishReason::Stop));
        let retry = ChatterJSON::process_chunk(
            &mut pb,
            chunk,
            &mut chunks,
            &mut consecutive_blanks,
            &mut last_finish_reason,
        );
        assert!(!retry);
        assert_eq!(last_finish_reason, Some(FinishReason::Stop));
    }

    ////////////////////////////////////////////////////////////////////////////////////////////////
    // collect_chunks() tests
    ////////////////////////////////////////////////////////////////////////////////////////////////
    #[tokio::test]
    async fn test_collect_chunks() {
        let msg = create_message("Request: Hello");

        let request = ChatterJSON::create_request(&[msg]).unwrap();

        let response_chunks = vec![create_chunk(
            r#"{"message": "Hello, World!"}"#,
            Some(FinishReason::Stop),
        )];

        let mock = make_mock(response_chunks);
        let chatter = ChatterJSON::with_client(Box::new(mock));
        let mut pb = DoublingProgressBar::new("test_progress_bar").unwrap();
        let result = chatter.collect_chunks(&mut pb, &request).await.unwrap();
        assert_eq!(
            result,
            ProcessingOutcome::ApiSuccess(
                r#"{"message": "Hello, World!"}"#.to_string(),
                Some(FinishReason::Stop)
            )
        );
    }
    #[tokio::test]
    async fn test_collect_chunks_length() {
        let msg = create_message("Request: Hello");

        let request = ChatterJSON::create_request(&[msg]).unwrap();

        let response_chunks = vec![create_chunk(
            r#"{"message": "Hello, World!"}"#,
            Some(FinishReason::Length),
        )];

        let mock = make_mock(response_chunks);
        let chatter = ChatterJSON::with_client(Box::new(mock));
        let mut pb = DoublingProgressBar::new("test_progress_bar").unwrap();
        let result = chatter.collect_chunks(&mut pb, &request).await.unwrap();
        assert_eq!(
            result,
            ProcessingOutcome::ApiSuccess(
                r#"{"message": "Hello, World!"}"#.to_string(),
                Some(FinishReason::Length)
            )
        );
    }
    #[tokio::test]
    async fn test_collect_chunks_too_many_blanks() {
        let msg = create_message("Request: Hello");

        let request = ChatterJSON::create_request(&[msg]).unwrap();

        let response_chunks =
            vec![create_chunk("", Some(FinishReason::Stop)); MAX_CONSECUTIVE_BLANKS + 1];

        let mock = make_mock(response_chunks);
        let chatter = ChatterJSON::with_client(Box::new(mock));
        let mut pb = DoublingProgressBar::new("test_progress_bar").unwrap();
        let result = chatter.collect_chunks(&mut pb, &request).await.unwrap();
        assert_eq!(result, ProcessingOutcome::Retry);
    }

    ////////////////////////////////////////////////////////////////////////////////////////////////
    // chat() tests
    ////////////////////////////////////////////////////////////////////////////////////////////////

    #[tokio::test]
    async fn test_chat_with_successful_response() {
        let request = create_message("Request: Hello, World!");

        let response_chunks = vec![create_chunk(
            r#"{"message": "Hello, World!"}"#,
            Some(FinishReason::Stop),
        )];

        let mock = make_mock(response_chunks);
        let chatter = ChatterJSON::with_client(Box::new(mock));
        let mut pb = DoublingProgressBar::new("test_progress_bar").unwrap();
        let result = chatter.chat(&mut pb, &[request]).await.unwrap();
        assert_eq!(result, json!({"message": "Hello, World!"})); // Adjust this assertion based on your actual expected output
    }

    #[tokio::test]
    async fn test_chat_with_successful_multipart_request() {
        let msgs = [
            create_message("Request: "),
            create_message("Hello, "),
            create_message("World!"),
        ];

        let response_chunks = vec![create_chunk(
            r#"{"message": "Hello, World!"}"#,
            Some(FinishReason::Stop),
        )];
        let mock = make_mock(response_chunks);
        let chatter = ChatterJSON::with_client(Box::new(mock));
        let mut pb = DoublingProgressBar::new("test_progress_bar").unwrap();
        let result = chatter.chat(&mut pb, &msgs).await.unwrap();
        assert_eq!(result, json!({"message": "Hello, World!"})); // Adjust this assertion based on your actual expected output
    }

    #[tokio::test]
    async fn test_chat_with_successful_multipart_response() {
        let request = create_message("Request: Hello, World!");

        let response_chunks = vec![
            create_chunk(r#"{"message""#, None),
            create_chunk(r#": "Hello"#, None),
            create_chunk(r#", World!"}"#, Some(FinishReason::Stop)),
        ];

        let mock = make_mock(response_chunks);
        let chatter = ChatterJSON::with_client(Box::new(mock));
        let mut pb = DoublingProgressBar::new("test_progress_bar").unwrap();
        let result = chatter.chat(&mut pb, &[request]).await.unwrap();
        assert_eq!(result, json!({"message": "Hello, World!"})); // Adjust this assertion based on your actual expected output
    }

    #[tokio::test]
    async fn test_chat_with_max_retries() {
        let request = create_message("Request: Hello, World!");

        let response_chunks = vec![create_chunk("", None); MAX_RETRIES + 1];

        let mock = make_mock(response_chunks);
        let chatter = ChatterJSON::with_client(Box::new(mock));
        let mut pb = DoublingProgressBar::new("test_progress_bar").unwrap();
        let result = chatter.chat(&mut pb, &[request]).await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            format!("too many API retries: {}", MAX_RETRIES)
        );
    }

    ////////////////////////////////////////////////////////////////////////////////////////////////
    // validate_fields() tests
    ////////////////////////////////////////////////////////////////////////////////////////////////

    // Test valid fields
    #[test]
    fn test_validate_fields_valid() {
        let fields = vec!["field1", "field2"];
        let value = json!({
          "field1": "value1",
          "field2": "value2"
        });

        let fields = ChatterJSON::validate_fields(&value, fields).unwrap();
        assert!(fields.is_empty());
    }

    #[test]
    fn test_validate_fields_extra() {
        let fields = vec!["field1", "field2"];
        let value = json!({
          "field1": "value1",
          "field2": "value2",
          "field3": "value3"
        });

        let extra_fields = ChatterJSON::validate_fields(&value, fields).unwrap();
        assert_eq!(extra_fields, vec!["field3"]);
    }

    #[test]
    fn test_validate_fields_missing() {
        let fields = vec!["field1", "field2"];
        let value = json!({
          "field1": "value1"
        });

        match ChatterJSON::validate_fields(&value, fields) {
            Ok(_) => panic!("Expected an error for missing fields, but got Ok"),
            Err(e) => match e.downcast_ref::<AiCriticError>() {
                Some(AiCriticError::MissingJsonFields { fields }) => {
                    let expected_missing: HashSet<_> = ["field2"].iter().cloned().collect();
                    let actual_missing: HashSet<_> = fields.iter().map(|s| s.as_str()).collect();
                    assert_eq!(
                        expected_missing, actual_missing,
                        "Unexpected missing fields"
                    );
                }
                _ => panic!("Expected MissingJsonFields error, got different error"),
            },
        }
    }

    #[test]
    fn test_validate_fields_not_json_object() {
        let value = json!("This is not a JSON object");

        let fields = vec!["field1", "field2"]; // These fields are irrelevant in this case

        match ChatterJSON::validate_fields(&value, fields) {
            Ok(_) => panic!("Expected an error for non-object JSON, but got Ok"),
            Err(e) => match e.downcast_ref::<AiCriticError>() {
                Some(AiCriticError::NotJsonObject) => {}
                _ => panic!("Expected NotJsonObject error, got different error"),
            },
        }
    }
}
