#!/bin/bash

# Run the 8 problems 3 times and average the number of iterations required to solve each problem.
output_file="avg_iters_required.out"

# Clear the file.
true >$output_file

# Loop over each of the 8 problems.
for i in {1..8}; do
    total_iterations=0
    for _ in {1..3}; do
        cargo run -- problems/coding_problem${i}.txt
        exit_status=$?
        total_iterations=$((total_iterations + exit_status))
    done
    average_iterations=$((total_iterations / 3))
    echo "Average #iterations over 3 runs for problem $i: $average_iterations" >>$output_file
done
