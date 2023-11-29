use crate::chatter_json::ChatterJSON;
use crate::errors::Result;
use async_openai::types::{
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
    ChatCompletionRequestUserMessageArgs,
};
use serde::Deserialize;

const CODER_NAME: &str = "Critic";
const SYSTEM_PROMPT: &str = "Evaluate the correctness of the following code. Make no comments or \
                             explanations. Return JSON with two fields: 1) a field named `correct` \
                             with value `true` if the code is correct, else false; and 2) a field \
                             `corrections` containing list of the errors, if any. else `None`.";

pub struct CriticAgent {
    name: String,
    system_msg: ChatCompletionRequestMessage,
    chatter: ChatterJSON,
}

#[derive(Deserialize, Debug)]
pub struct Correction {
    pub correct: bool,
    pub corrections: Vec<String>,
}

impl CriticAgent {
    pub fn new(id: usize) -> Result<Self> {
        let system_msg = ChatCompletionRequestSystemMessageArgs::default()
            .content(SYSTEM_PROMPT)
            .build()?
            .into();

        Ok(CriticAgent {
            name: format!("{}_{}", CODER_NAME, id),
            system_msg,
            chatter: ChatterJSON::new(),
        })
    }

    pub async fn chat(&self, msg: &str) -> Result<Correction> {
        let user_msg = ChatCompletionRequestUserMessageArgs::default()
            .content(msg)
            .build()?
            .into();

        let json = self
            .chatter
            .chat(&[self.system_msg.clone(), user_msg])
            .await?;

        // Check the fields. Should only be two: `correct` and `corrections`.
        let extra_keys = self
            .chatter
            .validate_fields(&json, vec!["correct", "corrections"])?;
        if !extra_keys.is_empty() {
            println!("Extra keys in response for {}: {:?}", self.name, extra_keys);
        }
        Ok(serde_json::from_value(json)?)
    }
}
