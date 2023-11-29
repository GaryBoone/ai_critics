use crate::chatter_json::ChatterJSON;
use crate::errors::Result;
use async_openai::types::{
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
    ChatCompletionRequestUserMessageArgs,
};

const CODER_NAME: &str = "Coder";
const SYSTEM_PROMPT: &str =
    "Write the requested program. Add no explanations. Just return the code with complete tests. \
     The code will piped directly to the rustc compiler so should be formatted to compile. Do not \
     offset the code with ticks or triple ticks. Output plain text. Any clarifying explanations \
     should be included in the code as // comments. Be sure that the code includes tests that \
     demonstrates that the code solves the requested problem. Return the response as JSON.";

pub struct CoderAgent {
    _name: String,
    system_msg: ChatCompletionRequestMessage,
    chatter: ChatterJSON,
}

impl CoderAgent {
    pub fn new(id: usize) -> Result<Self> {
        let system_msg = ChatCompletionRequestSystemMessageArgs::default()
            .content(SYSTEM_PROMPT)
            .build()?
            .into();

        Ok(CoderAgent {
            _name: format!("{}_{}", CODER_NAME, id),
            system_msg,
            chatter: ChatterJSON::new(),
        })
    }

    pub async fn chat(&self, msg: &str) -> Result<String> {
        let user_msg = ChatCompletionRequestUserMessageArgs::default()
            .content(msg)
            .build()?
            .into();

        let json = self
            .chatter
            .chat(&[self.system_msg.clone(), user_msg])
            .await?;

        // Check the fields. Should only be one: `code`.
        let extra_keys = self.chatter.validate_fields(&json, vec!["code"])?;
        if !extra_keys.is_empty() {
            println!("Extra keys in Coder response: {:?}", extra_keys);
        }

        ChatterJSON::get_field(&json, "code")
    }
}
