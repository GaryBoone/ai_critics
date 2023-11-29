use crate::chatter_json::ChatterJSON;
use crate::errors::Result;
use async_openai::types::{
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
    ChatCompletionRequestUserMessageArgs,
};

const FIXER_NAME: &str = "Fixer";
const SYSTEM_PROMPT: &str =
    "You will be given a coding goal, then an example program to attempts to solve it, then one or \
     more suggested corrections. Each of these is separated by a line of `------`. For each \
     suggested correction, first decide if it is a legitimate criticism. For the legitimate \
     ones, correct the program according to the suggestion. Add no explanations. Ensure that it \
     includes tests demonstrating that the original coding goal is solved. Return the corrected \
     code as JSON. ";

pub struct FixerAgent {
    name: String,
    system_msg: ChatCompletionRequestMessage,
    chatter: ChatterJSON,
}

impl FixerAgent {
    pub fn new(id: usize) -> Result<Self> {
        let system_msg = ChatCompletionRequestSystemMessageArgs::default()
            .content(SYSTEM_PROMPT)
            .build()?
            .into();

        Ok(FixerAgent {
            name: format!("{}_{}", FIXER_NAME, id),
            system_msg,
            chatter: ChatterJSON::new(),
        })
    }

    pub async fn chat(&self, goal: &str, code: &str, criticisms: &[String]) -> Result<String> {
        let msg = format!(
            "{}\n\n------\n\n{}\n\n------\n\n{}",
            goal,
            code,
            criticisms.join("\n\n------\n\n")
        );

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
            println!(
                "{}: Extra keys in Coder response: {:?}",
                self.name, extra_keys
            );
        }

        ChatterJSON::get_field(&json, "code")
    }
}
