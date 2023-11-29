use coder::CoderAgent;
use compiler::CompilerAgent;
use critic::CriticAgent;
use fixer::FixerAgent;
use futures::future::join_all;
use std::env;
use std::{error::Error, process::exit};

mod chatter_json;
mod coder;
mod compiler;
mod critic;
mod errors;
mod fixer;

const NUM_CRITICS: usize = 3;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    if env::var("OPENAI_API_KEY").is_err() {
        eprintln!("Please set the OPENAI_API_KEY environment variable.");
        exit(1);
    }
    let coder1 = CoderAgent::new(1)?;
    let goal = r#"Write a program that shows how to remove duplicate values from a sorted \
                      linked list. That is, if there is a sequence of values in the list that are \
                      the same, then the whole sequence should be removed from the list. Do not \
                      use an existing library for linked lists. Assume the following definition of \
                      ListNode: \
                      // Definition for singly-linked list. \
                      // #[derive(PartialEq, Eq, Clone, Debug)] \
                      // pub struct ListNode { \
                      //   pub val: i32, \
                      //   pub next: Option<Box<ListNode>> \
                      // } \
                      \
                      And code this function: \
                        pub fn delete_duplicates(head: Option<Box<ListNode>>) -> Option<Box<ListNode>> {}"#;
    println!("Coder 1 requesting:\n{}\n", goal);
    let mut code = coder1.chat(goal).await?;

    let mut proposal_count = 1;
    loop {
        let mut critics = vec![];
        for i in 1..=NUM_CRITICS {
            critics.push(CriticAgent::new(i)?);
        }
        let fixer1 = FixerAgent::new(1)?;
        let compiler = CompilerAgent::new(1);

        println!("Proposed code #{}: -----------\n{}", proposal_count, &code);
        println!("------------------------------\n");
        println!("Critics evaluating...");
        let mut tasks = vec![];
        for c in critics {
            let cc = code.clone();
            tasks.push(tokio::task::spawn(async move { c.chat(&cc).await }));
        }

        // The join! macro will wait for all three async methods to complete.
        let results = join_all(tasks).await;
        // Extract errors / continue with successful API calls.
        let mut corrections = Vec::new();
        for result in results {
            match result {
                Ok(ok_result) => match ok_result {
                    Ok(correction) => corrections.push(correction),
                    Err(e) => return Err(e.into()), // Handle error in `c.chat()`
                },
                Err(e) => return Err(e.into()), // JoinError is unlikely.
            }
        }

        let mut suggestions = vec![];
        for c in &mut corrections {
            if !c.correct {
                suggestions.append(&mut c.corrections);
            }
        }
        let agreement = if suggestions.is_empty() {
            println!("All correct. Compiling...");
            true
        } else {
            println!("Critics did not approve of the code:");
            for s in &suggestions {
                println!("• {}", s);
            }
            println!("\n==> Fixer correcting...");
            code = fixer1.chat(goal, &code, &suggestions).await?;
            println!("Fixer corrects to:\n{}", code);
            false
        };

        if agreement {
            println!("==> Compiling...");
            compiler.compile(&code).await?;
            println!(
                "Success after {} proposals!\n Final code:\n --------------\n{}\n --------------\n",
                proposal_count, &code
            );
            break;
        }
        proposal_count += 1;
    }
    Ok(())
}
