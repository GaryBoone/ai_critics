use plotters::coord::types::{RangedCoordf32, RangedCoordu32};
use plotters::{define_color, doc, prelude::*};
use polars::datatypes::{DataType, Field};
use polars::lazy::dsl::col;
use polars::prelude::*;
use std::env;
use std::error::Error;
use std::process::Command;

const SYMBOL_SIZE: i32 = 5;
const OUTPUT_FILENAME: &str = "plot.png";

define_color!(DARK_ORANGE, 255, 140, 0, "DarkOrange");
define_color!(DARK_GREEN, 0, 100, 0, "DarkGreen");
define_color!(DARK_BLUE, 0, 0, 139, "DarkBlue");
define_color!(PURPLE, 128, 0, 128, "Purple");

fn read_data() -> Result<DataFrame, PolarsError> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <filename>", args[0]);
        std::process::exit(1);
    }
    let schema = Schema::from_iter(vec![
        Field::new("Problem", DataType::UInt32),
        Field::new("NumCritics", DataType::UInt32),
        Field::new("SuccessCount", DataType::UInt32),
        Field::new("FailureCount", DataType::UInt32),
        Field::new("DivergenceCount", DataType::UInt32),
        Field::new("SuccessIterations", DataType::UInt32),
    ]);
    let df = CsvReader::from_path(&args[1])?
        .with_schema(Some(Arc::new(schema)))
        .has_header(true)
        .finish()?;
    Ok(df)
}

fn process_data(df: DataFrame) -> Result<DataFrame, PolarsError> {
    let df = df
        .lazy()
        .with_column(
            (col("SuccessIterations").cast(DataType::Float64)
                / col("SuccessCount").cast(DataType::Float64))
            .alias("AvgIterations"),
        )
        .collect()?;
    let lf = df
        .lazy()
        .with_column(col("Problem").cast(DataType::UInt32))
        .with_column(col("NumCritics").cast(DataType::UInt32))
        .with_column(col("SuccessIterations").cast(DataType::UInt32))
        .collect()?;
    Ok(lf)
}

fn triangle_shape(color: &RGBColor) -> Polygon<(i32, i32)> {
    Polygon::new(
        vec![
            (0, SYMBOL_SIZE),
            (SYMBOL_SIZE, -SYMBOL_SIZE),
            (-SYMBOL_SIZE, -SYMBOL_SIZE),
        ],
        ShapeStyle::from(color).filled(),
    )
}
fn square_shape(color: &RGBColor) -> Polygon<(i32, i32)> {
    Polygon::new(
        vec![
            (-SYMBOL_SIZE, SYMBOL_SIZE),
            (SYMBOL_SIZE, SYMBOL_SIZE),
            (SYMBOL_SIZE, -SYMBOL_SIZE),
            (-SYMBOL_SIZE, -SYMBOL_SIZE),
        ],
        ShapeStyle::from(color).filled(),
    )
}

fn diamond_shape(color: &RGBColor) -> Polygon<(i32, i32)> {
    Polygon::new(
        vec![
            (-SYMBOL_SIZE, 0),
            (0, SYMBOL_SIZE),
            (SYMBOL_SIZE, 0),
            (0, -SYMBOL_SIZE),
        ],
        ShapeStyle::from(color).filled(),
    )
}

fn create_series(problem: u32, lf: &DataFrame) -> Result<Vec<(u32, f64)>, Box<dyn Error>> {
    let mask_expr = col("Problem").eq(lit(problem));
    let filtered_data = lf.clone().lazy().filter(mask_expr).collect()?;
    let critics_data: Vec<u32> = filtered_data
        .column("NumCritics")?
        .u32()?
        .into_no_null_iter()
        .collect();
    let avg_iterations: Vec<f64> = filtered_data
        .column("AvgIterations")?
        .f64()?
        .into_no_null_iter()
        .collect();
    let line_data: Vec<(u32, f64)> = critics_data.into_iter().zip(avg_iterations).collect();
    Ok(line_data)
}

fn add_problem_to_plot(
    problem: u32,
    lf: &DataFrame,
    chart: &mut ChartContext<'_, BitMapBackend<'_>, Cartesian2d<RangedCoordu32, RangedCoordf32>>,
) -> Result<(), Box<dyn Error>> {
    let line_data = create_series(problem, lf)?;
    let colors = [
        &BLACK,
        &RED,
        &DARK_GREEN,
        &DARK_BLUE,
        &PURPLE,
        &MAGENTA,
        &DARK_ORANGE,
    ];
    let color = *colors[problem as usize % colors.len()];
    chart
        .draw_series(LineSeries::new(
            line_data.iter().map(|&(x, y)| (x, y as f32)),
            color,
        ))?
        .label(format!("Problem {}", problem))
        .legend(move |(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], color));
    chart.draw_series(line_data.iter().map(|&(x, y)| {
        let shape = match problem % 3 {
            0 => triangle_shape(&color),
            1 => square_shape(&color),
            _ => diamond_shape(&color),
        };
        EmptyElement::at((x, y as f32)) + shape
    }))?;
    Ok(())
}

fn create_plot(lf: DataFrame) -> Result<(), Box<dyn Error>> {
    let root = BitMapBackend::new(OUTPUT_FILENAME, (1024, 768)).into_drawing_area();
    root.fill(&WHITE)?;
    let mut chart = ChartBuilder::on(&root)
        .caption(
            "Iterations Required vs Number of Critics",
            ("sans-serif", 40).into_font(),
        )
        .margin(10)
        .x_label_area_size(30)
        .y_label_area_size(30)
        .build_cartesian_2d(0u32..6u32, 0f32..12f32)?;
    chart.configure_mesh().draw()?;

    let unique_problems: Vec<u32> = lf
        .column("Problem")?
        .unique()?
        .u32()?
        .into_no_null_iter()
        .collect();
    for &problem in unique_problems.iter() {
        add_problem_to_plot(problem, &lf, &mut chart)?;
    }
    chart
        .configure_series_labels()
        .background_style(WHITE.mix(0.8))
        .border_style(BLACK)
        .draw()?;
    drop(chart);
    root.present()?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let df = read_data()?;

    let lf = process_data(df)?;

    create_plot(lf)?;

    // Display the result.
    if cfg!(target_os = "macos") {
        Command::new("open")
            .arg(OUTPUT_FILENAME)
            .spawn()
            .expect("Failed to open image");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mock data for testing
    fn mock_data_frame() -> DataFrame {
        let s0 = Series::new("Problem", &[1, 2, 1, 2]);
        let s1 = Series::new("NumCritics", &[3, 4, 3, 4]);
        let s2 = Series::new("SuccessCount", &[2, 2, 2, 2]);
        let s3 = Series::new("FailureCount", &[0, 0, 1, 0]);
        let s4 = Series::new("DivergenceCount", &[0, 1, 0, 0]);
        let s5 = Series::new("SuccessIterations", &[10, 20, 10, 20]);
        DataFrame::new(vec![s0, s1, s2, s3, s4, s5]).expect("Failed to create DataFrame")
    }

    #[test]
    fn test_process_data() {
        let df = mock_data_frame();
        let result = process_data(df).expect("Failed to process data");
        assert!(result.column("Problem").is_ok());
        assert!(result.column("NumCritics").is_ok());
        assert!(result.column("SuccessCount").is_ok());
        assert!(result.column("FailureCount").is_ok());
        assert!(result.column("DivergenceCount").is_ok());
        assert!(result.column("AvgIterations").is_ok());
    }

    #[test]
    fn test_create_plot() {
        let df = mock_data_frame();
        let lf = process_data(df).expect("Failed to process data");
        let result = create_plot(lf);
        assert!(result.is_ok());
    }
}
