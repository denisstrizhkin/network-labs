#!/bin/bash

# Run benchmarks
echo "Running benchmarks..."
if command -v fish > /dev/null; then
    fish -c "cargo run --release"
else
    cargo run --release
fi

# Generate plots
echo "Generating plots..."
gnuplot report/images/plot_loss.gp
gnuplot report/images/plot_window.gp

echo "Plots generated in report/images/"
