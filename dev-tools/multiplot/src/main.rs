use std::collections::HashMap;
use std::error::Error;
use std::fs::{self, File};
use std::io::Write;
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
use serde::{Deserialize, Serialize};

#[derive(Parser, Debug, Clone)]
#[command(name = "Multi-plot", author, version, about = "Plot and compare multi-column CSV datasets")]
struct Args {
    /// Input CSV files containing motion or profile data (legacy standalone mode)
    #[arg(value_name = "FILES")]
    files: Vec<PathBuf>,

    /// Set configuration JSON file paths (.json) to load as standalone tab sets
    #[arg(short, long = "set", value_name = "SET_CONFIG")]
    sets: Vec<PathBuf>,

    /// Column names to completely HIDE / FILTER out initially from command line sets
    #[arg(short, long = "filter", value_name = "COLUMN_NAME")]
    filters: Vec<String>,

    /// Output SVG image file path. If specified, saves the render and exits immediately.
    #[arg(short, long, value_name = "OUTPUT")]
    output: Option<PathBuf>,
}

/// Serializable payload representing a unique tab configuration
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct SetConfiguration {
    /// Paths to individual CSV files tracked in this set
    pub files: Vec<PathBuf>,
    /// Header columns that are currently unchecked/inactive
    pub filtered_columns: Vec<String>,
}

/// Global tracking file layout used to reload application state between restarts
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct AppSessionState {
    /// Ordered list of active tab set configurations
    pub sets: Vec<SetConfiguration>,
    /// File tracking associations for which tab was saved under what configuration path
    pub config_paths: Vec<Option<PathBuf>>,
}

#[derive(Clone)]
struct ParsedDataset {
    filename: String,
    headers: Vec<String>,
    x_header: String,
    x_points: Vec<f64>,
    columns: HashMap<String, Vec<f64>>,
}

struct TrackedFile {
    path: PathBuf,
    dataset: Option<ParsedDataset>,
    error_msg: Option<String>,
    last_modified: Option<SystemTime>,
    last_size: u64,
}

struct MotionTab {
    tab_name: String,
    files: Vec<TrackedFile>,
    auto_reload: bool,
    active_filters: HashMap<String, bool>,
    /// Path pointing to the underlying dedicated .json tab profile if one exists
    associated_config_path: Option<PathBuf>,
}

impl MotionTab {
    fn from_config(config: SetConfiguration, path: Option<PathBuf>) -> Self {
        let mut tab = Self::new(config.files);
        tab.associated_config_path = path;

        // Apply historical filtered options by flipping matched flags to false
        for target in config.filtered_columns {
            tab.active_filters.insert(target, false);
        }
        tab
    }

    fn to_config(&self) -> SetConfiguration {
        let files = self.files.iter().map(|f| f.path.clone()).collect();
        let filtered_columns = self.active_filters.iter()
            .filter(|&(_, &visible)| !visible)
            .map(|(name, _)| name.clone())
            .collect();

        SetConfiguration { files, filtered_columns }
    }

    fn new(paths: Vec<PathBuf>) -> Self {
        let tab_name = if paths.is_empty() {
            "Empty Tab".to_string()
        } else if paths.len() == 1 {
            paths[0].file_name().and_then(|n| n.to_str()).unwrap_or("Dataset").to_string()
        } else {
            let first = paths[0].file_name().and_then(|n| n.to_str()).unwrap_or("Dataset");
            format!("{} (+{})", first, paths.len() - 1)
        };

        let mut tab = Self {
            tab_name,
            files: paths.into_iter().map(|p| TrackedFile {
                path: p,
                dataset: None,
                error_msg: None,
                last_modified: None,
                last_size: 0,
            }).collect(),
            auto_reload: true,
            active_filters: HashMap::new(),
            associated_config_path: None,
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

    fn rebuild_filters(&mut self) {
        for file in &self.files {
            if let Some(ds) = &file.dataset {
                for header in &ds.headers {
                    self.active_filters.entry(header.clone()).or_insert(true);
                }
            }
        }
    }

    fn poll_for_changes(&mut self) -> bool {
        if !self.auto_reload { return false; }
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
        if changed_any { self.rebuild_filters(); }
        changed_any
    }
}

fn get_session_store_path() -> PathBuf {
    std::env::current_exe()
        .map(|p| p.with_file_name("multiplot_session.json"))
        .unwrap_or_else(|_| PathBuf::from("multiplot_session.json"))
}

fn load_session_state() -> Option<AppSessionState> {
    let path = get_session_store_path();
    let file = File::open(path).ok()?;
    serde_json::from_reader(file).ok()
}

fn save_session_state(state: &AppSessionState) {
    let path = get_session_store_path();
    if let Ok(mut file) = File::create(path) {
        if let Ok(json) = serde_json::to_string_pretty(state) {
            let _ = file.write_all(json.as_bytes());
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    let is_cli_driven = !args.files.is_empty() || !args.sets.is_empty() || args.output.is_some();

    // Process output SVG layout directly if specified on command line
    if let Some(out_path) = &args.output {
        let mut datasets = Vec::new();
        // Load legacy files
        for p in &args.files {
            datasets.push(parse_dynamic_csv(p)?.0);
        }
        // Load set profiles
        for sp in &args.sets {
            let f = File::open(sp)?;
            let config: SetConfiguration = serde_json::from_reader(f)?;
            for p in &config.files {
                datasets.push(parse_dynamic_csv(p)?.0);
            }
        }
        render_to_svg(&datasets, out_path)?;
        return Ok(());
    }

    run_interactive_gui(args, is_cli_driven)?;
    Ok(())
}

fn is_non_decreasing(data: &[f64]) -> bool {
    if data.len() < 2 { return false; }
    data.first() < data.last() && data.windows(2).all(|w| w[0] <= w[1])
}

fn is_non_increasing(data: &[f64]) -> bool {
    if data.len() < 2 { return false; }
    data.first() > data.last() && data.windows(2).all(|w| w[0] >= w[1])
}

fn parse_dynamic_csv(path: &PathBuf) -> Result<(ParsedDataset, Option<fs::Metadata>), Box<dyn Error>> {
    let file = File::open(path)?;
    let metadata = file.metadata().ok();
    let mut rdr = csv::ReaderBuilder::new().trim(csv::Trim::All).from_reader(file);

    let raw_headers = rdr.headers()?.clone();
    let headers: Vec<String> = raw_headers.iter().map(|s| s.to_string()).collect();
    let records: Vec<csv::StringRecord> = rdr.into_records().collect::<Result<_, _>>()?;
    if records.is_empty() { return Err("CSV contains no data".into()); }

    let mut parsed_matrix: Vec<Vec<f64>> = vec![Vec::with_capacity(records.len()); headers.len()];
    for record in &records {
        for (pos, _) in headers.iter().enumerate() {
            let val = record.get(pos).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
            parsed_matrix[pos].push(val);
        }
    }

    let mut x_index = None;
    for (pos, col_data) in parsed_matrix.iter().enumerate() {
        if is_non_decreasing(col_data) || is_non_increasing(col_data) {
            x_index = Some(pos);
            break;
        }
    }

    let (x_header, x_points, final_x_idx) = match x_index {
        Some(idx) => (headers[idx].clone(), parsed_matrix[idx].clone(), Some(idx)),
        None => {
            let index_points: Vec<f64> = (0..records.len()).map(|i| i as f64).collect();
            ("Row Index".to_string(), index_points, None)
        }
    };

    let mut columns = HashMap::new();
    let mut filtered_headers = Vec::new();
    for (pos, header) in headers.into_iter().enumerate() {
        if Some(pos) != final_x_idx {
            filtered_headers.push(header.clone());
            columns.insert(header, parsed_matrix[pos].clone());
        }
    }

    let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("Unknown").to_string();
    Ok((ParsedDataset { filename, headers: filtered_headers, x_header, x_points, columns }, metadata))
}

fn run_interactive_gui(args: Args, is_cli_driven: bool) -> Result<(), Box<dyn Error>> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1200.0, 800.0]).with_title("Multi-plot"),
        ..Default::default()
    };

    let mut initial_tabs = Vec::new();

    if is_cli_driven {
        // Mode A: Overridden explicit arguments supplied via CLI
        if !args.files.is_empty() {
            let mut tab = MotionTab::new(args.files.clone());
            for f_name in &args.filters {
                tab.active_filters.insert(f_name.clone(), false);
            }
            initial_tabs.push(tab);
        }
        for set_config_path in &args.sets {
            if let Ok(f) = File::open(set_config_path) {
                if let Ok(config) = serde_json::from_reader::<_, SetConfiguration>(f) {
                    let mut tab = MotionTab::from_config(config, Some(set_config_path.clone()));
                    for f_name in &args.filters {
                        tab.active_filters.insert(f_name.clone(), false);
                    }
                    initial_tabs.push(tab);
                }
            }
        }
    } else if let Some(session) = load_session_state() {
        // Mode B: Seamless zero-arguments historical resume
        for (cfg, path) in session.sets.into_iter().zip(session.config_paths.into_iter()) {
            initial_tabs.push(MotionTab::from_config(cfg, path));
        }
    }

    let dock_state = DockState::new(initial_tabs);
    let dock_state_shared = Arc::new(Mutex::new(dock_state));

    eframe::run_native(
        "Multi-plot",
        options,
        Box::new({
            let dock_state = Arc::clone(&dock_state_shared);
            move |cc| {
                let ctx_clone = cc.egui_ctx.clone();
                let dock_state_clone = Arc::clone(&dock_state);
                thread::spawn(move || loop {
                    thread::sleep(Duration::from_millis(250));
                    let mut needs_repaint = false;
                    if let Ok(mut state) = dock_state_clone.lock() {
                        for tab in state.iter_all_tabs_mut() {
                            if tab.1.poll_for_changes() { needs_repaint = true; }
                        }
                    }
                    if needs_repaint { ctx_clone.request_repaint(); }
                });

                Ok(Box::new(PlotApp {
                    dock_state,
                    is_cli_driven,
                }))
            }
        }),
    )
        .map_err(|e| format!("Failed to run GUI: {:?}", e).into())
}

struct PlotApp {
    dock_state: Arc<Mutex<DockState<MotionTab>>>,
    is_cli_driven: bool,
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

            ui.separator();

            // Save Set Button Implementation
            let save_label = if let Some(path) = &tab.associated_config_path {
                format!("💾 Save Set ({})", path.file_name().unwrap().to_string_lossy())
            } else {
                "💾 Save Set Config...".to_string()
            };

            if ui.button(save_label).clicked() {
                let target_path = if let Some(path) = &tab.associated_config_path {
                    Some(path.clone())
                } else {
                    rfd::FileDialog::new()
                        .add_filter("Configuration File", &["json"])
                        .set_file_name("set_config.json")
                        .save_file()
                };

                if let Some(path) = target_path {
                    let config = tab.to_config();
                    if let Ok(mut file) = File::create(&path) {
                        if let Ok(json) = serde_json::to_string_pretty(&config) {
                            let _ = file.write_all(json.as_bytes());
                            tab.associated_config_path = Some(path);
                        }
                    }
                }
            }

            let newest_time = tab.files.iter().filter_map(|f| f.last_modified).max();
            if let Some(m) = newest_time {
                if let Ok(elapsed) = m.elapsed() {
                    ui.weak(format!("(Updated: {}s ago)", elapsed.as_secs()));
                }
            }
        });

        ui.separator();

        ui.horizontal_wrapped(|ui| {
            ui.label("Display Columns:");
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
            for err in errors { ui.colored_label(Color32::RED, err); }
            return;
        }

        let parsed_sets: Vec<&ParsedDataset> = tab.files.iter().filter_map(|f| f.dataset.as_ref()).collect();
        if parsed_sets.is_empty() {
            ui.label("No active datasets loaded in this tab.");
            return;
        }

        let x_axis_label = parsed_sets.first().map(|d| d.x_header.as_str()).unwrap_or("X Axis");
        let plot = Plot::new(format!("plot_group_{}", tab.tab_name))
            .legend(Legend::default().position(egui_plot::Corner::LeftTop))
            .x_axis_label(x_axis_label)
            .y_axis_label("Value");

        plot.show(ui, |plot_ui| {
            let num_files = parsed_sets.len();
            for (file_idx, ds) in parsed_sets.into_iter().enumerate() {
                let hue = file_idx as f32 / num_files as f32;
                let saturation = 0.85;
                let num_columns = ds.headers.len();

                for (col_idx, header) in ds.headers.iter().enumerate() {
                    if let Some(&true) = tab.active_filters.get(header) {
                        if let Some(y_points) = ds.columns.get(header) {
                            let pts: PlotPoints = ds.x_points.iter().zip(y_points.iter())
                                .map(|(&x, &y)| [x, y]).collect();

                            let val_brightness = 0.35 + (col_idx as f32 / num_columns.max(1) as f32) * 0.55;
                            let hsva = egui::ecolor::Hsva::new(hue, saturation, val_brightness, 1.0);
                            let color = Color32::from(hsva);

                            plot_ui.line(Line::new(format!("{} ({})", header, ds.filename), pts).color(color).width(2.0));
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

                if ui.button("📂 Open CSV Set...").clicked() {
                    if let Some(files) = rfd::FileDialog::new().add_filter("CSV Data", &["csv"]).pick_files() {
                        if !files.is_empty() {
                            let new_tab = MotionTab::new(files);
                            dock_state.main_surface_mut().push_to_focused_leaf(new_tab);
                        }
                    }
                }

                if ui.button("📂 Open Saved Set Profile...").clicked() {
                    if let Some(path) = rfd::FileDialog::new().add_filter("Config JSON", &["json"]).pick_file() {
                        if let Ok(f) = File::open(&path) {
                            if let Ok(config) = serde_json::from_reader::<_, SetConfiguration>(f) {
                                let new_tab = MotionTab::from_config(config, Some(path));
                                dock_state.main_surface_mut().push_to_focused_leaf(new_tab);
                            }
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

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Only update persistent global fallback configuration if the session was launched blindly without args
        if !self.is_cli_driven {
            let dock_state = self.dock_state.lock().unwrap();
            let mut sets = Vec::new();
            let mut config_paths = Vec::new();

            for (_, tab) in dock_state.iter_all_tabs() {
                sets.push(tab.to_config());
                config_paths.push(tab.associated_config_path.clone());
            }

            let session = AppSessionState {
                sets,
                config_paths,
            };
            save_session_state(&session);
        }
    }
}

fn render_to_svg(datasets: &[ParsedDataset], output_path: &PathBuf) -> Result<(), Box<dyn Error>> {
    if datasets.is_empty() { return Err("No input datasets to render".into()); }
    let mut x_min = f64::INFINITY; let mut x_max = f64::NEG_INFINITY;
    let mut y_min = f64::INFINITY; let mut y_max = f64::NEG_INFINITY;

    for ds in datasets {
        for &x in &ds.x_points {
            if x < x_min { x_min = x; } if x > x_max { x_max = x; }
        }
        for vec in ds.columns.values() {
            for &y in vec {
                if y < y_min { y_min = y; } if y > y_max { y_max = y; }
            }
        }
    }

    let y_padding = if y_max == y_min { 1.0 } else { (y_max - y_min) * 0.05 };
    y_min -= y_padding; y_max += y_padding;

    let root = SVGBackend::new(output_path, (1200, 800)).into_drawing_area();
    root.fill(&WHITE)?;

    let x_axis_label = datasets.first().map(|d| d.x_header.as_str()).unwrap_or("X Axis");
    let mut chart = ChartBuilder::on(&root)
        .caption("Multi-profile CSV Comparison", ("sans-serif", 24)).margin(15)
        .x_label_area_size(50).y_label_area_size(60)
        .build_cartesian_2d(x_min..x_max, y_min..y_max)?;

    chart.configure_mesh().x_desc(x_axis_label).y_desc("Value").draw()?;
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

    chart.configure_series_labels().background_style(&WHITE.mix(0.85)).border_style(&BLACK)
        .position(SeriesLabelPosition::UpperLeft).draw()?;

    root.present()?;
    Ok(())
}