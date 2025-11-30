//! Main entry point for the runzip CLI application.
//!
//! This binary provides a command-line interface for extracting ZIP files
//! from both local filesystem and remote HTTP URLs.

use anyhow::Result;
use clap::Parser;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use runzip::{Cli, HttpRangeReader, LocalFileReader, ReadAt, ZipExtractor, ZipFileEntry};

/// Application entry point.
///
/// Parses command-line arguments and dispatches to the appropriate handler
/// based on whether the input is a local file or HTTP URL.
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.is_http_url() {
        // Handle remote ZIP file via HTTP Range requests
        let reader = HttpRangeReader::new(cli.file.clone()).await?;
        let transferred_before = reader.transferred_bytes();
        let reader = Arc::new(reader);

        process_zip(reader.clone(), &cli).await?;

        // Display network transfer statistics for HTTP sources
        if !cli.is_quiet() {
            let transferred = reader.transferred_bytes() - transferred_before;
            eprintln!("\nTotal bytes transferred: {}", format_size(transferred));
        }
    } else {
        // Handle local ZIP file
        let reader = Arc::new(LocalFileReader::new(Path::new(&cli.file))?);
        process_zip(reader, &cli).await?;
    }

    Ok(())
}

/// Process a ZIP archive based on CLI options.
///
/// This function handles both listing and extraction modes:
/// - List mode (`-l` or `-v`): Display archive contents
/// - Extract mode: Extract files matching the specified filters
///
/// # Arguments
///
/// * `reader` - A reader implementing the `ReadAt` trait for random access
/// * `cli` - Parsed command-line arguments
///
/// # Returns
///
/// Returns `Ok(())` on success, or an error if processing fails.
async fn process_zip<R: ReadAt + 'static>(reader: Arc<R>, cli: &Cli) -> Result<()> {
    let extractor = ZipExtractor::new(reader);

    // List mode: display archive contents and exit
    if cli.list || cli.verbose {
        return list_files(&extractor, cli.verbose).await;
    }

    // Extract mode: get all entries from the archive
    let entries = extractor.list_files().await?;

    // Apply filters to determine which files to extract:
    // 1. Skip directories (they are created automatically during extraction)
    // 2. If specific files are requested, only include matching entries
    // 3. Exclude files matching the exclusion patterns
    let files_to_extract: Vec<_> = entries
        .iter()
        .filter(|e| {
            // Skip directory entries
            if e.is_directory {
                return false;
            }

            // If specific files are requested via positional arguments,
            // only include entries that match
            if !cli.files.is_empty() {
                let matches = cli.files.iter().any(|f| {
                    if has_glob_chars(f) {
                        // Pattern contains wildcards: use glob matching
                        glob_match(f, &e.file_name)
                    } else {
                        // No wildcards: exact match on filename or full path
                        let basename = Path::new(&e.file_name)
                            .file_name()
                            .map(|s| s.to_string_lossy())
                            .unwrap_or_default();
                        e.file_name == *f || basename == *f
                    }
                });
                if !matches {
                    return false;
                }
            }

            // Exclude files matching the -x patterns
            if cli
                .exclude
                .iter()
                .any(|x| e.file_name.contains(x) || glob_match(x, &e.file_name))
            {
                return false;
            }

            true
        })
        .collect();

    // Extract each matching file
    let multiple_files = cli.pipe && files_to_extract.len() > 1;
    for entry in files_to_extract {
        extract_file(&extractor, entry, cli, multiple_files).await?;
    }

    Ok(())
}

/// List files in the ZIP archive.
///
/// Supports two output formats:
/// - Simple format (`-l`): Just file names, one per line
/// - Verbose format (`-v`): Detailed table with size, compression ratio, and timestamps
///
/// # Arguments
///
/// * `extractor` - The ZIP extractor instance
/// * `verbose` - If true, display detailed information in table format
///
/// # Returns
///
/// Returns `Ok(())` on success, or an error if listing fails.
async fn list_files<R: ReadAt + 'static>(extractor: &ZipExtractor<R>, verbose: bool) -> Result<()> {
    let entries = extractor.list_files().await?;

    if verbose {
        // Print table header for verbose output
        println!(
            "{:>10}  {:>10}  {:>5}  {:>10}  {:>5}  Name",
            "Length", "Size", "Cmpr", "Date", "Time"
        );
        println!("{}", "-".repeat(70));
    }

    // Track totals for summary line
    let mut total_uncompressed = 0u64;
    let mut total_compressed = 0u64;
    let mut file_count = 0usize;

    for entry in &entries {
        if verbose {
            // Parse DOS timestamp into human-readable format
            let (year, month, day) = entry.mod_date();
            let (hour, minute, _second) = entry.mod_time();

            // Calculate compression ratio as percentage saved
            let ratio = if entry.uncompressed_size > 0 {
                format!(
                    "{:>4}%",
                    100 - (entry.compressed_size * 100 / entry.uncompressed_size)
                )
            } else {
                "  0%".to_string()
            };

            // Print detailed entry information
            println!(
                "{:>10}  {:>10}  {}  {:04}-{:02}-{:02}  {:02}:{:02}  {}",
                entry.uncompressed_size,
                entry.compressed_size,
                ratio,
                year,
                month,
                day,
                hour,
                minute,
                entry.file_name
            );

            // Accumulate totals (excluding directories)
            if !entry.is_directory {
                total_uncompressed += entry.uncompressed_size;
                total_compressed += entry.compressed_size;
                file_count += 1;
            }
        } else {
            // Simple format: just the file name
            println!("{}", entry.file_name);
        }
    }

    // Print summary line in verbose mode
    if verbose {
        println!("{}", "-".repeat(70));
        let total_ratio = if total_uncompressed > 0 {
            format!(
                "{:>4}%",
                100 - (total_compressed * 100 / total_uncompressed)
            )
        } else {
            "  0%".to_string()
        };
        println!(
            "{:>10}  {:>10}  {}  {:>21}  {} files",
            total_uncompressed, total_compressed, total_ratio, "", file_count
        );
    }

    Ok(())
}

/// Extract a single file from the archive.
///
/// Handles various extraction options:
/// - Pipe mode (`-p`): Write to stdout instead of file
/// - Custom output directory (`-d`): Extract to specified directory
/// - Junk paths (`-j`): Ignore directory structure in archive
/// - Overwrite control (`-n`, `-o`): Handle existing files
///
/// # Arguments
///
/// * `extractor` - The ZIP extractor instance
/// * `entry` - The ZIP file entry to extract
/// * `cli` - Parsed command-line arguments
/// * `show_filename` - If true, print filename marker before content (for pipe mode with multiple files)
///
/// # Returns
///
/// Returns `Ok(())` on success, or an error if extraction fails.
async fn extract_file<R: ReadAt + 'static>(
    extractor: &ZipExtractor<R>,
    entry: &ZipFileEntry,
    cli: &Cli,
    show_filename: bool,
) -> Result<()> {
    // Pipe mode: write file contents directly to stdout
    if cli.pipe {
        if show_filename {
            use tokio::io::AsyncWriteExt;
            let mut stdout = tokio::io::stdout();
            stdout
                .write_all(format!("--- {} ---\n", entry.file_name).as_bytes())
                .await?;
        }
        return extractor.extract_to_stdout(entry).await;
    }

    // Determine the output path based on CLI options
    let output_path = if let Some(ref dir) = cli.extract_dir {
        // Extract to custom directory
        let file_name = if cli.junk_paths {
            // Junk paths: use only the base filename, ignore directory structure
            Path::new(&entry.file_name)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| entry.file_name.clone())
        } else {
            // Preserve directory structure from archive
            entry.file_name.clone()
        };
        PathBuf::from(dir).join(&file_name)
    } else {
        // Extract to current directory
        let file_name = if cli.junk_paths {
            Path::new(&entry.file_name)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| entry.file_name.clone())
        } else {
            entry.file_name.clone()
        };
        PathBuf::from(&file_name)
    };

    // Handle existing files based on overwrite options
    if output_path.exists() {
        if cli.never_overwrite {
            // -n flag: never overwrite, skip silently (unless quiet)
            if !cli.is_quiet() {
                eprintln!("Skipping: {} (file exists)", entry.file_name);
            }
            return Ok(());
        }

        if !cli.overwrite {
            // Default behavior: skip with suggestion to use -o
            if !cli.is_quiet() {
                eprintln!("Skipping: {} (use -o to overwrite)", entry.file_name);
            }
            return Ok(());
        }
        // -o flag: overwrite without prompting (fall through to extraction)
    }

    // Display extraction progress
    if !cli.is_quiet() {
        println!("  extracting: {}", entry.file_name);
    }

    // Perform the actual extraction
    extractor.extract_to_file(entry, &output_path).await?;

    Ok(())
}

/// Check if a pattern contains glob wildcard characters.
///
/// # Arguments
///
/// * `pattern` - The pattern to check
///
/// # Returns
///
/// Returns `true` if the pattern contains `*` or `?` wildcards.
fn has_glob_chars(pattern: &str) -> bool {
    pattern.contains('*') || pattern.contains('?')
}

/// Simple glob pattern matching supporting `*` and `?` wildcards.
///
/// This is a basic implementation for file matching:
/// - `*` matches zero or more characters
/// - `?` matches exactly one character
///
/// # Arguments
///
/// * `pattern` - The glob pattern to match against
/// * `text` - The text to check for a match
///
/// # Returns
///
/// Returns `true` if the text matches the pattern, `false` otherwise.
///
/// # Examples
///
/// ```ignore
/// assert!(glob_match("*.txt", "readme.txt"));
/// assert!(glob_match("file?.dat", "file1.dat"));
/// assert!(!glob_match("*.txt", "readme.md"));
/// ```
fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern_chars: Vec<char> = pattern.chars().collect();
    let text_chars: Vec<char> = text.chars().collect();

    /// Recursive helper function for glob matching.
    ///
    /// Uses a simple backtracking algorithm to handle `*` wildcards.
    fn do_match(pattern: &[char], text: &[char]) -> bool {
        match (pattern.first(), text.first()) {
            // Both exhausted: match successful
            (None, None) => true,
            // Star matches zero or more characters
            (Some('*'), _) => {
                // Try matching zero characters (skip the star)
                // OR matching one character (keep the star for more)
                do_match(&pattern[1..], text) || (!text.is_empty() && do_match(pattern, &text[1..]))
            }
            // Question mark matches exactly one character
            (Some('?'), Some(_)) => do_match(&pattern[1..], &text[1..]),
            // Literal character match
            (Some(p), Some(t)) if *p == *t => do_match(&pattern[1..], &text[1..]),
            // No match
            _ => false,
        }
    }

    do_match(&pattern_chars, &text_chars)
}

/// Format a byte size into a human-readable string.
///
/// Automatically selects the appropriate unit (bytes, KB, MB, GB)
/// based on the size magnitude.
///
/// # Arguments
///
/// * `size` - The size in bytes to format
///
/// # Returns
///
/// A formatted string with the size and appropriate unit.
///
/// # Examples
///
/// ```ignore
/// assert_eq!(format_size(500), "500 bytes");
/// assert_eq!(format_size(1536), "1.50 KB");
/// assert_eq!(format_size(1048576), "1.00 MB");
/// ```
fn format_size(size: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if size >= GB {
        format!("{:.2} GB", size as f64 / GB as f64)
    } else if size >= MB {
        format!("{:.2} MB", size as f64 / MB as f64)
    } else if size >= KB {
        format!("{:.2} KB", size as f64 / KB as f64)
    } else {
        format!("{} bytes", size)
    }
}
