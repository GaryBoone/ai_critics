use std::fmt;

use crate::{chatter_json::ChatterJSON, DoublingProgressBar};
use async_openai::types::{
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
    ChatCompletionRequestUserMessageArgs,
};
use color_eyre::eyre::Result;
use serde::Deserialize;

const CODER_NAME: &str = "Coder";
const SYSTEM_PROMPT: &str = "
    Write the requested program in Rust. Include complete unit tests. Return the code as JSON in a 
    string field called `code`.
    Any clarifying explanations should be included in the code as // comments. 
    Be sure that the tests demonstrate that the code solves the requested problem. 
    Any `assert` used should include a custom message with a unique 6-digit hex number labelled 
    `assert_id` that uniquely identifies the assert line so that line numbers are not required.
";

pub struct CoderAgent {
    pub name: String,
    system_msg: ChatCompletionRequestMessage,
    chatter: ChatterJSON,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Code {
    pub code: String,
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Code:\n{}", self.code)
    }
}

impl CoderAgent {
    pub fn new(id: usize) -> Result<Self> {
        let system_msg = ChatCompletionRequestSystemMessageArgs::default()
            .content(SYSTEM_PROMPT)
            .build()?
            .into();

        Ok(CoderAgent {
            name: format!("{}_{}", CODER_NAME, id),
            system_msg,
            chatter: ChatterJSON::new(),
        })
    }

    pub async fn chat(&self, pb: &mut DoublingProgressBar, msg: &str) -> Result<Code> {
        let user_msg = ChatCompletionRequestUserMessageArgs::default()
            .content(msg)
            .build()?
            .into();

        let json = self
            .chatter
            .chat(pb, &[self.system_msg.clone(), user_msg])
            .await?;

        // Check the fields. Should only be one: `code`.
        let extra_keys = ChatterJSON::validate_fields(&json, vec!["code"])?;
        if !extra_keys.is_empty() {
            println!(
                "{}: Warning: Extra keys in Coder response: {:?}",
                self.name, extra_keys
            );
        }
        Ok(serde_json::from_value(json)?)
    }
}
