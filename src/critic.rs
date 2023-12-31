use crate::chatter_json::ChatterJSON;
use crate::DoublingProgressBar;
use async_openai::types::{
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
    ChatCompletionRequestUserMessageArgs,
};
use color_eyre::eyre::Result;
use serde::Deserialize;
use serde::Deserializer;
use serde_json::Value;

// There are 3 types critic agents that vary based the type of critique they give. Roughly these are:
//
// 1. Design: Does the code use an algorithm that will correctly solve the given coding problem?
// 2. Correctness: Does the code solve the given coding problem?
// 3. Syntax: Is the code syntactically correct?
//
// As an alternative to these specialized agents, general agent combines the above into
// a single prompt.

// All critic agents share the base prompt.
const BASE_PROMPT: &str = "
    Evaluate this code based on the criteria below. Make no comments or explanations.
    Return JSON with two fields:
    1. a field named `lgtm` with value `true` if the code is correct, else `false`.
    2. a field `corrections` containing list of the errors, if any, else `None`.
";

const GENERAL_SYSTEM_PROMPT: &str = "
    Review the code for design, correctness, and syntax issues.
";

const DESIGN_SYSTEM_PROMPT: &str = "
    Evaluation Criteria: Evaluate the _design_ of the solution, considering the following questions: 
    1. Is this the right the design to solve the problem?
    2. Does the method chosen meet the constraints of the problem?
    3. Does it use a the correct algorithms and data structures to solve the problem?
";

const CORRECTNESS_SYSTEM_PROMPT: &str = "
    Evaluation Criteria: Evaluate the _correctness_ of the solution, considering the following 
    questions: 
    1. Does the code correctly implement the intended solution approach?
    2. Does the code generate the expected output?
    3. Does the output meet the original problem constraints?
    4. Are there enough tests to demonstrate the correctness of the solution?
    5. Do the tests correctly capture situations that validate or invalidate the solution?
";

const SYNTAX_SYSTEM_PROMPT: &str = "
    Evaluation Criteria: Evaluate the _syntax_ of the solution, considering the following questions: 
    1. Are there any syntactic errors?
    2. Will the code and tests compile and run?
    3. Are there any language errors such as borrowing violations or lifetime problems?
    4. Are there any cleanups needed such as unused variables or imports?
";

pub enum CriticType {
    General,
    Design,
    Correctness,
    Syntax,
}

pub struct CriticAgent {
    pub name: String,
    pub critic_type: CriticType,
    system_msg: ChatCompletionRequestMessage,
    chatter: ChatterJSON,
}

#[derive(Deserialize, Debug, Eq, PartialEq, Hash)]
pub struct Correction {
    #[serde(skip_deserializing)]
    pub name: String,
    #[serde(default)]
    pub lgtm: bool,
    #[serde(deserialize_with = "deserialize_corrections")]
    pub corrections: Vec<String>,
}

// The `#[serde(default)]` annotation doesn't, so we need to do this manually.
fn deserialize_corrections<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let v = Value::deserialize(deserializer)?;
    match v {
        Value::Null => Ok(Vec::new()), // Handle null as empty Vec.
        Value::Array(arr) => arr
            .into_iter()
            .map(|val| {
                val.as_str().map_or_else(
                    || Err(serde::de::Error::custom("Expected string")),
                    |s| Ok(s.to_string()),
                )
            })
            .collect(),
        _ => Err(serde::de::Error::custom("Expected array or null")),
    }
}

impl CriticAgent {
    pub fn new(critic_type: CriticType, id: usize) -> Result<Self> {
        let name = match critic_type {
            CriticType::General => format!("General Critic {}", id),
            CriticType::Design => format!("Design Critic {}", id),
            CriticType::Correctness => format!("Correctness Critic {}", id),
            CriticType::Syntax => format!("Syntax Critic {}", id),
        };

        let critic_prompt = match critic_type {
            CriticType::General => format!("{}\n{}", BASE_PROMPT, GENERAL_SYSTEM_PROMPT),
            CriticType::Design => format!("{}\n{}", BASE_PROMPT, DESIGN_SYSTEM_PROMPT),
            CriticType::Correctness => format!("{}\n{}", BASE_PROMPT, CORRECTNESS_SYSTEM_PROMPT),
            CriticType::Syntax => format!("{}\n{}", BASE_PROMPT, SYNTAX_SYSTEM_PROMPT),
        };

        let system_msg = ChatCompletionRequestSystemMessageArgs::default()
            .content(critic_prompt)
            .build()?
            .into();

        let chatter = ChatterJSON::new();

        Ok(CriticAgent {
            name,
            critic_type,
            system_msg,
            chatter,
        })
    }

    pub async fn chat(&self, pb: &mut DoublingProgressBar, msg: &str) -> Result<Correction> {
        let user_msg = ChatCompletionRequestUserMessageArgs::default()
            .content(msg)
            .build()?
            .into();

        let json = self
            .chatter
            .chat(pb, &[self.system_msg.clone(), user_msg])
            .await?;

        // Check the fields. Should only be two: `lgtm` and `corrections`.
        let extra_keys = ChatterJSON::validate_fields(&json, vec!["lgtm", "corrections"])?;
        if !extra_keys.is_empty() {
            println!(
                "{}: Warning: Extra keys in critic response: {:?}",
                self.name, extra_keys
            );
        }
        // Ok(serde_json::from_value(json)?) // Convert to AiCriticError.
        let mut correction: Correction = serde_json::from_value(json)?;
        correction.name = self.name.clone();
        Ok(correction)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_corrections() {
        // Test case 1: Empty array
        let input1 = Value::Array(vec![]);
        let result1 = deserialize_corrections(&input1).unwrap();
        assert!(result1.is_empty());

        // Test case 2: Array with strings
        let input2 = Value::Array(vec![
            Value::String("error1".to_string()),
            Value::String("error2".to_string()),
        ]);
        let result2 = deserialize_corrections(&input2).unwrap();
        assert_eq!(result2, vec!["error1", "error2"]);

        // Test case 3: Null value
        let input3 = Value::Null;
        let result3 = deserialize_corrections(&input3).unwrap();
        assert!(result3.is_empty());

        // Test case 4: Invalid input (not an array or null)
        let input4 = Value::Bool(true);
        let result4 = deserialize_corrections(&input4);
        assert!(result4.is_err());
    }
}
