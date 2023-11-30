use serde_json::Value;
use std::error::Error;
use std::fmt;
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

    #[error("the process was terminated by signal")]
    ProcessTerminated,

    #[error("the test exited with code {}", code)]
    TestingFailed { code: i32 },

    #[error("the returned JSON is not an object")]
    NotJsonObject,

    #[error("unexpected JSON structure in json:\n{}", json)]
    UnexpectedJsonStructure { json: Value },

    #[error("the returned JSON is missing fields `{:?}`", fields)]
    MissingJsonFields { fields: Vec<String> },

    #[error("failed to parse JSON: {}", source)]
    JsonParseError {
        #[from]
        source: serde_json::Error,
    },

    #[error("too many retries: {}", retries)]
    MaxRetriesExceeded { retries: usize },
}

// Here's how to define a Result<> type for AiCriticError:
// pub type Result<T, E = AiCriticError> = std::result::Result<T, E>;
// But we'll use the Result type from eyre to ensure that the backtrace Reports are propagated from
// the error site.

// AiCriticReport is a newtype wrapper around AiCriticError.
// This wrapper is necessary because `color_eyre::Report` has a blanket implementation for
// converting any error that implements `std::error::Error`. By wrapping AiCriticError in a newtype,
// we can provide custom behavior for how AiCriticError is converted and represented in the context
// of `color_eyre::Report`.
#[derive(Debug)]
pub struct AiCriticReport(AiCriticError);

// Implement `fmt::Display` for AiCriticReport.
// Required by `color_eyre::Report`.
impl fmt::Display for AiCriticReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// Implement `std::error::Error` for AiCriticReport.
// `std::error::Error` for AiCriticReport is required for `color_eyre::Report`'s blanket
// implementation to work with AiCriticError. By implementing `Error` for AiCriticReport, we ensure
// that AiCriticError can be wrapped in a `color_eyre::Report` through the AiCriticReport newtype.
impl Error for AiCriticReport {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.0)
    }
}

// Implement `From<AiCriticError> for AiCriticReport`.
// This implementation allows for the automatic conversion of AiCriticError to AiCriticReport using
// `.into()`. It's used when you need to convert an AiCriticError into a `color_eyre::Report`
// indirectly. For example, when returning an error from a function where the return type is
// `Result<_, color_eyre::Report>`, and the error being returned is an AiCriticError, use
// `AiCriticError.into()`. This conversion is automatic because of the `From` trait implementation.
impl From<AiCriticError> for AiCriticReport {
    fn from(err: AiCriticError) -> Self {
        AiCriticReport(err)
    }
}
