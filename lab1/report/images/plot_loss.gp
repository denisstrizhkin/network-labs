set terminal png size 800,600
set output 'report/images/efficiency_vs_loss.png'
set title 'Protocol Efficiency vs Loss Rate (Window Size = 5)'
set xlabel 'Loss Rate'
set ylabel 'Efficiency Coefficient'
set grid
set key outside
plot 'report/data/gbn_vs_loss.dat' using 1:2 with linespoints title 'Go-Back-N', \
     'report/data/sr_vs_loss.dat' using 1:2 with linespoints title 'Selective Repeat'
