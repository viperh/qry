use clap::{Parser, builder::PossibleValuesParser};

use crate::{
    config::{get_config_dir, get_data_dir},
    theme::{ColorSupport, Theme},
};

#[derive(Parser, Debug)]
#[command(author, version = version(), about)]
pub struct Cli {
    /// Tick rate, i.e. number of ticks per second
    #[arg(short, long, value_name = "FLOAT", default_value_t = 4.0)]
    pub tick_rate: f64,

    /// Frame rate, i.e. number of frames per second
    #[arg(short, long, value_name = "FLOAT", default_value_t = 60.0)]
    pub frame_rate: f64,

    /// Colour theme; overrides the config's "theme"
    #[arg(long, value_name = "NAME", value_parser = PossibleValuesParser::new(Theme::NAMES))]
    pub theme: Option<String>,

    /// Print the config in effect (the defaults with your files over them)
    /// in the config's format, and exit
    #[arg(long)]
    pub print_config: bool,
}

const VERSION_MESSAGE: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    "-",
    env!("VERGEN_GIT_DESCRIBE"),
    " (",
    env!("VERGEN_BUILD_DATE"),
    ")"
);

pub(crate) fn version() -> String {
    let author = clap::crate_authors!();

    // let current_exe_path = PathBuf::from(clap::crate_name!()).display().to_string();
    let config_dir_path = get_config_dir().display().to_string();
    let data_dir_path = get_data_dir().display().to_string();
    let colors = ColorSupport::detect();

    format!(
        "\
{VERSION_MESSAGE}

Authors: {author}

Config directory: {config_dir_path}
Data directory: {data_dir_path}
Colour support: {colors}"
    )
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod cli_tests;
