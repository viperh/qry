use std::io::{self, Write};

use clap::{CommandFactory, Parser, Subcommand, builder::PossibleValuesParser};
use clap_complete::Shell;

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

    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Subcommands. Each one runs instead of the TUI, before the terminal is
/// touched.
#[derive(Subcommand, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Write a release asset (the man page or a completion script) to stdout
    #[command(hide = true)]
    Generate {
        #[command(subcommand)]
        asset: Asset,
    },
}

/// What `qry generate` writes.
#[derive(Subcommand, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asset {
    /// The man page, qry.1, in roff
    Man,
    /// A completion script for the given shell
    Completions {
        #[arg(value_enum)]
        shell: Shell,
    },
}

/// The man page's ENVIRONMENT section, in roff. clap_mangen has no section
/// for environment variables, so it goes in by hand between OPTIONS and
/// VERSION.
const MAN_ENVIRONMENT: &str = r".SH ENVIRONMENT
.TP
\fBQRY_CONFIG\fR
Directory to read config files from, instead of the per\-user config directory.
.TP
\fBQRY_DATA\fR
Directory for the log file and other data, instead of the per\-user data directory.
.TP
\fBQRY_LOG_LEVEL\fR
Log filter, such as \fBdebug\fR or \fBqry=trace\fR.
\fBRUST_LOG\fR takes precedence when set.
.TP
\fBQRY_KEYRING\fR
Set to \fBoff\fR to keep passwords in memory for the session instead of in the OS keychain.
.TP
\fBNO_COLOR\fR
When set to a non\-empty value, draw without colour.
.TP
\fBCOLORTERM\fR
\fBtruecolor\fR or \fB24bit\fR selects 24\-bit colour; otherwise the theme falls back to 256 colours.
";

/// The command line as the release assets describe it: the user-facing
/// flags only, without the hidden `generate` subcommand (which would put
/// `[subcommands]` in the man page's synopsis and in the completions).
///
/// The version is the plain package version: `--version`'s text names the
/// build machine's directories, which have no place in a man page and would
/// make the assets differ between machines.
fn asset_command() -> clap::Command {
    let cli = Cli::command();
    let mut cmd = clap::Command::new(cli.get_name().to_owned()).version(env!("CARGO_PKG_VERSION"));
    if let Some(about) = cli.get_about() {
        cmd = cmd.about(about.clone());
    }
    if let Some(long_about) = cli.get_long_about() {
        cmd = cmd.long_about(long_about.clone());
    }
    if let Some(author) = cli.get_author() {
        cmd = cmd.author(author.to_owned());
    }
    cli.get_arguments()
        .fold(cmd, |cmd, arg| cmd.arg(arg.clone()))
}

/// Writes `asset` to `out`. Runs before the TUI starts and never touches the
/// terminal.
pub fn generate(asset: Asset, out: &mut dyn Write) -> io::Result<()> {
    let mut cmd = asset_command();
    match asset {
        Asset::Man => {
            let man = clap_mangen::Man::new(cmd);
            man.render_title(out)?;
            man.render_name_section(out)?;
            man.render_synopsis_section(out)?;
            man.render_description_section(out)?;
            man.render_options_section(out)?;
            out.write_all(MAN_ENVIRONMENT.as_bytes())?;
            man.render_version_section(out)?;
            man.render_authors_section(out)?;
        }
        Asset::Completions { shell } => {
            let name = cmd.get_name().to_owned();
            clap_complete::generate(shell, &mut cmd, name, out);
        }
    }
    out.flush()
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
