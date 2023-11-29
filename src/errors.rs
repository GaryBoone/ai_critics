use thiserror::Error;

#[derive(Debug, Error)]
pub enum AiCriticError {
    #[error("IO error: {}", source)]
    Io {
        #[from]
        source: std::io::Error,
    },

    #[error("Reqwest error: {}", source)]
    Reqwest {
        #[from]
        source: reqwest::Error,
    },

    #[error("OpenAI error: {}", source)]
    OpenAI {
        #[from]
        source: async_openai::error::OpenAIError,
    },

    #[error("Regex error: {}", source)]
    Regex {
        #[from]
        source: regex::Error,
    },

    #[error("Process terminated by signal")]
    ProcessTerminated,

    #[error("Rust compiler exited with code {}", code)]
    CompilationFailed { code: i32 },

    #[error("No response choice found.")]
    NoResponseChoice,

    #[error("no text field in ChatChoice")]
    NoTextField,

    #[error("the returned JSON is not an object")]
    NotJsonObject,

    #[error("the returned JSON is missing fields `{:?}`", fields)]
    MissingJsonFields { fields: Vec<String> },

    #[error("failed to parse JSON: {}", source)]
    JsonParseError {
        #[from]
        source: serde_json::Error,
    },

    #[error("non-string element found in the array")]
    NonStringElement,

    #[error("provided value is not an array")]
    NotArray,

    #[error("maximum number of retries exceeded: {}", retries)]
    MaxRetriesExceeded { retries: usize },
}

pub type Result<T, E = AiCriticError> = std::result::Result<T, E>;
