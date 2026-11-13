//! `focuscraft` — command line driver.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use focuscraft_core::color::TransferFunction;
use focuscraft_core::io::{self, OutputFormat, OutputSpec};

#[derive(Parser, Debug)]
#[command(
    name = "focuscraft",
    version,
    about = "Focus stacking: merge frames shot at different focal planes",
    long_about = "FocusCraft merges photographs shot at different focal planes into one \
                  image with extended depth of field.\n\n\
                  All processing happens in unclamped scene-linear f32. Input files are \
                  linearised with their own transfer function and output is encoded with \
                  exactly the curve the consumer will decode with."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Report what a file contains and how it will be interpreted.
    Inspect {
        /// Files to inspect.
        #[arg(required = true)]
        files: Vec<PathBuf>,

        /// Force the transfer function used to linearise, instead of the convention
        /// for the file's sample type. One of: linear, srgb, gamma<N>.
        #[arg(long, value_parser = TransferFunction::parse)]
        input_transfer: Option<TransferFunction>,
    },

    /// Convert a file, making the output encoding explicit. Mostly a way to exercise
    /// the colour pipeline end to end before merging exists.
    Convert {
        /// File to read.
        input: PathBuf,

        /// File to write.
        #[arg(short, long)]
        output: PathBuf,

        /// Force the transfer function used to linearise the input.
        #[arg(long, value_parser = TransferFunction::parse)]
        input_transfer: Option<TransferFunction>,

        /// Output container and sample type: tiff16, tiff-f32, png16, png8.
        #[arg(long, default_value = "tiff16", value_parser = OutputFormat::parse)]
        format: OutputFormat,

        /// The curve the consumer will decode the output with. The encoder applies
        /// exactly its inverse. Defaults to the convention for the chosen format.
        #[arg(long, value_parser = TransferFunction::parse)]
        output_transfer: Option<TransferFunction>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Inspect {
            files,
            input_transfer,
        } => inspect(&files, input_transfer),
        Command::Convert {
            input,
            output,
            input_transfer,
            format,
            output_transfer,
        } => convert(&input, &output, input_transfer, format, output_transfer),
    }
}

fn inspect(files: &[PathBuf], input_transfer: Option<TransferFunction>) -> Result<()> {
    for path in files {
        let frame = io::load(path, input_transfer)
            .with_context(|| format!("inspecting {}", path.display()))?;
        let info = &frame.info;
        let stats = frame.image.stats();

        println!("{}", path.display());
        println!(
            "  {}x{}, {}-bit {:?}",
            info.width,
            info.height,
            info.sample.bits(),
            info.sample
        );
        println!(
            "  transfer: {} ({})",
            info.transfer,
            if info.assumed {
                "assumed from sample type"
            } else {
                "set explicitly"
            }
        );
        println!(
            "  scene-linear luma: min {:.5}, mean {:.5}, max {:.5}",
            stats.min_luma, stats.mean_luma, stats.max_luma
        );
        if stats.pixels_above_one > 0 {
            println!(
                "  {} pixels above 1.0 — highlights a clamping pipeline would destroy",
                stats.pixels_above_one
            );
        }
        if stats.non_finite > 0 {
            println!("  WARNING: {} non-finite pixels", stats.non_finite);
        }
        println!();
    }
    Ok(())
}

fn convert(
    input: &PathBuf,
    output: &PathBuf,
    input_transfer: Option<TransferFunction>,
    format: OutputFormat,
    output_transfer: Option<TransferFunction>,
) -> Result<()> {
    let frame = io::load(input, input_transfer)?;
    let spec = OutputSpec::new(
        format,
        output_transfer.unwrap_or_else(|| format.conventional_transfer()),
    );

    let report = io::write(output, &frame.image, spec)?;

    println!(
        "{} -> {} ({:?}, encoded for {})",
        input.display(),
        output.display(),
        spec.format,
        spec.transfer
    );
    if report.clipped_any() {
        println!(
            "  WARNING: clipped {} samples high and {} low (brightest was {:.3} scene-linear)",
            report.clipped_high, report.clipped_low, report.max_linear
        );
        if !spec.format.is_high_dynamic_range() {
            println!("  use --format tiff-f32 to keep values outside 0..1");
        }
    }
    Ok(())
}
