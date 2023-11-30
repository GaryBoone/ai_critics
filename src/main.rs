use coder::{Code, CoderAgent};
use color_eyre::Result;
use critic::{Correction, CriticAgent};
use errors::AiCriticError;
use fixer::FixerAgent;
use futures::future::join_all;
use indicatif::MultiProgress;
use indoc::indoc;
use progress_bar::DoublingProgressBar;
use std::collections::HashSet;
use std::env;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::exit;
use tester::{TesterAgent, TesterResult};
use tokio::task::JoinHandle;

use crate::critic::CriticType;

mod backtraces;
mod chatter_json;
mod coder;
mod critic;
mod errors;
mod fixer;
mod progress_bar;
mod tester;

// The default problem file if none is specified.
const DEFAULT_PROBLEM_FILE: &str = "problems/coding_problem1.txt";
// NUM_CRITICS is the number of critics that will be used.
const NUM_DESIGN_CRITICS: usize = 5;
const NUM_CORRECTNESS_CRITICS: usize = 5;
const NUM_SYNTAX_CRITICS: usize = 5;
// MAX_PROPOSALS is the maximum number of proposals that the critics will generate.
const MAX_PROPOSALS: usize = 20;

fn setup() -> Result<()> {
    if env::var("OPENAI_API_KEY").is_err() {
        println!("Please set the OPENAI_API_KEY environment variable.");
        exit(1);
    }

    backtraces::setup_color_eyre()
}

// Read the file with the given filename in the project root, ignoring lines starting with '#'.
fn read_file(filename: &str) -> Result<String> {
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let full_path = project_root.join(filename);
    println!("Reading file '{}'", full_path.display());

    let file = match File::open(&full_path) {
        Ok(file) => file,
        Err(e) => {
            eprintln!("Error opening file '{}': {}", full_path.display(), e);
            return Err(e.into());
        }
    };
    let reader = BufReader::new(file);

    let mut contents = String::new();
    for line in reader.lines() {
        let line = line?;
        if !line.starts_with('#') {
            contents.push_str(&line);
            contents.push('\n'); // Preserve line breaks.
        }
    }
    Ok(contents)
}

fn read_coding_problem(filename: &str) -> Result<String> {
    let goal = read_file(filename)?;
    println!("The coding problem is:\n\n{}\n", goal);
    Ok(goal)
}

// Have the AI Coder write a solution to the given coding problem.
async fn ai_write_code(goal: &str) -> Result<Code> {
    println!("\n==> Coder writing solution...");
    let coder1 = CoderAgent::new(1)?;
    let code = {
        let mut pb = DoublingProgressBar::new(&coder1.name)?;
        coder1.chat(&mut pb, goal).await?
    };
    Ok(code)
}

fn spawn_critics(
    critics: Vec<CriticAgent>,
    problem: &str,
    code: &Code,
) -> Result<(Vec<JoinHandle<Result<Correction>>>, MultiProgress)> {
    let mut tasks = vec![];
    let multi_progress = MultiProgress::new();
    let mut bars = vec![];
    let msg = format!("{}\n\n------\n\n{}", problem, code.code);
    for c in critics {
        let mut pb = DoublingProgressBar::new_multi(&multi_progress, &c.name)?;
        bars.push(pb.clone());
        let msg = msg.clone();
        tasks.push(tokio::task::spawn(
            async move { c.chat(&mut pb, &msg).await },
        ));
    }
    Ok((tasks, multi_progress))
}

fn collect_suggestions(
    results: Vec<Result<Result<Correction>, tokio::task::JoinError>>,
) -> Result<Vec<Correction>> {
    let mut corrections = Vec::new();
    for result in results {
        match result {
            Ok(ok_result) => match ok_result {
                Ok(correction) => corrections.push(correction),
                Err(e) => return Err(e), // Handle error in `c.chat()`
            },
            Err(e) => return Err(e.into()), // JoinError is unlikely.
        }
    }
    Ok(corrections)
}

fn print_corrections(corrections: &[Correction]) {
    println!("Critic results:");
    for c in corrections.iter() {
        println!("  {}:", c.name);
        println!("    Correct? {}", c.correct);
        if !c.correct {
            for s in c.corrections.iter() {
                println!("    • {}", s);
            }
        }
    }
}

// Have the AI Critics review the code. Return a list of their suggestions.
async fn ai_review_code(proposal_count: usize, problem: &str, code: &Code) -> Result<Vec<String>> {
    let critics = create_critics()?;

    println!("Proposed code #{}: -----------\n{}", proposal_count, &code);
    println!("------------------------------\n");
    println!("\n==> Critics reviewing...");

    // Spawn the critic tasks.
    let (tasks, multi_progress) = spawn_critics(critics, problem, code)?;

    // Wait for the critic tasks to complete.
    let results = join_all(tasks).await;
    multi_progress.clear()?;

    // Collect the results.
    let corrections = collect_suggestions(results)?;

    print_corrections(&corrections);

    // For the Corrections that are incorrect, collect the suggestions into a HashSet, deduping
    // them. Note that suggestions from GPT are often the same idea but using different words, so
    // this deduplication only removes the less frequent literal duplicates. Return them as a
    // Vec<String>.
    let suggestions: Vec<String> = corrections
        .iter()
        .filter(|cs| !cs.correct)
        .flat_map(|cs| &cs.corrections)
        .cloned()
        .collect::<HashSet<String>>()
        .into_iter()
        .collect();

    Ok(suggestions)
}

fn create_critics() -> Result<Vec<CriticAgent>> {
    let mut critics = vec![];
    for i in 1..=NUM_DESIGN_CRITICS {
        critics.push(CriticAgent::new(CriticType::Design, i)?);
    }
    for i in 1..=NUM_CORRECTNESS_CRITICS {
        critics.push(CriticAgent::new(CriticType::Correctness, i)?);
    }
    for i in 1..=NUM_SYNTAX_CRITICS {
        critics.push(CriticAgent::new(CriticType::Syntax, i)?);
    }
    Ok(critics)
}

fn report_test_success(proposal_count: usize, code: &str, test_output: &str) {
    println!(
        indoc! {"
            Success after {} proposals.
            Final code:
            --------------------------------------------------------------------------------
            {}
            --------------------------------------------------------------------------------
            Test output:
            --------------------------------------------------------------------------------
            {}
            --------------------------------------------------------------------------------
        "},
        proposal_count, &code, test_output
    );
}

fn report_tester_failure(stderr: &str) {
    println!(
        indoc! {"
            Compiling/Testing failure:
            --------------------------------------------------------------------------------
            {}
            --------------------------------------------------------------------------------
        "},
        stderr
    );
}

// Have the AI Fixer agent correct the code given the critics' suggestions.
async fn ai_fix_code(goal: &str, code: &Code, suggestions: &[String]) -> Result<Code> {
    println!("\n==> Fixer correcting...");

    let fixer1 = FixerAgent::new(1)?;
    let mut pb = DoublingProgressBar::new(&fixer1.name)?;
    let code = fixer1.chat(&mut pb, goal, &code.code, suggestions).await?;
    println!("Fixer corrects to:\n{}", code);
    Ok(code)
}

// Compile and test the code. Return an optional suggestion if the code fails to compile of fails
// the test.
async fn compile_and_test(proposal_count: usize, code: &Code) -> Result<Option<String>> {
    println!("All of the critics agree that code is correct.");
    println!("\n==> Tester compiling and testing...");
    let tester = TesterAgent::new(1);

    match tester.compile_and_test(&code.code).await? {
        TesterResult::Success { stdout, .. } => {
            report_test_success(proposal_count, &code.code, &stdout);
            Ok(None)
        }
        TesterResult::Failure { stdout, suggestion } => {
            report_tester_failure(&stdout);
            // Continue, seeing if the AI can fix the code/tests so it passes.
            Ok(Some(suggestion))
        }
    }
}

// Main run loop: Read the problem and run the AI agents to solve it. Use a Coder agent to produce
// an initial solution, then in a loop run the AI critics to review the code, the fixer agent to
// correct it, and the tester agent to test it. Repeat until it works or MAX_PROPOSALS is reached.
async fn run() -> Result<()> {
    setup()?;

    let filename = env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_PROBLEM_FILE.to_string());

    let goal = read_coding_problem(&filename)?;

    let mut code = ai_write_code(&goal).await?;

    for proposal_count in 1..=MAX_PROPOSALS {
        let suggestions = ai_review_code(proposal_count, &goal, &code).await?;

        if !suggestions.is_empty() {
            code = ai_fix_code(&goal, &code, &suggestions).await?;
        }
        match compile_and_test(proposal_count, &code).await? {
            Some(suggestion) => {
                code = ai_fix_code(&goal, &code, &[suggestion]).await?;
            }
            None => {
                break;
            }
        }
    }

    Ok(())
}

// Main entry point. Run the main loop, catching the errors. All errors should be caught and handled
// here. Errors that are not caught are development errors that are printed with a stack trace for
// debugging.
#[tokio::main]
async fn main() -> Result<()> {
    match run().await {
        Ok(()) => {}
        Err(e) => match e.downcast_ref::<errors::AiCriticError>() {
            // Manage the expected errors here, letting unexpected ones be reported with stack
            // traces.
            Some(AiCriticError::MaxRetriesExceeded { retries }) => {
                println!("Too many retries ({}). Exiting.", retries);
            }
            _ => {
                println!("Error: {}", e);
            }
        },
    }
    Ok(())
}
