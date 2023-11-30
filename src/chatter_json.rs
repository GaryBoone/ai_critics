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
use serde_json::{Map, Value};
use std::collections::HashSet;
use tokio::time::timeout;

const MODEL: &str = "gpt-4-1106-preview";
//const MODEL: &str = "gpt-4"; // Try comparing.
const MAX_TOKENS: u16 = 2048;
const TEMPERATURE: f32 = 0.1;
const MAX_RETRIES: usize = 5;
const TIMEOUT_DURATION: std::time::Duration = std::time::Duration::from_secs(30);
// The OpenAI API has a bug where the model will return a stream of spaces and newlines instead of
// the actual text response. Eventually, this stream will exceed the max_tokens limit and the API
// will return a 'Length' stop reason in the response's ChatChoice. But there's no reason to wait
// for the full max_tokens to be exhausted with empty chunks before noticing the abnormal response.
// Instead, we'll allow only MAX_CONSECUTIVE_BLANKS consecutive empty chunks in the response stream.
const MAX_CONSECUTIVE_BLANKS: usize = 100;

#[derive(PartialEq)]
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
            if Self::check_for_excessive_blanks(consecutive_blanks, content) {
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
            pb.inc();
            match timeout(TIMEOUT_DURATION, stream.next()).await {
                Ok(Some(message)) => {
                    if Self::process_chunk(
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

    // Process the JSON Value returned by the OpenAI API. In some of our System messages, we
    // instruct GPT-4 to generate a code snippet. It should be returned in the Value as a field
    // called 'code' with a String value. However, it is sometimes returned as an Object value
    // instead. This function will handle both cases, returning the value of the 'code' field as a
    // String.
    fn process_code_value(
        map: &Map<String, Value>,
        pb: &mut DoublingProgressBar,
    ) -> Result<ProcessingOutcome> {
        {
            match map.get("code") {
                Some(Value::Object(_)) => {
                    // The code value is an object instead of a String. For example:
                    //   [Object {"code": Object("...")}]
                    // TODO: Remove.
                    pb.clone().println("[OK] Found Object in JSON object");

                    // Recreate the code value as a String.
                    let serialized = map["code"].to_string();
                    let new_value = Value::Object(
                        [("code".to_string(), Value::String(serialized))]
                            .iter()
                            .cloned()
                            .collect(),
                    );
                    Ok(ProcessingOutcome::Done(new_value))
                }
                _ => {
                    // This is expected if the code object isn't nested:
                    //   [Object {"code": String("...")}]
                    Ok(ProcessingOutcome::Done(Value::Object(map.clone())))
                }
            }
        }
    }

    // Process the JSON string returned by the OpenAI API when the STOP finish reason is returned.
    // Return it as a Value for further processing.
    fn process_stop(json_str: String, pb: &mut DoublingProgressBar) -> Result<ProcessingOutcome> {
        let value: Value = serde_json::from_str(&json_str)?;
        match &value {
            // Check for the case where the API returns the code value as an Object instead of a
            // String.
            Value::Object(map) if map.contains_key("code") => Self::process_code_value(map, pb),
            Value::Object(_) => {
                // This is expected if the API isn't trying to return a code object.
                Ok(ProcessingOutcome::Done(value))
            }
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
            Some(FinishReason::Stop) => Self::process_stop(json_str, pb),
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
                    pb.clone()
                        .println("Retrying due to too many empty chunks returned by the API.");
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
