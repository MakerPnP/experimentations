use std::error::Error;
use std::fs::{self, File};
use std::path::PathBuf;
use std::time::SystemTime;

use clap::Parser;
use eframe::egui;
use egui::Color32;
use egui_dock::{DockArea, DockState, TabViewer};
use egui_plot::{Legend, Line, Plot, PlotPoints};
use plotters::prelude::*;
use serde::Deserialize;

#[derive(Parser, Debug, Clone)]
#[command(author, version, about = "Plot and compare S-Curve motion profiles")]
struct Args {
    /// Input CSV files containing motion data
    #[arg(value_name = "FILES")]
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

struct TrackedFile {
    path: PathBuf,
    dataset: Option<ParsedDataset>,
    error_msg: Option<String>,
    last_modified: Option<SystemTime>,
    last_size: u64,
}

// One single Tab manages a SET of files
struct MotionTab {
    tab_name: String,
    files: Vec<TrackedFile>,
    auto_reload: bool,
}

impl MotionTab {
    fn new(paths: Vec<PathBuf>) -> Self {
        let tab_name = if paths.is_empty() {
            "Empty Tab".to_string()
        } else if paths.len() == 1 {
            paths[0]
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("Dataset")
                .to_string()
        } else {
            let first = paths[0]
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("Dataset");
            format!("{} (+{})", first, paths.len() - 1)
        };

        let mut tab = Self {
            tab_name,
            files: paths
                .into_iter()
                .map(|p| TrackedFile {
                    path: p,
                    dataset: None,
                    error_msg: None,
                    last_modified: None,
                    last_size: 0,
                })
                .collect(),
            auto_reload: true,
        };
        tab.reload_all();
        tab
    }

    fn reload_all(&mut self) {
        for file in &mut self.files {
            match parse_single_csv(&file.path) {
                Ok((dataset, metadata)) => {
                    file.dataset = Some(dataset);
                    if let Some(meta) = metadata {
                        file.last_modified = meta.modified().ok();
                        file.last_size = meta.len();
                    }
                    file.error_msg = None;
                }
                Err(e) => {
                    file.error_msg = Some(e.to_string());
                }
            }
        }
    }

    fn poll_for_changes(&mut self) -> bool {
        if !self.auto_reload {
            return false;
        }

        let mut changed_any = false;
        for file in &mut self.files {
            if let Ok(meta) = fs::metadata(&file.path) {
                let current_mod = meta.modified().ok();
                let current_size = meta.len();

                if current_mod != file.last_modified || current_size != file.last_size {
                    match parse_single_csv(&file.path) {
                        Ok((dataset, metadata)) => {
                            file.dataset = Some(dataset);
                            if let Some(m) = metadata {
                                file.last_modified = m.modified().ok();
                                file.last_size = m.len();
                            }
                            file.error_msg = None;
                        }
                        Err(e) => {
                            file.error_msg = Some(e.to_string());
                        }
                    }
                    changed_any = true;
                }
            }
        }
        changed_any
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let raw_args: Vec<String> = std::env::args().collect();
    if raw_args.len() <= 1 {
        run_interactive_gui(Vec::new())?;
        return Ok(());
    }

    let args = Args::parse();

    if args.output.is_none() {
        run_interactive_gui(args.files)?;
        return Ok(());
    }

    let mut datasets = Vec::new();
    for path in &args.files {
        let (ds, _) = parse_single_csv(path)?;
        datasets.push(ds);
    }
    render_to_svg(&datasets, &args.output.unwrap())?;

    Ok(())
}

fn parse_single_csv(path: &PathBuf) -> Result<(ParsedDataset, Option<fs::Metadata>), Box<dyn Error>> {
    let file = File::open(path)?;
    let metadata = file.metadata().ok();
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

    Ok((
        ParsedDataset {
            filename,
            time_points,
            position,
            velocity,
            accel,
            jerk,
        },
        metadata,
    ))
}

fn run_interactive_gui(initial_files: Vec<PathBuf>) -> Result<(), Box<dyn Error>> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_title("S-Curve Workstation"),
        ..Default::default()
    };

    let initial_tabs = if initial_files.is_empty() {
        Vec::new()
    } else {
        vec![MotionTab::new(initial_files)]
    };

    let dock_state = DockState::new(initial_tabs);

    eframe::run_native(
        "S-Curve Profile Workstation",
        options,
        Box::new(|_cc| {
            Ok(Box::new(PlotApp {
                dock_state,
                show_position: true,
                show_velocity: true,
                show_accel: true,
                show_jerk: true,
            }))
        }),
    )
        .map_err(|e| format!("Failed to run GUI: {:?}", e).into())
}

struct PlotApp {
    dock_state: DockState<MotionTab>,
    show_position: bool,
    show_velocity: bool,
    show_accel: bool,
    show_jerk: bool,
}

struct MainTabViewer {
    show_position: bool,
    show_velocity: bool,
    show_accel: bool,
    show_jerk: bool,
}

impl TabViewer for MainTabViewer {
    type Tab = MotionTab;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        (&tab.tab_name).into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        ui.horizontal(|ui| {
            if ui.button("🔄 Refresh All Set").on_hover_text("Manually reload all files in this tab").clicked() {
                tab.reload_all();
            }

            ui.checkbox(&mut tab.auto_reload, "Auto-reload on changes");

            let newest_time = tab.files.iter()
                .filter_map(|f| f.last_modified)
                .max();

            if let Some(m) = newest_time {
                if let Ok(elapsed) = m.elapsed() {
                    ui.weak(format!("(Updated: {}s ago)", elapsed.as_secs()));
                }
            }
        });

        ui.separator();

        let errors: Vec<String> = tab.files.iter()
            .filter_map(|f| f.error_msg.as_ref().map(|err| format!("{}: {}", f.path.display(), err)))
            .collect();

        if !errors.is_empty() {
            for err in errors {
                ui.colored_label(Color32::RED, err);
            }
            return;
        }

        let parsed_sets: Vec<&ParsedDataset> = tab.files.iter()
            .filter_map(|f| f.dataset.as_ref())
            .collect();

        if parsed_sets.is_empty() {
            ui.label("No active datasets loaded in this tab.");
            return;
        }

        let plot = Plot::new(format!("plot_group_{}", tab.tab_name))
            .legend(Legend::default().position(egui_plot::Corner::LeftTop))
            .x_axis_label("Time (Cycles)")
            .y_axis_label("Value");

        plot.show(ui, |plot_ui| {
            let num_files = parsed_sets.len();

            for (i, ds) in parsed_sets.into_iter().enumerate() {
                let hue = i as f32 / num_files as f32;
                let saturation = 0.85;

                let get_color = |brightness: f32| {
                    let hsva = egui::ecolor::Hsva::new(hue, saturation, brightness, 1.0);
                    Color32::from(hsva)
                };

                let pos_color = get_color(0.35);
                let vel_color = get_color(0.55);
                let acc_color = get_color(0.75);
                let jrk_color = get_color(0.90);

                if self.show_position {
                    let pts: PlotPoints = ds
                        .time_points
                        .iter()
                        .zip(ds.position.iter())
                        .map(|(&x, &y)| [x, y])
                        .collect();
                    // Corrected Line::new signature for 0.34
                    plot_ui.line(
                        Line::new(format!("Position ({})", ds.filename), pts)
                            .color(pos_color)
                            .width(2.0),
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
                        Line::new(format!("Velocity ({})", ds.filename),pts)
                            .color(vel_color)
                            .width(2.0),
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
                        Line::new(format!("Accel ({})", ds.filename), pts)
                            .color(acc_color)
                            .width(2.0),
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
                        Line::new(format!("Jerk ({})", ds.filename), pts)
                            .color(jrk_color)
                            .width(2.0),
                    );
                }
            }
        });
    }
}

impl eframe::App for PlotApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint();

        for tab in self.dock_state.iter_all_tabs_mut() {
            let changed = tab.1.poll_for_changes();
            if changed {
                ctx.request_repaint();
            }
        }

        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("S-Curve Docking Workstation");
                ui.separator();

                if ui.button("📂 Open CSV Set...").on_hover_text("Open a set of CSVs to display inside a single new tab").clicked() {
                    if let Some(files) = rfd::FileDialog::new()
                        .add_filter("CSV Motion Data", &["csv"])
                        .pick_files()
                    {
                        if !files.is_empty() {
                            let new_tab = MotionTab::new(files);
                            self.dock_state.main_surface_mut().push_to_focused_leaf(new_tab);
                        }
                    }
                }

                ui.separator();
                ui.label("Global Filters:");
                ui.checkbox(&mut self.show_position, "Position");
                ui.checkbox(&mut self.show_velocity, "Velocity");
                ui.checkbox(&mut self.show_accel, "Acceleration");
                ui.checkbox(&mut self.show_jerk, "Jerk");
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let mut viewer = MainTabViewer {
                show_position: self.show_position,
                show_velocity: self.show_velocity,
                show_accel: self.show_accel,
                show_jerk: self.show_jerk,
            };

            DockArea::new(&mut self.dock_state)
                .style(egui_dock::Style::from_egui(ui.style().as_ref()))
                .show_inside(ui, &mut viewer);
        });
    }
}

fn render_to_svg(datasets: &[ParsedDataset], output_path: &PathBuf) -> Result<(), Box<dyn Error>> {
    if datasets.is_empty() {
        return Err("No input datasets to render".into());
    }

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