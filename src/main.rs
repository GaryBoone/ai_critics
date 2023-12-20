use crate::critic::CriticType;
use clap::Parser;
use coder::{Code, CoderAgent};
use color_eyre::Result;
use critic::{Correction, CriticAgent};
use errors::AiCriticError;
use fixer::{FixerAgent, ReviewNeeded, ReviewType};
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
// NUM_CRITICS is the number of each kind of critic that will be used.
const DEFAULT_NUM_CRITICS: usize = 1;
// MAX_PROPOSALS is the maximum number of attempts to solve the coding problem.
const MAX_PROPOSALS: usize = 20;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Number of critics to use.
    #[arg(short, long, default_value_t = DEFAULT_NUM_CRITICS)]
    num_critics: usize,

    /// Problem file to use.
    #[arg(short, long, default_value_t = DEFAULT_PROBLEM_FILE.to_string())]
    problem_file: String,

    /// Use only a general critic.
    #[arg(short, long, default_value_t = false)]
    general_critic_only: bool,
}

fn setup() -> Result<Args> {
    pretty_env_logger::init();

    if env::var("OPENAI_API_KEY").is_err() {
        println!("Please set the OPENAI_API_KEY environment variable.");
        exit(1);
    }

    backtraces::setup_color_eyre()?;

    Ok(Args::parse())
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

// Spawn the critics' API calls as parallel tasks. Return the tasks so that they can be joined
// later. Also return a MultiProgress bar so that the progress bars can be managed as a group for
// all of the critics.
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

// Combine the results of the given critics into a single vector. Return an error if any of the
// critics failed.
fn collect_comments(
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
        println!("    Correct? {}", c.lgtm);
        if !c.lgtm {
            for s in c.corrections.iter() {
                println!("    • {}", s);
            }
        }
    }
}

// Have the AI Critics review the code. Return ReviewNeeded with their comments or None if all of
// them agree that the code is correct.
async fn ai_review_code(
    num_critics: usize,
    proposal_count: usize,
    problem: &str,
    code: &Code,
    general_critic_only: bool,
) -> Result<Option<ReviewNeeded>> {
    let critics = create_critics(num_critics, general_critic_only)?;

    println!(
        "Proposed code #{}: -----------\n{}",
        proposal_count, &code.code
    );
    println!("------------------------------\n");
    println!("\n==> Critics reviewing...");

    // Spawn the critic tasks.
    let (tasks, multi_progress) = spawn_critics(critics, problem, code)?;

    // Wait for the critic tasks to complete.
    let results = join_all(tasks).await;
    multi_progress.clear()?;

    // Collect the results.
    let corrections = collect_comments(results)?;

    print_corrections(&corrections);

    if corrections.iter().all(|item| item.lgtm) {
        println!("All of the critics agree that code is correct.");
        return Ok(None);
    }

    // For the Corrections that say the code is incorrect, collect the review comments into a
    // HashSet, deduping them. Note that comments from GPT are often the same idea but using
    // different words, so this deduplication only removes the less frequent literal duplicates.
    // Return them as a Vec<String>.
    let comments: Vec<String> = corrections
        .iter()
        .filter(|cs| !cs.lgtm)
        .flat_map(|cs| &cs.corrections)
        .cloned()
        .collect::<HashSet<String>>()
        .into_iter()
        .collect();

    Ok(Some(ReviewNeeded {
        review_type: ReviewType::CodeReview,
        comments,
    }))
}

// Create the set of critics, whether general or specific, based on the requested number of critics.
// Note that if the general_critics_only flag is set, then the number of general critics is the
// requested number of critics. Otherwise, the total number of critics is the requested number * 3
// because there is one design, one correctness, and one syntax critic for each requested number of
// critics.
fn create_critics(num_critics: usize, general_critics_only: bool) -> Result<Vec<CriticAgent>> {
    let mut critics = vec![];
    if general_critics_only {
        for i in 1..=num_critics {
            critics.push(CriticAgent::new(CriticType::General, i)?);
        }
    } else {
        for i in 1..=num_critics {
            critics.push(CriticAgent::new(CriticType::Design, i)?);
        }
        for i in 1..=num_critics {
            critics.push(CriticAgent::new(CriticType::Correctness, i)?);
        }
        for i in 1..=num_critics {
            critics.push(CriticAgent::new(CriticType::Syntax, i)?);
        }
    }
    Ok(critics)
}

// Pretty print the current code and iteration count.
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

// Pretty print the current error.
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

// Have the AI Fixer agent correct the code given the critics' comments.
async fn ai_fix_code(code: &Code, review: ReviewNeeded) -> Result<Code> {
    println!("\n==> Fixer correcting...");

    let fixer1 = FixerAgent::new(1)?;
    let mut pb = DoublingProgressBar::new(&fixer1.name)?;
    let code = fixer1.chat(&mut pb, &code.code, review).await?;
    Ok(code)
}

// Compile and test the code. Return an optional ReviewNeeded if the code fails to compile or fails
// the test.
async fn compile_and_test(proposal_count: usize, code: &Code) -> Result<Option<ReviewNeeded>> {
    println!("\n==> Tester compiling and testing...");
    let tester = TesterAgent::new(1);

    match tester.compile_and_test(&code.code).await? {
        TesterResult::Success { stdout, .. } => {
            report_test_success(proposal_count, &code.code, &stdout);
            Ok(None)
        }
        TesterResult::Failure {
            output: stdout,
            review,
        } => {
            report_tester_failure(&stdout);
            // Continue, seeing if the AI can fix the code/tests so it passes.
            Ok(Some(review))
        }
    }
}

// Main run loop: Read the problem and run the AI agents to solve it. Use a Coder agent to produce
// an initial solution, then in a loop run the AI critics to review the code, the fixer agent to
// correct it, and the tester agent to test it. Repeat until it works or MAX_PROPOSALS is reached.
async fn run() -> Result<usize> {
    let args = setup()?;

    let problem = read_coding_problem(&args.problem_file)?;

    let mut code = ai_write_code(&problem).await?;

    for proposal_count in 1..=MAX_PROPOSALS {
        let review_res = ai_review_code(
            args.num_critics,
            proposal_count,
            &problem,
            &code,
            args.general_critic_only,
        )
        .await?;
        if let Some(review_needed) = review_res {
            code = ai_fix_code(&code, review_needed).await?;
        }
        match compile_and_test(proposal_count, &code).await? {
            Some(review_needed) => {
                code = ai_fix_code(&code, review_needed).await?;
            }
            None => {
                return Ok(proposal_count);
            }
        }
    }

    Err(AiCriticError::MaxProposalsExceeded {
        proposals: MAX_PROPOSALS,
    }
    .into())
}

// Main entry point. Run the main loop, catching the errors. All errors should be caught and handled
// here. Errors that are not caught are development errors that are printed with a stack trace for
// debugging. Return code 0 indicates an error while >1 is the number of iterations it took to
// solve the problem. 255 means that the program failed to converge.
#[tokio::main]
async fn main() {
    match run().await {
        Ok(iteration_count) => {
            std::process::exit(iteration_count as i32);
        }
        Err(e) => match e.downcast_ref::<errors::AiCriticError>() {
            // Manage the expected errors here, letting unexpected ones be reported with stack
            // traces.
            Some(AiCriticError::MaxProposalsExceeded { proposals }) => {
                println!(
                    "The AI critics failed to converge on a solution in {} proposals. Exiting.",
                    proposals
                );
                std::process::exit(255);
            }
            _ => {
                println!("Error: {}", e);
                std::process::exit(0);
            }
        },
    }
}
