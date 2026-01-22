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

# Generate PDF
echo "Generating PDF..."
cd report
pandoc --pdf-engine=weasyprint report.md -o final_report.pdf
cd ..

echo "Done! Report generated at report/final_report.pdf"
