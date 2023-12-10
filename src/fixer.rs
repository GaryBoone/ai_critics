use crate::{chatter_json::ChatterJSON, coder::Code, DoublingProgressBar};
use async_openai::types::{
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
    ChatCompletionRequestUserMessageArgs,
};
use color_eyre::eyre::Result;

const FIXER_NAME: &str = "Fixer";
const SYSTEM_PROMPT: &str = "
    Correct the code, returning the fixed code as JSON in a string field called `code`.";

const CODE_REVIEW_PROMPT: &str = "
    Specifically address these code review issues:
";

const COMPILE_FIX_PROMPT: &str = "
    Fix the code so that it compiles.
    Correct the compilation errors without changing the code's functionality.
    The code failed to compile with the following errors:
";

const TEST_FIX_PROMPT: &str = "
    The code failed its unit tests as shown below. Fix the code so that it passes all tests.
    1. Match the given `assert_id` value to the assert() in the code to find the assertion that 
       failed.
    2. Is the test correct? If not, write the correct test.
    3. Is the assertion correct? If not, write the correct assertion.
    4. Only if the test and assertion are correct, correct the non-test code.
    This is the output of the failed test:
";

pub enum ReviewType {
    CodeReview,
    CompilerFix,
    TestFix,
}

pub struct ReviewNeeded {
    pub review_type: ReviewType,
    pub comments: Vec<String>,
}

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
        code: &str,
        review: ReviewNeeded,
    ) -> Result<Code> {
        let review_prompt = match review.review_type {
            ReviewType::CodeReview => CODE_REVIEW_PROMPT,
            ReviewType::CompilerFix => COMPILE_FIX_PROMPT,
            ReviewType::TestFix => TEST_FIX_PROMPT,
        };
        let msg = format!(
            "{}\n\n{}\n\n{}",
            review_prompt,
            review
                .comments
                .iter()
                .map(|comment| format!("• {}", comment))
                .collect::<Vec<_>>()
                .join("\n"),
            code,
        );

        log::info!(
            "Review request for {} is {} characters.",
            self.name,
            msg.len(),
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
        Ok(serde_json::from_value(json)?)
    }
}
