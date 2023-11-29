use crate::errors::{AiCriticError, Result};
use async_openai::{
    config::OpenAIConfig,
    types::{
        ChatChoice, ChatCompletionRequestMessage, ChatCompletionResponseFormat,
        ChatCompletionResponseFormatType, CreateChatCompletionRequestArgs, FinishReason,
    },
    Client,
};
use serde_json::Value;
use std::collections::HashSet;

// const MODEL: &str = "gpt-4";
const MODEL: &str = "gpt-4-1106-preview";
const MAX_TOKENS: u16 = 2048;
const TEMPERATURE: f32 = 0.1;
const MAX_RETRIES: usize = 5;

pub struct ChatterJSON {
    client: Client<OpenAIConfig>,
}

#[derive(Debug, serde::Deserialize, Clone)]
pub struct Code {
    pub code: String,
}

impl ChatterJSON {
    pub fn new() -> Self {
        ChatterJSON {
            client: Client::new(),
        }
    }

    // Parse a ChatChoice object containing JSON into a Value object.
    fn parse_json(&self, choice: &ChatChoice) -> Result<Value> {
        let json_str = choice
            .message
            .content
            .as_ref()
            .ok_or(AiCriticError::NoTextField)?;

        // Deserialize the JSON string into a serde_json::Value
        serde_json::from_str(json_str).map_err(|e| AiCriticError::JsonParseError { source: e })
    }

    // Validate fields from a JSON Value object.
    pub fn validate_fields(&self, value: &Value, fields: Vec<&str>) -> Result<Vec<String>> {
        // Iterate over fields if it's an object.
        match value.as_object() {
            Some(obj) => {
                let obj_keys: HashSet<_> = obj.keys().cloned().collect();
                let fields_set: HashSet<_> = fields.iter().map(|&k| k.to_string()).collect();

                let missing_keys: Vec<_> = fields_set.difference(&obj_keys).cloned().collect();
                if !missing_keys.is_empty() {
                    return Err(AiCriticError::MissingJsonFields {
                        fields: missing_keys,
                    });
                }
                Ok(obj_keys.difference(&fields_set).cloned().collect())
            }
            None => Err(AiCriticError::NotJsonObject),
        }
    }

    pub fn get_field_string(value: &Value) -> Result<String> {
        value
            .as_str()
            .map(|s| s.replace("\\n", "\n"))
            .ok_or(AiCriticError::NotJsonObject)
    }

    // Get a field from a JSON Value object as a string. Assume lines are returned in the given
    // field as an array of strings.
    pub fn get_field_array(value: &Value) -> Result<String> {
        if let Some(array) = value.as_array() {
            // Collect the strings from the array
            let strings: Result<Vec<String>> = array
                .iter()
                .map(|item| {
                    // Ensure each item is a string
                    item.as_str()
                        .ok_or(AiCriticError::NonStringElement)
                        .map(|s| s.to_string())
                })
                .collect();

            // Join the strings with '\n' if there were no errors
            strings.map(|s| s.join("\n"))
        } else {
            Err(AiCriticError::NotArray)
        }
    }

    pub fn get_field(value: &Value, field: &str) -> Result<String> {
        let val = value
            .get(field)
            .ok_or_else(|| AiCriticError::MissingJsonFields {
                fields: vec![field.to_string()],
            })?;
        match val {
            Value::String(_) => Self::get_field_string(val),
            Value::Array(_) => Self::get_field_array(val),
            Value::Number(_) => {
                println!("{} was a number.", field);
                Ok("".to_string())
            }
            Value::Object(_) => {
                println!("{} was an object.", field);
                Ok("".to_string())
            }
            Value::Bool(_) => {
                println!("{} was a boolean.", field);
                Ok("".to_string())
            }
            Value::Null => {
                println!("{} was null", field);
                Ok("".to_string())
            }
        }
    }
    // pub fn get_field(value: &Value, field: &str) -> Result<Value> {
    //     value
    //         .get(field)
    //         .ok_or_else(|| AiCriticError::MissingJsonFields {
    //             fields: vec![field.to_string()],
    //         })
    //         .map(|v| v.clone())
    // }

    pub async fn chat(&self, msgs: &[ChatCompletionRequestMessage]) -> Result<Value> {
        let request = CreateChatCompletionRequestArgs::default()
            .model(MODEL)
            .max_tokens(MAX_TOKENS)
            .temperature(TEMPERATURE)
            .response_format(ChatCompletionResponseFormat {
                r#type: ChatCompletionResponseFormatType::JsonObject,
            })
            .n(1)
            .messages(msgs)
            .build()?;

        for i in 0..MAX_RETRIES {
            // Call OpenAI API. Note that the response_format JSON doesn't work with the streaming API.
            let response = match self.client.chat().create(request.clone()).await {
                Ok(response) => response,
                Err(err) => return Err(err)?,
            };

            // We expect only one ChatChoice in the response.
            assert_eq!(response.choices.len(), 1);
            let resp = response
                .choices
                .first()
                .ok_or(AiCriticError::NoResponseChoice)?;

            if let Some(reason) = resp.finish_reason {
                if reason == FinishReason::Stop {
                    return self.parse_json(resp);
                }
                println!("Finish reason for attempt {}: {:?}. Retrying.", i, reason);
            }
        }
        Err(AiCriticError::MaxRetriesExceeded {
            retries: MAX_RETRIES,
        })
    }
}
