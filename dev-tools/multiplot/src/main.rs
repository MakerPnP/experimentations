use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs::{self, File};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};

use clap::Parser;
use eframe::egui;
use egui::Color32;
use egui_dock::{DockArea, DockState, TabViewer};
use egui_plot::{Legend, Line, Plot, PlotPoints};
use plotters::prelude::*;

#[derive(Parser, Debug, Clone)]
#[command(author, version, about = "Plot and compare multi-column CSV datasets")]
struct Args {
    /// Input CSV files containing motion or profile data
    #[arg(value_name = "FILES")]
    files: Vec<PathBuf>,

    /// Output SVG image file path. If not specified, displays an interactive GUI window.
    #[arg(short, long, value_name = "OUTPUT")]
    output: Option<PathBuf>,
}

#[derive(Clone)]
struct ParsedDataset {
    filename: String,
    /// Header names in insertion order
    headers: Vec<String>,
    /// The mapped independent X-axis label used
    x_header: String,
    /// X-axis coordinates
    x_points: Vec<f64>,
    /// Map of other column headers -> Y values
    columns: HashMap<String, Vec<f64>>,
}

struct TrackedFile {
    path: PathBuf,
    dataset: Option<ParsedDataset>,
    error_msg: Option<String>,
    last_modified: Option<SystemTime>,
    last_size: u64,
}

// One single Tab manages a SET of files and its own plot filters
struct MotionTab {
    tab_name: String,
    files: Vec<TrackedFile>,
    auto_reload: bool,
    /// Toggles for every unique column header found in this tab's files
    active_filters: HashMap<String, bool>,
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
            active_filters: HashMap::new(),
        };
        tab.reload_all();
        tab
    }

    fn reload_all(&mut self) {
        for file in &mut self.files {
            match parse_dynamic_csv(&file.path) {
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
        self.rebuild_filters();
    }

    /// Pulls all column headers from loaded files and initializes them as "enabled" (true) if new.
    fn rebuild_filters(&mut self) {
        let mut all_headers = HashSet::new();
        for file in &self.files {
            if let Some(ds) = &file.dataset {
                for header in &ds.headers {
                    all_headers.insert(header.clone());
                }
            }
        }

        // Clean out filters no longer present, and insert defaults (true) for new ones
        self.active_filters.retain(|k, _| all_headers.contains(k));
        for header in all_headers {
            self.active_filters.entry(header).or_insert(true);
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
                    match parse_dynamic_csv(&file.path) {
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
        if changed_any {
            self.rebuild_filters();
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
        let (ds, _) = parse_dynamic_csv(path)?;
        datasets.push(ds);
    }
    render_to_svg(&datasets, &args.output.unwrap())?;

    Ok(())
}

/// Helper to check if a column is non-decreasing (allowing flat spots, but not completely constant)
fn is_non_decreasing(data: &[f64]) -> bool {
    if data.len() < 2 { return false; }
    // Must change overall from start to end, and each step must be >= the previous
    data.first() < data.last() && data.windows(2).all(|w| w[0] <= w[1])
}

/// Helper to check if a column is non-increasing (allowing flat spots, but not completely constant)
fn is_non_increasing(data: &[f64]) -> bool {
    if data.len() < 2 { return false; }
    // Must change overall from start to end, and each step must be <= the previous
    data.first() > data.last() && data.windows(2).all(|w| w[0] >= w[1])
}

/// Parses a CSV dynamically, detecting the independent X-axis by scanning for a monotonic sequence.
fn parse_dynamic_csv(path: &PathBuf) -> Result<(ParsedDataset, Option<fs::Metadata>), Box<dyn Error>> {
    let file = File::open(path)?;
    let metadata = file.metadata().ok();

    let mut rdr = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .from_reader(file);

    let raw_headers = rdr.headers()?.clone();
    let headers: Vec<String> = raw_headers.iter().map(|s| s.to_string()).collect();

    // 1. Read all rows into memory to analyze trends and populate columns
    let records: Vec<csv::StringRecord> = rdr.into_records().collect::<Result<_, _>>()?;

    if records.is_empty() {
        return Err("CSV file contains no data rows".into());
    }

    // Temporary matrix to parse all columns as float datasets
    let mut parsed_matrix: Vec<Vec<f64>> = vec![Vec::with_capacity(records.len()); headers.len()];

    for record in &records {
        for (pos, _) in headers.iter().enumerate() {
            let val = record.get(pos)
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(0.0);
            parsed_matrix[pos].push(val);
        }
    }

    // 2. Pure Dynamic Detection: Search all columns for the FIRST monotonic column (no English keywords!)
    let mut x_index = None;
    for (pos, col_data) in parsed_matrix.iter().enumerate() {
        if is_non_decreasing(col_data) || is_non_increasing(col_data) {
            x_index = Some(pos);
            break;
        }
    }

    // Fallback: If no column is strictly monotonic, generate a neutral mock timeline
    let (x_header, x_points, final_x_idx) = match x_index {
        Some(idx) => (headers[idx].clone(), parsed_matrix[idx].clone(), Some(idx)),
        None => {
            let index_points: Vec<f64> = (0..records.len()).map(|i| i as f64).collect();
            ("Row Index".to_string(), index_points, None)
        }
    };

    // 3. Separate remaining columns into the Y-axis column map
    let mut columns = HashMap::new();
    let mut filtered_headers = Vec::new();

    for (pos, header) in headers.into_iter().enumerate() {
        if Some(pos) != final_x_idx {
            filtered_headers.push(header.clone());
            columns.insert(header, parsed_matrix[pos].clone());
        }
    }

    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Unknown")
        .to_string();

    Ok((
        ParsedDataset {
            filename,
            headers: filtered_headers,
            x_header,
            x_points,
            columns,
        },
        metadata,
    ))
}
fn run_interactive_gui(initial_files: Vec<PathBuf>) -> Result<(), Box<dyn Error>> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_title("Multi-plot"),
        ..Default::default()
    };

    let initial_tabs = if initial_files.is_empty() {
        Vec::new()
    } else {
        vec![MotionTab::new(initial_files)]
    };

    let dock_state = Arc::new(Mutex::new(DockState::new(initial_tabs)));

    eframe::run_native(
        "Multi-plot",
        options,
        Box::new({
            let dock_state = Arc::clone(&dock_state);
            move |cc| {
                let ctx_clone = cc.egui_ctx.clone();
                let dock_state_clone = Arc::clone(&dock_state);
                thread::spawn(move || loop {
                    thread::sleep(Duration::from_millis(250));
                    let mut needs_repaint = false;

                    if let Ok(mut state) = dock_state_clone.lock() {
                        for tab in state.iter_all_tabs_mut() {
                            if tab.1.poll_for_changes() {
                                needs_repaint = true;
                            }
                        }
                    }

                    if needs_repaint {
                        ctx_clone.request_repaint();
                    }
                });

                Ok(Box::new(PlotApp { dock_state }))
            }
        }),
    )
        .map_err(|e| format!("Failed to run GUI: {:?}", e).into())
}

struct PlotApp {
    dock_state: Arc<Mutex<DockState<MotionTab>>>,
}

struct MainTabViewer;

impl TabViewer for MainTabViewer {
    type Tab = MotionTab;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        (&tab.tab_name).into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        ui.horizontal(|ui| {
            if ui.button("🔄 Refresh All Set").clicked() {
                tab.reload_all();
            }

            ui.checkbox(&mut tab.auto_reload, "Auto-reload");

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

        // 1. Draw localized Combined Tab Checkboxes
        ui.horizontal_wrapped(|ui| {
            ui.label("Display Columns:");

            // Sort column list alphabetically by cloning the keys to avoid borrow conflicts
            let mut sorted_filters: Vec<String> = tab.active_filters.keys().cloned().collect();
            sorted_filters.sort();

            for header in &sorted_filters {
                if let Some(checked) = tab.active_filters.get_mut(header) {
                    ui.checkbox(checked, header);
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

        // Determine unified X-Axis label representing this dataset's main timeline coordinate
        let x_axis_label = parsed_sets.first().map(|d| d.x_header.as_str()).unwrap_or("X Axis");

        let plot = Plot::new(format!("plot_group_{}", tab.tab_name))
            .legend(Legend::default().position(egui_plot::Corner::LeftTop))
            .x_axis_label(x_axis_label)
            .y_axis_label("Value");

        plot.show(ui, |plot_ui| {
            let num_files = parsed_sets.len();

            for (file_idx, ds) in parsed_sets.into_iter().enumerate() {
                // Hue base shifts per file inside this tab so lines are visually separate
                let hue = file_idx as f32 / num_files as f32;
                let saturation = 0.85;

                // Unique brightness steps for different columns in a file
                let num_columns = ds.headers.len();

                for (col_idx, header) in ds.headers.iter().enumerate() {
                    // Check if this combined checkbox is enabled for this tab
                    if let Some(&true) = tab.active_filters.get(header) {
                        if let Some(y_points) = ds.columns.get(header) {
                            let pts: PlotPoints = ds
                                .x_points
                                .iter()
                                .zip(y_points.iter())
                                .map(|(&x, &y)| [x, y])
                                .collect();

                            // Spread brightness/value depending on the column index
                            let val_brightness = 0.35 + (col_idx as f32 / num_columns.max(1) as f32) * 0.55;
                            let hsva = egui::ecolor::Hsva::new(hue, saturation, val_brightness, 1.0);
                            let color = Color32::from(hsva);

                            plot_ui.line(
                                Line::new(format!("{} ({})", header, ds.filename), pts)
                                    .color(color)
                                    .width(2.0),
                            );
                        }
                    }
                }
            }
        });
    }
}

impl eframe::App for PlotApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut dock_state = self.dock_state.lock().unwrap();

        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Multi-plot");
                ui.separator();

                if ui.button("📂 Open CSV Set...").on_hover_text("Open a set of CSVs to display inside a single new tab").clicked() {
                    if let Some(files) = rfd::FileDialog::new()
                        .add_filter("CSV Data", &["csv"])
                        .pick_files()
                    {
                        if !files.is_empty() {
                            let new_tab = MotionTab::new(files);
                            dock_state.main_surface_mut().push_to_focused_leaf(new_tab);
                        }
                    }
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let mut viewer = MainTabViewer;

            DockArea::new(&mut *dock_state)
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
        for &x in &ds.x_points {
            if x < x_min { x_min = x; }
            if x > x_max { x_max = x; }
        }
        for vec in ds.columns.values() {
            for &y in vec {
                if y < y_min { y_min = y; }
                if y > y_max { y_max = y; }
            }
        }
    }

    let y_padding = if y_max == y_min { 1.0 } else { (y_max - y_min) * 0.05 };
    y_min -= y_padding;
    y_max += y_padding;

    let root = SVGBackend::new(output_path, (1200, 800)).into_drawing_area();
    root.fill(&WHITE)?;

    let x_axis_label = datasets.first().map(|d| d.x_header.as_str()).unwrap_or("X Axis");

    let mut chart = ChartBuilder::on(&root)
        .caption("CSV Comparison", ("sans-serif", 24))
        .margin(15)
        .x_label_area_size(50)
        .y_label_area_size(60)
        .build_cartesian_2d(x_min..x_max, y_min..y_max)?;

    chart
        .configure_mesh()
        .x_desc(x_axis_label)
        .y_desc("Value")
        .draw()?;

    let num_files = datasets.len();

    for (file_idx, ds) in datasets.iter().enumerate() {
        let hue = file_idx as f64 / num_files as f64;
        let saturation = 0.85;
        let num_columns = ds.headers.len();

        for (col_idx, header) in ds.headers.iter().enumerate() {
            if let Some(y_points) = ds.columns.get(header) {
                let val_brightness = 0.25 + (col_idx as f64 / num_columns.max(1) as f64) * 0.55;
                let color = HSLColor(hue, saturation, val_brightness);

                chart.draw_series(LineSeries::new(
                    ds.x_points.iter().cloned().zip(y_points.iter().cloned()),
                    color.stroke_width(2),
                ))?
                    .label(format!("{} ({})", header, ds.filename))
                    .legend(move |(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], color.stroke_width(2)));
            }
        }
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