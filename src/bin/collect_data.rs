use std::fs::File;
use std::io::{self, Write};
use std::process::{Command, Output};
#[cfg(not(test))]
use {std::thread::sleep, std::time::Duration};

//
// Run like:
// $ cargo run --bin collect_data
//
// Test like:
// $ cargo test --bin collect_data -- --nocapture

const NUM_PROBLEMS: usize = 8;
const NUM_ITERATIONS: usize = 3;
const OUTPUT_FILENAME: &str = "iterations_data.csv";
const PROBLEM_BASE: &str = "problems/coding_problem";
const PROBLEM_SUFFIX: &str = ".txt";
const NUM_CRITICS_VALUES: [usize; 3] = [1, 3, 5];
const NUM_RETRIES: usize = 3;
const GENERAL_CRITIC_ONLY: bool = false;

struct Outcome {
    // The number of times that the AI critics found a solution.
    success_count: usize,
    // The number of failures by network or unknown reason.
    failure_count: usize,
    // The number of times the AI critics failed to find a solution.
    divergence_count: usize,
    // The number of iterations that the AI critic needed to find a solution.
    success_iterations: usize,
}

pub trait CommandRunner {
    fn run(&self, args: &[String]) -> io::Result<Output>;
}

pub struct RealCommandRunner;

impl CommandRunner for RealCommandRunner {
    fn run(&self, args: &[String]) -> io::Result<Output> {
        Command::new("cargo")
            .env("RUST_LOG", "info")
            .args(args)
            .output()
    }
}

pub struct DataCollector<'a> {
    command_runner: &'a dyn CommandRunner,
}

impl<'a> DataCollector<'a> {
    pub fn new(command_runner: &'a dyn CommandRunner) -> Self {
        DataCollector { command_runner }
    }

    pub fn collect_data<W: Write>(&self, file: &mut W) -> io::Result<()> {
        println!(
            "[collect_data] Running ai_critic for {:?} critics...",
            NUM_CRITICS_VALUES
        );
        for num_critics in &NUM_CRITICS_VALUES {
            println!(
                "[collect_data] Running ai_critic with {} critics...",
                num_critics
            );
            self.process_problems_for_num_critics(*num_critics, file, GENERAL_CRITIC_ONLY)?;
        }

        Ok(())
    }

    fn process_problems_for_num_critics<W: Write>(
        &self,
        num_critics: usize,
        file: &mut W,
        general_critic_only: bool,
    ) -> io::Result<()> {
        println!("[collect_data] Running {} problems...", NUM_PROBLEMS);
        for i in 1..=NUM_PROBLEMS {
            println!("[collect_data] Running problem #{}...", i);
            let outcome = self.run_iterations_for_problem(i, num_critics, general_critic_only)?;

            writeln!(
                file,
                "{},{},{},{},{},{}",
                i,
                num_critics,
                outcome.success_count,
                outcome.failure_count,
                outcome.divergence_count,
                outcome.success_iterations
            )?;
        }

        Ok(())
    }

    fn run_iterations_for_problem(
        &self,
        problem_number: usize,
        num_critics: usize,
        general_critic_only: bool,
    ) -> io::Result<Outcome> {
        let mut success_count = 0;
        let mut failure_count = 0;
        let mut divergence_count = 0;
        let mut success_iterations = 0;

        println!("[collect_data] Running {} iterations...", NUM_ITERATIONS);
        for i in 1..=NUM_ITERATIONS {
            println!("[collect_data] Running iteration {}...", i);
            let iterations =
                self.run_command_with_retries(problem_number, num_critics, general_critic_only)?; // 0 indicates error.
            println!("[collect_data] i {} ==> iterations {}.", i, iterations);
            match iterations {
                0 => {
                    failure_count += 1;
                }
                255 => {
                    divergence_count += 1;
                }
                _ => {
                    success_count += 1;
                    success_iterations += iterations;
                }
            }
        }

        Ok(Outcome {
            success_count,
            failure_count,
            divergence_count,
            success_iterations,
        })
    }

    fn run_command_with_retries(
        &self,
        problem_number: usize,
        num_critics: usize,
        general_critic_only: bool,
    ) -> io::Result<usize> {
        let mut retries = 0;
        let mut args = vec![
            "run".to_string(),
            "--".to_string(),
            format!(
                "--problem-file={}{}{}",
                PROBLEM_BASE, problem_number, PROBLEM_SUFFIX
            ),
            format!("--num-critics={}", num_critics),
        ];
        if general_critic_only {
            args.push("--general-critic-only".to_string());
        }
        while retries < NUM_RETRIES {
            println!("[collect_data] Running `cargo {}`...", args.join(" "));
            let output = self.command_runner.run(&args)?;
            let status = output.status;
            match status.code() {
                Some(code) if code < 0 => {
                    return Err(io::Error::new(
                        io::ErrorKind::Other,
                        format!("unexpected error (exit code: {}); exiting", code),
                    ));
                }
                Some(code) if code > 0 => {
                    // An exit code > 0 indicates success where the value indicates the number of
                    // iterations. 255 indicates a convergence failure.
                    return Ok(code as usize);
                }
                Some(_) => {
                    // An exit code of 0 indicates a program error; retry.
                }
                None => {
                    // Unknown error; retry.
                }
            };

            retries += 1;
            #[cfg(not(test))]
            {
                println!("[collect_data] Sleeping for 5 seconds before retry...");
                sleep(Duration::from_secs(5));
            }
        }

        Ok(0)
    }
}

fn main() -> io::Result<()> {
    let command_runner = RealCommandRunner;
    let data_collector = DataCollector::new(&command_runner);

    let mut file = File::create(OUTPUT_FILENAME)?;
    writeln!(
        file,
        "Problem,NumCritics,SuccessCount,FailureCount,DivergenceCount,SuccessIterations"
    )?;

    data_collector.collect_data(&mut file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::{os::unix::process::ExitStatusExt, process::ExitStatus};

    #[derive(Debug)]
    struct MockCommandRunner {
        exit_codes: RefCell<Vec<i32>>,
    }

    impl MockCommandRunner {
        fn new(mut exit_codes: Vec<i32>) -> Self {
            // Reverse the order of the exit codes so that later we can use pop() to remove them in
            // the correct order.
            exit_codes.reverse();
            MockCommandRunner {
                exit_codes: RefCell::new(exit_codes),
            }
        }
    }

    impl CommandRunner for MockCommandRunner {
        fn run(&self, _args: &[String]) -> io::Result<Output> {
            let exit_code = self.exit_codes.borrow_mut().pop().unwrap_or(0);
            // Shift the exit code into the higher-order bits.
            let status_code = exit_code << 8;
            Ok(Output {
                status: ExitStatus::from_raw(status_code),
                stdout: vec![],
                stderr: vec![],
            })
        }
    }

    #[test]
    fn test_process_problems_for_num_critics_all_success() {
        let mock_command_runner = MockCommandRunner::new(vec![1, 2, 3, 1, 2, 3, 1, 2, 3]);
        let data_collector = DataCollector::new(&mock_command_runner);
        let mut mock_file = Vec::new();

        data_collector
            .process_problems_for_num_critics(1, &mut mock_file, false)
            .unwrap();

        let output = std::str::from_utf8(&mock_file).unwrap();
        // "Problem,NumCritics,SuccessCount,FailureCount,DivergenceCount,SuccessIterations"
        assert!(output.contains("1,1,3,0,0,6")); // First problem.
        assert!(output.contains("2,1,3,0,0,6")); // Second problem.
        assert!(output.contains("3,1,3,0,0,6")); // ...
        assert!(output.contains("4,1,0,3,0,0")); // Exit codes are 0 after 9th one above...
        assert!(output.contains("5,1,0,3,0,0"));
        assert!(output.contains("6,1,0,3,0,0"));
        assert!(output.contains("7,1,0,3,0,0"));
        assert!(output.contains("8,1,0,3,0,0"));
    }

    #[test]
    fn test_process_problems_for_num_critics_mixed_outcomes() {
        let mock_command_runner = MockCommandRunner::new(vec![1, 0, 255, 2, 0, 255, 3, 0, 255]);
        let data_collector = DataCollector::new(&mock_command_runner);
        let mut mock_file = Vec::new();

        data_collector
            .process_problems_for_num_critics(1, &mut mock_file, false)
            .unwrap();

        let output = std::str::from_utf8(&mock_file).unwrap();

        // "Problem,NumCritics,SuccessCount,FailureCount,DivergenceCount,SuccessIterations"
        // First problem:
        //   NUM_ITERATIONS = 3, exit codes to consume = [1, 0, 255, 2, 0, 255, 3, 0, 255]
        //   iteration 1: 1 => a success (+1 iteration)
        //   iteration 2: 0 is retried, 255 => a divergence
        //   iteration 3: 2  => a success (+2 iteration)
        // So we have problem 1, 1 critic, 2 successes, no failures, 1 divergence, and 3 iterations:
        // 1,1,2,0,1,3
        assert!(output.contains("1,1,2,0,1,3")); // First problem.
        assert!(output.contains("2,1,1,0,2,3")); // Second.
        assert!(output.contains("3,1,0,3,0,0")); // ...
        assert!(output.contains("4,1,0,3,0,0"));
        assert!(output.contains("5,1,0,3,0,0"));
        assert!(output.contains("6,1,0,3,0,0"));
        assert!(output.contains("7,1,0,3,0,0"));
        assert!(output.contains("8,1,0,3,0,0"));
    }

    #[test]
    fn test_run_command_with_retries_success() {
        let mock_command_runner = MockCommandRunner::new(vec![4]);
        let data_collector = DataCollector::new(&mock_command_runner);

        let result = data_collector.run_command_with_retries(1, 1, false);
        assert_eq!(result.unwrap(), 4);
    }

    #[test]
    fn test_run_command_with_retries_failure() {
        let mock_command_runner = MockCommandRunner::new(vec![0, 0, 0, 0, 0, 0]); // 6 Retry fails.
        let data_collector = DataCollector::new(&mock_command_runner);

        let result = data_collector.run_command_with_retries(1, 1, false);
        assert_eq!(result.unwrap(), 0);
    }

    #[test]
    fn test_run_command_with_retries_divergence() {
        // Note that the run_command_with_retries() doesn't know about the exit code 255, so it returns it as is.
        let mock_command_runner = MockCommandRunner::new(vec![255]);
        let data_collector = DataCollector::new(&mock_command_runner);

        let result = data_collector.run_command_with_retries(1, 1, false);
        assert_eq!(result.unwrap(), 255);
    }

    #[test]
    fn test_run_command_with_retries_retry() {
        let mock_command_runner = MockCommandRunner::new(vec![0, 0, 2]); // Fails twice, then succeeds.
        let data_collector = DataCollector::new(&mock_command_runner);

        let result = data_collector.run_command_with_retries(1, 1, false);
        assert_eq!(result.unwrap(), 2);
    }

    #[test]
    fn test_run_iterations_for_problem_success() {
        let mock_command_runner = MockCommandRunner::new(vec![1, 2, 3]); // Three successes.
        let data_collector = DataCollector::new(&mock_command_runner);

        let outcome = data_collector
            .run_iterations_for_problem(1, 1, false)
            .unwrap();
        assert_eq!(outcome.success_count, 3);
        assert_eq!(outcome.failure_count, 0);
        assert_eq!(outcome.divergence_count, 0);
        assert_eq!(outcome.success_iterations, 6); // 1 + 2 + 3.
    }

    #[test]
    fn test_run_iterations_for_problem_failure() {
        let mock_command_runner = MockCommandRunner::new(vec![0, 0, 0, 0, 0]);
        let data_collector = DataCollector::new(&mock_command_runner);

        let outcome = data_collector
            .run_iterations_for_problem(1, 1, false)
            .unwrap();
        assert_eq!(outcome.success_count, 0);
        assert_eq!(outcome.failure_count, 3);
        assert_eq!(outcome.divergence_count, 0);
        assert_eq!(outcome.success_iterations, 0);
    }

    #[test]
    fn test_run_iterations_for_problem_divergence() {
        let mock_command_runner = MockCommandRunner::new(vec![255, 255, 255]); // Three divergence failures
        let data_collector = DataCollector::new(&mock_command_runner);

        let outcome = data_collector
            .run_iterations_for_problem(1, 1, false)
            .unwrap();
        assert_eq!(outcome.success_count, 0);
        assert_eq!(outcome.failure_count, 0);
        assert_eq!(outcome.divergence_count, 3);
        assert_eq!(outcome.success_iterations, 0);
    }
}
