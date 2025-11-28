//! Command-line interface definition for runzip.
//!
//! This module defines the CLI structure using `clap` derive macros,
//! providing a familiar interface similar to the standard `unzip` utility.

use clap::Parser;

/// Command-line arguments for the runzip utility.
///
/// This structure defines all available command-line options,
/// mimicking the behavior of the standard Unix `unzip` command
/// while adding support for HTTP URLs.
#[derive(Parser, Debug)]
#[command(name = "runzip")]
#[command(version)]
#[command(about = "A Rust unzip utility with HTTP URL support", long_about = None)]
#[command(after_help = "Examples:\n  \
  runzip data1.zip -x joe        extract all files except joe from data1.zip\n  \
  runzip -p foo.zip | more       send contents of foo.zip via pipe into more\n  \
  runzip -l https://example.com/archive.zip   list files from remote ZIP")]
pub struct Cli {
    /// ZIP file path or HTTP URL.
    ///
    /// Can be either a local filesystem path or an HTTP/HTTPS URL.
    /// When an HTTP URL is provided, the tool uses Range requests
    /// to efficiently access specific parts of the archive.
    #[arg(value_name = "FILE")]
    pub file: String,

    /// Files to extract (default: all).
    ///
    /// Optional list of file patterns to extract from the archive.
    /// Supports substring matching and basic glob patterns (* and ?).
    /// If not specified, all files are extracted.
    #[arg(value_name = "FILES")]
    pub files: Vec<String>,

    /// List files (short format).
    ///
    /// Display the contents of the archive without extracting.
    /// Shows only file names, one per line.
    #[arg(short = 'l')]
    pub list: bool,

    /// List verbosely/show version info.
    ///
    /// Display detailed information about archive contents including
    /// file sizes, compression ratios, and modification timestamps.
    #[arg(short = 'v')]
    pub verbose: bool,

    /// Extract files to pipe, no messages.
    ///
    /// Write extracted file contents directly to stdout.
    /// Useful for piping archive contents to other commands.
    /// Suppresses all informational messages.
    #[arg(short = 'p')]
    pub pipe: bool,

    /// Extract files into exdir.
    ///
    /// Specify a target directory for extraction.
    /// The directory will be created if it doesn't exist.
    #[arg(short = 'd', value_name = "DIR")]
    pub extract_dir: Option<String>,

    /// Exclude files that follow.
    ///
    /// Specify patterns for files to exclude from extraction.
    /// Supports substring matching and basic glob patterns.
    #[arg(short = 'x', value_name = "FILE", num_args = 1..)]
    pub exclude: Vec<String>,

    /// Never overwrite existing files.
    ///
    /// Skip extraction of files that already exist in the target location.
    /// Takes precedence over the `-o` flag.
    #[arg(short = 'n')]
    pub never_overwrite: bool,

    /// Overwrite files WITHOUT prompting.
    ///
    /// Silently overwrite existing files during extraction.
    /// By default, existing files are skipped with a warning.
    #[arg(short = 'o')]
    pub overwrite: bool,

    /// Junk paths (do not make directories).
    ///
    /// Extract all files to the target directory without creating
    /// subdirectories. Only the base filename is used.
    #[arg(short = 'j')]
    pub junk_paths: bool,

    /// Quiet mode (-qq => quieter).
    ///
    /// Suppress informational output. Can be specified multiple times
    /// for increased quietness:
    /// - `-q`: Suppress most messages
    /// - `-qq`: Suppress all messages except errors
    #[arg(short = 'q', action = clap::ArgAction::Count)]
    pub quiet: u8,
}

impl Cli {
    /// Check if the input file is an HTTP/HTTPS URL.
    ///
    /// # Returns
    ///
    /// Returns `true` if the file path starts with "http://" or "https://".
    pub fn is_http_url(&self) -> bool {
        self.file.starts_with("http://") || self.file.starts_with("https://")
    }

    /// Check if quiet mode is enabled.
    ///
    /// Quiet mode is enabled either by the `-q` flag or by pipe mode (`-p`).
    ///
    /// # Returns
    ///
    /// Returns `true` if informational messages should be suppressed.
    pub fn is_quiet(&self) -> bool {
        self.quiet > 0 || self.pipe
    }

    /// Check if very quiet mode is enabled.
    ///
    /// Very quiet mode is enabled when `-q` is specified multiple times.
    ///
    /// # Returns
    ///
    /// Returns `true` if only error messages should be displayed.
    pub fn is_very_quiet(&self) -> bool {
        self.quiet > 1
    }
}
