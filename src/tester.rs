use crate::errors::AiCriticError;
use color_eyre::eyre::Result;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

const TESTER_AGENT_NAME: &str = "Tester";

pub struct TesterAgent {
    _name: String,
    _print: bool,
}

pub enum TesterResult {
    Success { stdout: String, exec_path: PathBuf },
    Failure { stdout: String, suggestion: String },
}

impl TesterAgent {
    pub fn new(id: usize) -> Self {
        TesterAgent {
            _name: format!("{}_{}", TESTER_AGENT_NAME, id),
            _print: false,
        }
    }

    // Compile the given code and return the path to the executable.
    pub async fn compile(&self, temp_dir_path: &Path, code: &str) -> Result<TesterResult> {
        let rs_file_path = temp_dir_path.join("code.rs");
        let exec_path = temp_dir_path.join("test");

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .open(&rs_file_path)?;
        write!(file, "{}", code)?;

        // Below, the unwrap()s guard against invalid UTF-8, but tempfile::Builder::new() generates
        // valid UTF-8.
        let output = Command::new("rustc")
            .arg("--test")
            .arg("-o")
            .arg(exec_path.to_str().unwrap())
            .arg(rs_file_path.to_str().unwrap())
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        match output.status.code() {
            Some(0) => Ok(TesterResult::Success { stdout, exec_path }),
            Some(_) => Ok(TesterResult::Failure {
                stdout: stderr.to_string(),
                suggestion: format!("Fix the following compilation error: {}", stderr).to_string(),
            }),
            None => Err(AiCriticError::ProcessTerminated.into()),
        }
    }

    // Run the given test executable and return the exit code.
    pub async fn test(&self, exec_path: PathBuf) -> Result<TesterResult> {
        let output = Command::new(exec_path).output()?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();

        match output.status.code() {
            Some(0) => Ok(TesterResult::Success {
                stdout,
                exec_path: "".into(),
            }),
            Some(101) => Ok(TesterResult::Failure {
                stdout: stdout.clone(),
                suggestion: format!("Fix the following test error: {}", stdout).to_string(),
            }),
            Some(code) => {
                println!("Test exited with unexpected code {}", code);
                println!("Stdout: {}", stdout);
                println!("Stderr: {}", String::from_utf8_lossy(&output.stderr));
                Err(AiCriticError::TestingFailed { code }.into())
            }
            None => Err(AiCriticError::ProcessTerminated.into()),
        }
    }

    // Compile the code then run the test executable, returning the stdout and stderr of the
    // outputs.
    pub async fn compile_and_test(&self, code: &str) -> Result<TesterResult> {
        // Create a temporary directory and compile the given code. The directory and its contents
        // will be deleted when the returned future is dropped.
        let temp_dir = TempDir::new()?;
        let temp_dir_path = temp_dir.path();
        let compilation_outcome = self.compile(temp_dir_path, code).await?;
        let exec_path = match compilation_outcome {
            TesterResult::Success { exec_path, .. } => exec_path,
            TesterResult::Failure { .. } => {
                return Ok(compilation_outcome);
            }
        };
        self.test(exec_path).await
    }
}
