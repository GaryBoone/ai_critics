# AI Coding Critics

## Can AI fix its own code?

## Intro: Recursive LLMs

Today's LLMs like GPT-4 can write sample code, but notoriously, their code is typically buggy and
incomplete with mistakes ranging rom simple syntax errors to hallucinated functions and libraries.
Even so, they are incredible tools for developers, providing a new dimension to coding: almost fully
intelligent help. But if they can help developers fix their coding errors, then can AI correct
their own mistakes?

This repo solves coding problems using AI to write initial code, then review and
correct the code until it successfully compiles and passes its tests.

This repo also creates a new kind of software: AI Algorithms. As AI is deployed to developers, we'll need to craft new algorithms that arrange and pipeline AI interactions to solve coding problems.

In this case, the AI Algorithm is: `AI code -> AI review -> AI fix -> compile/test -> repeat`. Will it work?

Turning LLMs into AI critics, reviewing and correcting their own code, raises several key questions:

1. How should AI critics be structured? Are different kinds of code review that specify different
   potential problems more useful than single, comprehensive, review prompts?
1. Does the number of AI reviewers matter? Do more Ai critics find more mistakes more quickly?
1. Does the AI review loop converge? That is, will the code/review/correct cycle eventually
   produce a final answer with no further review comments? Or will the AI review loop diverge,
   whether through continuously introduced mistakes or through endless, perhaps, conflicting advice or unrepaired code defects?

This repo answers these questions. Here's how it works:

1. The `ai-critics` program first gives a coding problem to AI Coder agent to solve.
1. AI Critic agents review the code, returning
   suggested fixes and improvements.
1. An AI Fixer agent will correct the code based on the suggested improvements and return new code.
1. If the critics judge that the code is correct, then a non-AI Tester agent will compile and test the code.
1. If the code fails compilation or testing, the code will be returned to the AI Fixer agent and the process will repeat.

This process loops until the code passes its tests or an iteration limit is reached.

## The Agents

In the research literature, the word "agent" is used to refer to a program that interacts with its
environment with some autonomy and intelligence, performing tasks that advance the program's goals.
The term is sometimes overused when it simply describes fixed subroutines. In this project, the
agents really are simple subroutines that make OpenAI API calls. However, they call actually
intelligent software, GPT-4, with its autonomy in how it responds to requests! Arguably, for the
first time, software using LLM APIs really are true autonomous intelligent agents.

There are several kinds of agents that create the code/review/fix/test loop. Each agent has a
system prompt and a user prompt as is expected for GPT-4 API calls.

## AI Coder Agent

The AI Coder agent calls GPT-4 with a coding problem in the user prompt like:

```
Given n, a number of pairs of open and closed parentheses, write a function to generate all combinations of well-formed parentheses.
```

and a system prompt like:

```quote
Write the requested program in Rust. Include complete unit tests. Return the code as JSON in a string field called `code`.

Any clarifying explanations should be included in the code as // comments.
Be sure that the tests demonstrate that the code solves the requested problem.

Any `assert` used should include a custom message with a unique 6-digit hex number labelled `assert_id` that uniquely identifies the assert line so that line numbers are not required.

```

The code given to GPT-4 does not have line numbers, so the `assert_id` instruction allows GPT-4 to
find assertion errors despite not having line numbers.

## AI Critic Agent

The AI Critic agents are given the code proposed by the Coder agent as a user prompt. There are
several types of AI Critic, depending on the type of code review they are asked to do. These agents
receive the same user prompt, but different system prompts.

To create the user prompt given to GPT-4, the coding problem and the current code are combined with
with the following base prompt:

```
Evaluate this code based on the criteria below. Make no comments or explanations.
Return JSON with two fields:
1. a field named `correct` with value `true` if the code is correct, else false.
2. a field `corrections` containing list of the errors, if any, else `None`.
```

### AI Design Critic Agent

AI Design Critics use the following system prompt:

```
Evaluation Criteria: Evaluate the _design_ of the solution, considering the following questions:
1. Is this the right the design to solve the problem?
2. Does the method chosen meet the constraints of the problem?
3. Does it use a the correct algorithms and data structures to solve the problem?
```

### AI Correctness Critic Agent

AI Correctness Critics use the following system prompt:

```
Evaluation Criteria: Evaluate the _correctness_ of the solution, considering the following questions:
1. Does the code correctly implement the intended solution approach?
2. Does the code generate the expected output?
3. Does the output meet the original problem constraints?
4. Are there enough tests to demonstrate the correctness of the solution?
5. Do the tests correctly capture situations that validate or invalidate the solution?
```

### AI Syntax Critic Agent

AI Syntax Critics use the following system prompt:

```
Evaluation Criteria: Evaluate the _syntax_ of the solution, considering the following questions:
1. Are there any syntactic errors?
2. Will the code and tests compile and run?
3. Are there any language errors such as borrowing violations or lifetime problems?
4. Are there any cleanups needed such as unused variables or imports?
```

## AI Fixer Agent

The AI Fixer agent corrects the current code using the feedback of the AI Critic agent suggestions
or the compilation/test errors found by the Tester agent. Its system prompt is:

```
Correct the code, returning the fixed code as JSON in a string field called `code`.";
```

Then the user prompt combines the code, the feedback, and the appropriate base prompt:

- Code Review Comments:

```
Specifically address these code review issues:
```

- Compiler Errors:

```
Fix the code so that it compiles.
Correct the compilation errors without changing the code's functionality.
The code failed to compile with the following errors:
```

- Test Failures:

```
The code failed its unit tests as shown below. Do the following:
1. Match the given `assert_id` value to the assert() in the code to find the assertion that
    failed.
2. Determine if the test design is correct for the test that failed. If not, do not correct the
    assertion, but instead correct the test.
3. If the test design is correct, determine if the assert line is correct. If not, write the
    correct line.
4. Fix the code so that it passes the test.
This is the output of the failed test:
```

## Tester Agent

The Tester agent is not AI-based and does not call GPT-4. Instead uses the Rust compiler to compile
and run the code. The Tester agent also runs the tests defined in the code. Errors in compilation or
testing are returned to the Fixer agent for correction as described above.

## The Code

The `ai_critics` code is primarily focused on reliably interacting with the OpenAI API. In addition
to the intelligent code review goal of the repo described above, the code demonstrates how to handle
a number of issues:

1. The API sometimes returns long runs of spaces and newlines. This is apparently generated by the
   GPT model, uncaught by the API. The problem is eventually terminated when the output reaches the
   requested maximum length. The API will return a `ChatChoice` with a `FinishReason` of `Length`.
   However, this is inefficient, especially as the requested response lengths increase. So this code
   demonstrates how to catch the terminate the runaway response early.
1. The code demonstrates request timeouts and other standard API error handling.
1. It demonstrates the use of streaming responses, providing continuous feedback to the user
   while GPT-4 figures out its answers.
1. It shows how to generate and return structured responses as JSON, as well as how to
   deserialize these into struct instances.

### Running the code

To run the code, you need to set the `OPENAI_API_KEY` environment variable to a valid OpenAi API key. View/create one [here](https://platform.openai.com/api-keys).

Note: The code is set up to run GPT-4, specifically `gpt-4-1106-preview`. If your API key corresponds to a different model, change the `MODEL` constant in `src/chatter_JSON.rs` . The list of available models is [here](https://platform.openai.com/docs/models/continuous-model-upgrades).

Export the API key in your shell, like:

```bash
$ export OPENAI_API_KEY=your_api_key
```

Protip: Use [direnv](https://direnv.net/) to set the environment variable for you as needed.

Now you can run the code:

```bash
$ cargo run
```

or

```bash
$ cargo build
$ ./target/debug/ai_critics
```

to try the first example problem, found in `problems/coding_problem1`. To try the other problems, or one in a file of your own, pass the filename:

```bash
$ cargo run -- --problem-file=problems/coding_problem3.txt
```

You can also specify how many copies of the critics to use:

```bash
$ cargo run -- --num-critics=3
```

So this command will run 3 each of the Design, Correctness, and Syntax critics.

### Problem format

The coding problems are formatted as plain text files. Lines beginning with `#` are ignored. The problem is sent directly to GPT-4. There is no need to add any additional prompting to instruct GPT about the problem or desired output. The code will provide a system prompt to instruct the model to generate the desired output format.

## Limitations

### Compiler and Test Line Numbers

The Rust compiler and unit tests errors include line numbers. However, the code given to the AI do not include line numbers. This means that the AI will not be able to use the exact location of the error.

This limitation can lead to ambiguous unit test failures:

```
thread 'tests::test_min_coins' panicked at /var/folders/dy/1_ns2s6s2x91htdf5xzc6xqc0000gn/T/.tmpIp3aZt/code.rs:37:9:
assertion `left == right` failed
  left: 2
 right: -1
```

Without access to the line numbers, finding this error is difficult.

In practice, this limitation led to runaway no-progress cycles in which the fixer failed to correct
a unit test so the same error recurred.

Compiler errors are less of any issue because the Rust compiler includes detailed information about the nature of the error in the error message. Often the description of the error is sufficient for the LLM to find and correct the error.

Including line numbers code be done by adding them to the code when sent to GPT-4, but not when sent to the compiler.

A simple mitigation is to ask GPT to return unique identifiers with the assertion errors:

```
Any `assert` used should include a custom message with a unique 6-digit hex number  labelled
`assert_id` that uniquely identifies the assert line so that line numbers are not required.";
```

### Self-fulfillment

A limitation is that this code asks GPT-4 to write the code _and_ the unit tests, which means that
the code could pass with trivial, incomplete, or erroneous tests. In practice, the AI does attempt
to create some reasonable tests. However, there are typically only a few created. It is likely that
they are incomplete.

### Coding Puzzles

This project uses coding problems sourced from [LeetCode](www.leetcode.com) and
[GeeksForGeeks](www.geeksforgeeks.org). Citations are included in the problem files. As such, they
are algorithmic queries that are short, common, and well-known. They are not representative of
real-world development tasks. Given that GPT-4 was trained on coding repositories, it is possible,
even likely, that it has directly trained on these specific problems.

## Observations and Discussion

### Nonlinearity

Under the review of GPT-4, the code does not evolve directly to a correct solution. Instead, it
evolves in meandering directions:

- It sometimes fails to correct issues the AI code reviewers found.
- It sometimes misses obvious, easily corrected, issues that it was explicitly instructed to fix,
  such as removing unused `use` imports.
- It sometimes reintroduces defects that were previously solved.

For example, incorrect unit tests present a challenge to the AI system. If the unit test is wrong,
the AI Fixer has to not correct the code to pass the test, but realize that the test assertion is
wrong and fix that instead. This challenge led to increased iterations of the AI system. During
these iterations, the AI Fixer sometimes introduced code errors that caused additional review cycles
to correct before returning to the test failure.

In general, they do solve the coding problems in only a few iterations. See the table below.

### Deduping in an AI World

The goal of using multiple critics is to find more issues more quickly. However, critics often find the same issues, as this example output shows:

```
  Design Critic 1:
    Correct? false
    • The code uses a HashSet to track values, which is not necessary for a sorted linked list.
    • The code does not correctly remove all duplicates; it only skips adding a node if the value is already in the set or if the next node has the same value.
    • The code does not handle the case where there are multiple duplicates in a row correctly; it should remove the entire sequence of duplicates, not just prevent them from being added.
    • The use of HashSet introduces unnecessary space complexity; the problem can be solved with constant space complexity by taking advantage of the sorted property of the list.
  Correctness Critic 1:
    Correct? false
    • The code uses a HashSet to track values, which is not necessary for a sorted list.
    • The code does not correctly remove all duplicates; it only skips adding a node if the value is already in the set, which does not handle consecutive duplicates correctly.
    • The code does not handle the case where the first sequence of nodes are duplicates correctly; it will leave the first node of such a sequence in the list.
  Syntax Critic 1:
    Correct? false
    • The code does not correctly implement the removal of duplicate sequences. It uses a HashSet to track seen values, but the problem statement requires that sequences of duplicates be removed entirely, not just individual duplicates.
    • The code does not handle the case where the first sequence of nodes are duplicates.
    • The code does not handle the case where the last sequence of nodes are duplicates.
```

Note how similar the review comments are. Note, too how similar the Design, Correctness, and Syntax
review comments were, despite their specific prompts for different types of reviews.

These critics found several similar issues. However, they worded the descriptions differently.
Removing duplicates is more challenging as simple string matching will likely fail to catch many
duplicates. At the cost of another round trip to GPT-4, AI could deduplicate these. Doing so would
cause a net increase in tokens spent to obtain a small subsequent token savings, though. Nor is it
obvious that deduplicating review comments is necessary or beneficial to finding the correct coding
solutions.

### AI Stubbornness

To the problem of LLMs hallucinating, we can add stubbornness, the refusal to obey repeated requests.

Again and again, problems identified by the AI critics were not corrected in subsequent revisions. This issue often recurred even when the comment was the only issue identified by multiple critics.

For example, a common criticism returned by the LLM for some of these coding problems was to not use a HashSet. Yet the LLM persisted in including a HashSet. When it did switch to another data structure, it often refused a review comment to remove the HashSet import.

In one example, the review comments included:

```
• The code uses `Rc` and `RefCell` which are unnecessary and not used in the implementation.
...
• The code uses `Rc` and `RefCell` which are not used in the implementation and should be removed.
...
• Unused imports: `std::rc::Rc` and `std::cell::RefCell` should be removed as they are not used in the code.
```

In this case, even though these criticisms were made, the critics all next reported that the code
was correct!

### Answers to the Key Questions

The introduction included several key questions raised by the code/review/rewrite approach to AI coding. Based on the results of this work, here are the answers:

### AI Critic Structure

1. How should AI critics be structured? Are different kinds of code review, specifying different potential problems, more useful than single, comprehensive, review prompts?

Structuring specific, detailed requests did not add value. Simple requests were sufficient. For example, an earlier version of the code had a single, comprehensive, review prompt:

```
Specifically:
1. Combine similarly worded, but duplicate comments.
2. Decide if an alternative implementation or data structure is needed to
   implement the comments.
3. Follow the implications of the comments to their conclusions, such as
   removing `use` statements if you remove the items they import.
4. Choose the solution approach that implements all of the comments.
5. Write the code that implements the new solution.
6. Review and modify the code for solution correctness.
7. Review and modify the code for syntax errors.
8. Review and modify the code to ensure that all of the comments are
   implemented.
```

However, this was not found to be more effective than the current prompt for AI Critics. It was not
clear that the AI critics were able to follow these steps. They never made large algorithm design or
problem approach changes, for example.

The AI critics were defined around different code review types: Design, Correctness, and Syntax. Each Critic used a prompt that specifically requested that the code be reviewed for these criteria. In fact, the data shows that the review specializations didn't matter. The coding problems were just as easily and quickly solved using a single review prompt, "Review the code for design, correctness, and syntax issues.".

### AI Critic Count

2. Does the number of AI reviewers matter? Do more Ai critics find more mistakes more quickly?

Adding more AI critics did not find more issues more quickly.

### AI Critic Convergence

3. Does the criticism converge at all? That is, will the code/review/correct cycle eventually
   produce a final answer with no further review comments? Or will the AI review loop diverge,
   whether through continuously introduced mistakes or through endless, perhaps, conflicting advice?

The AI critics loop converges. While the can sometimes take several iterations to find the correct
solution, in all cases the found solutions that passed the unit tests.

### Data

![Plot: Iterations Required vs Number of Critics for the 3 Different Critic Types](data/iterations_vs_critics_3_critic_types.png)

Plot: Iterations Required vs Number of Critics for the 3 Different Critic Types

As can be seen, while some runs required more iterations, these are likely outliers struggling to
correct unit tests. In general, increasing the number of critics did not prevent the review loop
from converging to a successful solution. There may be a small correlation between the iterations
required and the number of critics, but even with as many as 15 AI code reviewers the `ai_critics` review loop converged to a solution in only a few iterations.
