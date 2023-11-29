use crate::errors::{AiCriticError, Result};
use std::io::Write;
use std::process::Command;

const COMPILER_AGENT_NAME: &str = "Compiler";

pub struct CompilerAgent {
    _name: String,
    _print: bool,
}

impl CompilerAgent {
    pub fn new(id: usize) -> Self {
        CompilerAgent {
            _name: format!("{}_{}", COMPILER_AGENT_NAME, id),
            _print: false,
        }
    }

    pub async fn compile(&self, code: &str) -> Result<()> {
        let mut temp_file = tempfile::NamedTempFile::new()?;
        write!(temp_file, "{}", code)?;
        let temp_path = temp_file.path().to_owned();

        let output = Command::new("rustc")
            .arg("--test")
            .arg("--crate-name=aicritic_crate")
            .arg(temp_path)
            .output()?;

        eprintln!("rustc stdout: {}", String::from_utf8_lossy(&output.stdout));
        eprintln!("rustc stderr: {}", String::from_utf8_lossy(&output.stderr));

        match output.status.code() {
            Some(0) => Ok(()),
            Some(code) => Err(AiCriticError::CompilationFailed { code }),
            None => Err(AiCriticError::ProcessTerminated),
        }
    }
}
