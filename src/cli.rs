use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "runzip")]
#[command(version)]
#[command(about = "A Rust unzip utility with HTTP URL support", long_about = None)]
#[command(after_help = "Examples:\n  \
  runzip data1.zip -x joe        extract all files except joe from data1.zip\n  \
  runzip -p foo.zip | more       send contents of foo.zip via pipe into more\n  \
  runzip -l https://example.com/archive.zip   list files from remote ZIP")]
pub struct Cli {
    /// ZIP file path or HTTP URL
    #[arg(value_name = "FILE")]
    pub file: String,

    /// Files to extract (default: all)
    #[arg(value_name = "FILES")]
    pub files: Vec<String>,

    /// List files (short format)
    #[arg(short = 'l')]
    pub list: bool,

    /// List verbosely/show version info
    #[arg(short = 'v')]
    pub verbose: bool,

    /// Extract files to pipe, no messages
    #[arg(short = 'p')]
    pub pipe: bool,

    /// Extract files into exdir
    #[arg(short = 'd', value_name = "DIR")]
    pub extract_dir: Option<String>,

    /// Exclude files that follow
    #[arg(short = 'x', value_name = "FILE", num_args = 1..)]
    pub exclude: Vec<String>,

    /// Never overwrite existing files
    #[arg(short = 'n')]
    pub never_overwrite: bool,

    /// Overwrite files WITHOUT prompting
    #[arg(short = 'o')]
    pub overwrite: bool,

    /// Junk paths (do not make directories)
    #[arg(short = 'j')]
    pub junk_paths: bool,

    /// Quiet mode (-qq => quieter)
    #[arg(short = 'q', action = clap::ArgAction::Count)]
    pub quiet: u8,
}

impl Cli {
    pub fn is_http_url(&self) -> bool {
        self.file.starts_with("http://") || self.file.starts_with("https://")
    }

    pub fn is_quiet(&self) -> bool {
        self.quiet > 0 || self.pipe
    }

    pub fn is_very_quiet(&self) -> bool {
        self.quiet > 1
    }
}
