use crate::{errors::AiCriticError, DoublingProgressBar};
use async_openai::{
    config::OpenAIConfig,
    types::{
        ChatCompletionRequestMessage, ChatCompletionResponseFormat,
        ChatCompletionResponseFormatType, CreateChatCompletionRequest,
        CreateChatCompletionRequestArgs, FinishReason,
    },
    Client,
};
use color_eyre::eyre::Result;
use futures::StreamExt;
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use tokio::time::timeout; // Add this import statement

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

pub struct ChatterJSON {
    client: Client<OpenAIConfig>,
}

impl ChatterJSON {
    pub fn new() -> Self {
        ChatterJSON {
            client: Client::new(),
        }
    }

    pub fn with_client(client: Client<OpenAIConfig>) -> Self {
        ChatterJSON { client }
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
        response: async_openai::types::CreateChatCompletionStreamResponse,
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
        let mut stream = self.client.chat().create_stream(request.clone()).await?;
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
            Value::Object(map)
                if map.contains_key("correct") && map.contains_key("corrections") =>
            {
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

        for i in 1..=MAX_RETRIES {
            match self.collect_chunks(pb, &request).await {
                Ok(ProcessingOutcome::ApiSuccess(json_str, finish_reason)) => {
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
                Err(e) => return Err(e),
            };
            println!("Retry attempt: {}", i);
        }

        Err(AiCriticError::MaxRetriesExceeded {
            retries: MAX_RETRIES,
        }
        .into())
    }

    // Validate fields from a JSON Value object.
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
    use crate::{errors::AiCriticError, DoublingProgressBar};
    use async_openai::{
        config::OpenAIConfig,
        types::{
            ChatCompletionRequestMessage, ChatCompletionResponseFormat,
            ChatCompletionResponseFormatType, CreateChatCompletionRequest,
            CreateChatCompletionRequestArgs, FinishReason,
        },
        Client,
    };
    use color_eyre::eyre::Result;
    use futures::StreamExt;
    use serde_json::{json, Map, Value};
    use std::collections::HashSet;
    use tokio::time::timeout; // Add this import statement

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
    fn test_process_stop_without_code() {
        let json_str = r#"{"message": "Hello, World!"}"#.to_string();
        let result = ChatterJSON::process_stop(json_str).unwrap();
        assert_eq!(
            result,
            ProcessingOutcome::Done(json!({"message": "Hello, World!"}))
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

    // #[test]
    // fn test_chat_with_successful_response() {
    //     let mut pb = DoublingProgressBar::new("test_progress_bar").unwrap();
    //     let msgs = vec![ChatCompletionRequestMessage {
    //         role: "system".to_string(),
    //         content: "Hello, World!".to_string(),
    //     }];
    //     let result = chat(&pb, &msgs).unwrap();
    //     assert_eq!(result, json!({"message": "Hello, World!"}));
    // }

    // #[test]
    // fn test_chat_with_retry_response() {
    //     let mut pb = DoublingProgressBar::new("test_progress_bar");
    //     let msgs = vec![ChatCompletionRequestMessage {
    //         role: "system".to_string(),
    //         content: "Hello, World!".to_string(),
    //     }];
    //     let result = chat(&pb, &msgs);
    //     assert!(result.is_err());
    //     assert_eq!(
    //         result.unwrap_err().to_string(),
    //         "maximum retries exceeded: 3"
    //     );
    // }

    // #[test]
    // fn test_validate_fields_with_valid_fields() {
    //     let value = json!({"name": "John", "age": 30});
    //     let fields = vec!["name", "age"];
    //     let result = ChatterJSON::validate_fields(&value, fields).unwrap();
    //     assert_eq!(result, vec![]);
    // }

    // #[test]
    // fn test_validate_fields_with_missing_fields() {
    //     let value = json!({"name": "John"});
    //     let fields = vec!["name", "age"];
    //     let result = ChatterJSON::validate_fields(&value, fields);
    //     assert!(result.is_err());
    //     assert_eq!(
    //         result.unwrap_err().to_string(),
    //         "missing JSON fields: [\"age\"]"
    //     );
    // }

    // #[test]
    // fn test_validate_fields_with_non_object_value() {
    //     let value = json!(42);
    //     let fields = vec!["name", "age"];
    //     let result = ChatterJSON::validate_fields(&value, fields);
    //     assert!(result.is_err());
    //     assert_eq!(result.unwrap_err().to_string(), "not a JSON object");
    // }
}
