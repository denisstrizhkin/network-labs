use lab1::{gobackn, selective_repeat};
use std::fs::File;
use std::io::Write;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let message = "A".repeat(1000); // 1000 bytes message
    let loss_rates = [0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9];
    let window_sizes = [5, 10, 20];

    // Plot 1: Efficiency vs Loss Rate (fixed Window Size = 10)
    let fixed_window = 10;
    let mut gbn_loss_data = File::create("report/data/gbn_vs_loss.dat")?;
    let mut sr_loss_data = File::create("report/data/sr_vs_loss.dat")?;

    println!("Collecting data for Efficiency vs Loss Rate (Window Size = 10)...");
    for &loss in &loss_rates {
        println!("Loss rate: {}", loss);
        let (_, gbn_eff) = gobackn::silent_setup_loss(fixed_window, &message, loss);
        let (_, sr_eff) = selective_repeat::silent_setup_loss(fixed_window, &message, loss);
        writeln!(gbn_loss_data, "{} {}", loss, gbn_eff)?;
        writeln!(sr_loss_data, "{} {}", loss, sr_eff)?;
    }

    // Plot 2: Efficiency vs Window Size (fixed Loss Rate = 0.3)
    let fixed_loss = 0.3;
    let mut gbn_window_data = File::create("report/data/gbn_vs_window.dat")?;
    let mut sr_window_data = File::create("report/data/sr_vs_window.dat")?;

    println!("Collecting data for Efficiency vs Window Size (Loss Rate = 0.3)...");
    for &window in &window_sizes {
        println!("Window size: {}", window);
        let (_, gbn_eff) = gobackn::silent_setup_loss(window as u32, &message, fixed_loss);
        let (_, sr_eff) = selective_repeat::silent_setup_loss(window as u32, &message, fixed_loss);
        writeln!(gbn_window_data, "{} {}", window, gbn_eff)?;
        writeln!(sr_window_data, "{} {}", window, sr_eff)?;
    }

    println!("Data collection complete.");
    Ok(())
}
