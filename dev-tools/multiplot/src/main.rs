use std::error::Error;
use std::fs::File;
use std::path::PathBuf;

use clap::Parser;
use eframe::egui;
use egui::Color32;
use egui_plot::{Legend, Line, Plot, PlotPoints};
use plotters::prelude::*;
use serde::Deserialize;

#[derive(Parser, Debug, Clone)]
#[command(author, version, about = "Plot and compare multiple S-Curve motion profiles from CSV files")]
struct Args {
    /// Input CSV files containing motion data
    #[arg(required = true, value_name = "FILES")]
    files: Vec<PathBuf>,

    /// Output SVG image file path. If not specified, displays an interactive GUI window.
    #[arg(short, long, value_name = "OUTPUT")]
    output: Option<PathBuf>,
}

#[derive(Debug, Deserialize, Clone)]
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

#[derive(Clone)]
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

    // 1. Parse all input CSV files
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
            let t = match row.time_cycles {
                Some(t) => t,
                None => {
                    cumulative_cycles += row.interval_cycles;
                    cumulative_cycles
                }
            };
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

    if let Some(output_path) = &args.output {
        // Mode A: Build static SVG plot
        render_to_svg(&datasets, output_path)?;
    } else {
        // Mode B: Interactive egui Desktop Visualizer
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1100.0, 750.0])
                .with_title("S-Curve Interactive Visualizer"),
            ..Default::default()
        };

        eframe::run_native(
            "S-Curve Profile Plotter",
            options,
            Box::new(|_cc| {
                Ok(Box::new(PlotApp {
                    datasets,
                    show_position: true,
                    show_velocity: true,
                    show_accel: true,
                    show_jerk: true,
                }))
            }),
        )
            .map_err(|e| format!("Failed to run GUI: {:?}", e))?;
    }

    Ok(())
}

// ---------------------------------------------------------
// PlotApp: The interactive window state
// ---------------------------------------------------------
struct PlotApp {
    datasets: Vec<ParsedDataset>,
    show_position: bool,
    show_velocity: bool,
    show_accel: bool,
    show_jerk: bool,
}

impl eframe::App for PlotApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("S-Curve Interactive Visualizer");

            // Sidebar-style checkboxes to toggle entire metrics globally
            ui.horizontal(|ui| {
                ui.label("Display filters:");
                ui.checkbox(&mut self.show_position, "Position");
                ui.checkbox(&mut self.show_velocity, "Velocity");
                ui.checkbox(&mut self.show_accel, "Acceleration");
                ui.checkbox(&mut self.show_jerk, "Jerk");
            });

            ui.separator();

            let num_files = self.datasets.len();

            // Construct the egui_plot environment
            let plot = Plot::new("scurve_plot")
                .legend(Legend::default().position(egui_plot::Corner::LeftTop))
                .x_axis_label("Time (Cycles)")
                .y_axis_label("Value");

            plot.show(ui, |plot_ui| {
                for (i, ds) in self.datasets.iter().enumerate() {
                    // Distribute Hues evenly around the HSL wheel (0.0 to 1.0)
                    let hue = i as f32 / num_files as f32;
                    let saturation = 0.85;

                    // Convert HSVA values safely to egui Color32 structures
                    let get_egui_color = |l: f32| {
                        let egui_hsva = egui::ecolor::Hsva::new(hue, saturation, l, 1.0);
                        Color32::from(egui_hsva)
                    };

                    let pos_color = get_egui_color(0.25);
                    let vel_color = get_egui_color(0.45);
                    let acc_color = get_egui_color(0.65);
                    let jrk_color = get_egui_color(0.85);

                    if self.show_position {
                        let pts: PlotPoints = ds
                            .time_points
                            .iter()
                            .zip(ds.position.iter())
                            .map(|(&x, &y)| [x, y])
                            .collect();
                        plot_ui.line(
                            Line::new(pts)
                                .color(pos_color)
                                .width(2.0)
                                .name(format!("Position ({})", ds.filename)),
                        );
                    }

                    if self.show_velocity {
                        let pts: PlotPoints = ds
                            .time_points
                            .iter()
                            .zip(ds.velocity.iter())
                            .map(|(&x, &y)| [x, y])
                            .collect();
                        plot_ui.line(
                            Line::new(pts)
                                .color(vel_color)
                                .width(2.0)
                                .name(format!("Velocity ({})", ds.filename)),
                        );
                    }

                    if self.show_accel {
                        let pts: PlotPoints = ds
                            .time_points
                            .iter()
                            .zip(ds.accel.iter())
                            .map(|(&x, &y)| [x, y])
                            .collect();
                        plot_ui.line(
                            Line::new(pts)
                                .color(acc_color)
                                .width(2.0)
                                .name(format!("Accel ({})", ds.filename)),
                        );
                    }

                    if self.show_jerk {
                        let pts: PlotPoints = ds
                            .time_points
                            .iter()
                            .zip(ds.jerk.iter())
                            .map(|(&x, &y)| [x, y])
                            .collect();
                        plot_ui.line(
                            Line::new(pts)
                                .color(jrk_color)
                                .width(2.0)
                                .name(format!("Jerk ({})", ds.filename)),
                        );
                    }
                }
            });
        });
    }
}

// ---------------------------------------------------------
// Static SVG Renderer
// ---------------------------------------------------------
fn render_to_svg(datasets: &[ParsedDataset], output_path: &PathBuf) -> Result<(), Box<dyn Error>> {
    let mut x_min = f64::INFINITY;
    let mut x_max = f64::NEG_INFINITY;
    let mut y_min = f64::INFINITY;
    let mut y_max = f64::NEG_INFINITY;

    for ds in datasets {
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

    let y_padding = if y_max == y_min { 1.0 } else { (y_max - y_min) * 0.05 };
    y_min -= y_padding;
    y_max += y_padding;

    let root = SVGBackend::new(output_path, (1200, 800)).into_drawing_area();
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

    for (i, ds) in datasets.iter().enumerate() {
        let hue = i as f64 / num_files as f64;
        let saturation = 0.85;

        let pos_color = HSLColor(hue, saturation, 0.25);
        let vel_color = HSLColor(hue, saturation, 0.42);
        let acc_color = HSLColor(hue, saturation, 0.60);
        let jrk_color = HSLColor(hue, saturation, 0.78);

        chart.draw_series(LineSeries::new(
            ds.time_points.iter().cloned().zip(ds.position.iter().cloned()),
            pos_color.stroke_width(2),
        ))?
            .label(format!("Position ({})", ds.filename))
            .legend(move |(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], pos_color.stroke_width(2)));

        chart.draw_series(LineSeries::new(
            ds.time_points.iter().cloned().zip(ds.velocity.iter().cloned()),
            vel_color.stroke_width(2),
        ))?
            .label(format!("Velocity ({})", ds.filename))
            .legend(move |(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], vel_color.stroke_width(2)));

        chart.draw_series(LineSeries::new(
            ds.time_points.iter().cloned().zip(ds.accel.iter().cloned()),
            acc_color.stroke_width(2),
        ))?
            .label(format!("Accel ({})", ds.filename))
            .legend(move |(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], acc_color.stroke_width(2)));

        chart.draw_series(LineSeries::new(
            ds.time_points.iter().cloned().zip(ds.jerk.iter().cloned()),
            jrk_color.stroke_width(2),
        ))?
            .label(format!("Jerk ({})", ds.filename))
            .legend(move |(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], jrk_color.stroke_width(2)));
    }

    chart
        .configure_series_labels()
        .background_style(&WHITE.mix(0.85))
        .border_style(&BLACK)
        .position(SeriesLabelPosition::UpperLeft)
        .draw()?;

    root.present()?;
    println!("Graph rendering complete. Saved output to {:?}", output_path);

    Ok(())
}