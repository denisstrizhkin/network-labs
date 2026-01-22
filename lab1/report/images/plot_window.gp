set terminal png size 800,600
set output 'report/images/efficiency_vs_window.png'
set title 'Protocol Efficiency vs Window Size (Loss Rate = 0.3)'
set xlabel 'Window Size'
set ylabel 'Efficiency Coefficient'
set grid
set key outside
plot 'report/data/gbn_vs_window.dat' using 1:2 with linespoints title 'Go-Back-N', \
     'report/data/sr_vs_window.dat' using 1:2 with linespoints title 'Selective Repeat'
