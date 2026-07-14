use std::error::Error;
use std::fs::File;
use std::path::PathBuf;

use clap::Parser;
use plotters::prelude::*;
use serde::Deserialize;

#[derive(Parser, Debug)]
#[command(author, version, about = "Plot and compare multiple S-Curve motion profiles from CSV files")]
struct Args {
    /// Input CSV files containing motion data
    #[arg(required = true, value_name = "FILES")]
    files: Vec<PathBuf>,

    /// Output image file path (supports .svg or .png)
    #[arg(short, long, default_value = "plot.svg")]
    output: PathBuf,
}

// A flexible deserializer struct matching potential CSV headers
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct CsvRow {
    #[serde(alias = "step")]
    step: f64,
    #[serde(alias = "timecycles", alias = "Time")]
    time_cycles: Option<f64>,
    #[serde(alias = "intervalcycles", alias = "Interval")]
    interval_cycles: f64,
    #[serde(alias = "velocity", alias = "Vel")]
    velocity: f64,
    #[serde(alias = "accel", alias = "acceleration")]
    accel: f64,
    #[serde(alias = "jerk")]
    jerk: f64,
    #[serde(alias = "position", alias = "pos")]
    position: Option<f64>,
}

struct ParsedDataset {
    filename: String,
    time_points: Vec<f64>,
    position: Vec<f64>,
    velocity: Vec<f64>,
    accel: Vec<f64>,
    jerk: Vec<f64>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    let mut datasets = Vec::new();

    // 1. Parse all CSV files
    for path in &args.files {
        let file = File::open(path)?;
        let mut rdr = csv::ReaderBuilder::new()
            .trim(csv::Trim::All)
            .from_reader(file);

        let mut time_points = Vec::new();
        let mut position = Vec::new();
        let mut velocity = Vec::new();
        let mut accel = Vec::new();
        let mut jerk = Vec::new();

        let mut cumulative_cycles = 0.0;

        for result in rdr.deserialize() {
            let row: CsvRow = result?;

            // Reconstruct time if explicit TimeCycles is missing
            let t = match row.time_cycles {
                Some(t) => t,
                None => {
                    cumulative_cycles += row.interval_cycles;
                    cumulative_cycles
                }
            };

            // Fallback to step index for position if no position column exists
            let pos = row.position.unwrap_or(row.step);

            time_points.push(t);
            position.push(pos);
            velocity.push(row.velocity);
            accel.push(row.accel);
            jerk.push(row.jerk);
        }

        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Unknown")
            .to_string();

        datasets.push(ParsedDataset {
            filename,
            time_points,
            position,
            velocity,
            accel,
            jerk,
        });
    }

    // 2. Find global min and max across all series for chart auto-scaling
    let mut x_min = f64::INFINITY;
    let mut x_max = f64::NEG_INFINITY;
    let mut y_min = f64::INFINITY;
    let mut y_max = f64::NEG_INFINITY;

    for ds in &datasets {
        for &x in &ds.time_points {
            if x < x_min { x_min = x; }
            if x > x_max { x_max = x; }
        }
        let all_ys = ds.position.iter()
            .chain(&ds.velocity)
            .chain(&ds.accel)
            .chain(&ds.jerk);

        for &y in all_ys {
            if y < y_min { y_min = y; }
            if y > y_max { y_max = y; }
        }
    }

    // Pad limits slightly for visualization comfort
    let y_padding = if y_max == y_min { 1.0 } else { (y_max - y_min) * 0.05 };
    y_min -= y_padding;
    y_max += y_padding;

    // 3. Initialize Drawing Area
    let root = SVGBackend::new(&args.output, (1200, 800)).into_drawing_area();

    root.fill(&WHITE)?;

    let mut chart = ChartBuilder::on(&root)
        .caption("Multi-profile S-Curve Comparison", ("sans-serif", 24))
        .margin(15)
        .x_label_area_size(50)
        .y_label_area_size(60)
        .build_cartesian_2d(x_min..x_max, y_min..y_max)?;

    chart
        .configure_mesh()
        .x_desc("Time (Cycles)")
        .y_desc("Value")
        .draw()?;

    let num_files = datasets.len();

    // 4. Plot each series with brightness adjustments
    for (i, ds) in datasets.iter().enumerate() {
        // Distribute Hues evenly around the HSL wheel (0.0 to 1.0)
        let hue = i as f64 / num_files as f64;

        // Base saturation
        let saturation = 0.85;

        // Distinct brightness levels for each of the 4 motion attributes
        let pos_color = HSLColor(hue, saturation, 0.25);  // Deep/Dark
        let vel_color = HSLColor(hue, saturation, 0.42);  // Medium-Dark
        let acc_color = HSLColor(hue, saturation, 0.60);  // Bright
        let jrk_color = HSLColor(hue, saturation, 0.78);  // Soft/Light

        // Plot Position
        chart.draw_series(LineSeries::new(
            ds.time_points.iter().cloned().zip(ds.position.iter().cloned()),
            pos_color.stroke_width(2),
        ))?
            .label(format!("Position ({})", ds.filename))
            .legend(move |(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], pos_color.stroke_width(2)));

        // Plot Velocity
        chart.draw_series(LineSeries::new(
            ds.time_points.iter().cloned().zip(ds.velocity.iter().cloned()),
            vel_color.stroke_width(2),
        ))?
            .label(format!("Velocity ({})", ds.filename))
            .legend(move |(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], vel_color.stroke_width(2)));

        // Plot Acceleration
        chart.draw_series(LineSeries::new(
            ds.time_points.iter().cloned().zip(ds.accel.iter().cloned()),
            acc_color.stroke_width(2),
        ))?
            .label(format!("Accel ({})", ds.filename))
            .legend(move |(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], acc_color.stroke_width(2)));

        // Plot Jerk
        chart.draw_series(LineSeries::new(
            ds.time_points.iter().cloned().zip(ds.jerk.iter().cloned()),
            jrk_color.stroke_width(2),
        ))?
            .label(format!("Jerk ({})", ds.filename))
            .legend(move |(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], jrk_color.stroke_width(2)));
    }

    // 5. Render Legend and present image
    chart
        .configure_series_labels()
        .background_style(&WHITE.mix(0.85))
        .border_style(&BLACK)
        .position(SeriesLabelPosition::UpperLeft)
        .draw()?;

    root.present()?;
    println!("Graph rendering complete. Saved output to {:?}", args.output);

    Ok(())
}