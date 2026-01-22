#!/bin/bash

# Generate PDF
echo "Generating PDF..."
cd report
pandoc --pdf-engine=weasyprint --mathml report.md -o final_report.pdf
cd ..

echo "Done! Report generated at report/final_report.pdf"
