use crate::{chatter_json::ChatterJSON, coder::Code, DoublingProgressBar};
use async_openai::types::{
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
    ChatCompletionRequestUserMessageArgs,
};
use color_eyre::eyre::Result;

const FIXER_NAME: &str = "Fixer";
const SYSTEM_PROMPT: &str =
    "You will be given a coding goal, then an example program to attempts to solve it, then some 
     suggested corrections. Each of these is separated by a line of `------`. Correct the 
     program using all of the suggestions. Use the following steps:
     1. Combine similarly worded, but duplicate suggestions.
     2. Decide if an alternative implementation or data structure is needed to implement the 
        suggestions.
     3. Follow the implications of the suggestions to their conclusions, such as removing `use` 
        statements if you remove the items they import.
     4. Choose the solution approach that implements all of the suggestions.
     5. Write the code that implements the new solution.
     6. Review and modify the code for solution correctness.
     7. Review and modify the code for syntax errors.
     8. Review and modify the code to ensure that all of the suggestions are implemented.
     9. Ensure that the new code includes tests demonstrating that the original coding goal is
        solved. 
     10. Return the corrected code as JSON in a field called `code`.";

pub struct FixerAgent {
    pub name: String,
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

    pub async fn chat(
        &self,
        pb: &mut DoublingProgressBar,
        goal: &str,
        code: &str,
        suggestions: &[String],
    ) -> Result<Code> {
        let msg = format!(
            "{}\n\n------\n\n{}\n\n------\n\n{}",
            goal,
            code,
            suggestions.join("\n\n------\n\n")
        );

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
                "{}: Warning: Extra keys in fixer response: {:?}",
                self.name, extra_keys
            );
        }
        // TODO: Remove.
        match serde_json::from_value::<Code>(json.clone()) {
            Ok(result) => Ok(result),
            Err(e) => {
                eprintln!("Error deserializing JSON: {}", e);
                if let serde_json::error::Category::Data = e.classify() {
                    eprintln!(
                        "Data error: The structure of the JSON does not match the expected format. The icky JSON is:\n\n{:?}\n\n", json.clone()
                    );
                }

                Err(e.into())
            }
        }
        // Ok(serde_json::from_value(json)?)
    }
}
