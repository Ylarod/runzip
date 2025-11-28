use anyhow::Result;
use clap::Parser;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use runzip::{Cli, HttpRangeReader, LocalFileReader, ReadAt, ZipExtractor, ZipFileEntry};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.is_http_url() {
        let reader = HttpRangeReader::new(cli.file.clone()).await?;
        let transferred_before = reader.transferred_bytes();
        let reader = Arc::new(reader);

        process_zip(reader.clone(), &cli).await?;

        // Show transferred bytes for HTTP
        if !cli.is_quiet() {
            let transferred = reader.transferred_bytes() - transferred_before;
            eprintln!("\nTotal bytes transferred: {}", format_size(transferred));
        }
    } else {
        let reader = Arc::new(LocalFileReader::new(Path::new(&cli.file))?);
        process_zip(reader, &cli).await?;
    }

    Ok(())
}

async fn process_zip<R: ReadAt + 'static>(reader: Arc<R>, cli: &Cli) -> Result<()> {
    let extractor = ZipExtractor::new(reader);

    // List mode
    if cli.list || cli.verbose {
        return list_files(&extractor, cli.verbose).await;
    }

    // Extract mode
    let entries = extractor.list_files().await?;

    // Filter files to extract
    let files_to_extract: Vec<_> = entries
        .iter()
        .filter(|e| {
            // Skip directories
            if e.is_directory {
                return false;
            }

            // If specific files requested, filter by them
            if !cli.files.is_empty() {
                let matches = cli
                    .files
                    .iter()
                    .any(|f| e.file_name.contains(f) || glob_match(f, &e.file_name));
                if !matches {
                    return false;
                }
            }

            // Apply exclusions
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

    // Extract each file
    for entry in files_to_extract {
        extract_file(&extractor, entry, cli).await?;
    }

    Ok(())
}

async fn list_files<R: ReadAt + 'static>(extractor: &ZipExtractor<R>, verbose: bool) -> Result<()> {
    let entries = extractor.list_files().await?;

    if verbose {
        println!(
            "{:>10}  {:>10}  {:>5}  {:>10}  {:>5}  Name",
            "Length", "Size", "Cmpr", "Date", "Time"
        );
        println!("{}", "-".repeat(70));
    }

    let mut total_uncompressed = 0u64;
    let mut total_compressed = 0u64;
    let mut file_count = 0usize;

    for entry in &entries {
        if verbose {
            let (year, month, day) = entry.mod_date();
            let (hour, minute, _second) = entry.mod_time();

            let ratio = if entry.uncompressed_size > 0 {
                format!(
                    "{:>4}%",
                    100 - (entry.compressed_size * 100 / entry.uncompressed_size)
                )
            } else {
                "  0%".to_string()
            };

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

            if !entry.is_directory {
                total_uncompressed += entry.uncompressed_size;
                total_compressed += entry.compressed_size;
                file_count += 1;
            }
        } else {
            println!("{}", entry.file_name);
        }
    }

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

async fn extract_file<R: ReadAt + 'static>(
    extractor: &ZipExtractor<R>,
    entry: &ZipFileEntry,
    cli: &Cli,
) -> Result<()> {
    // Pipe mode: output to stdout
    if cli.pipe {
        return extractor.extract_to_stdout(entry).await;
    }

    // Determine output path
    let output_path = if let Some(ref dir) = cli.extract_dir {
        let file_name = if cli.junk_paths {
            Path::new(&entry.file_name)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| entry.file_name.clone())
        } else {
            entry.file_name.clone()
        };
        PathBuf::from(dir).join(&file_name)
    } else {
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

    // Check if file exists
    if output_path.exists() {
        if cli.never_overwrite {
            if !cli.is_quiet() {
                eprintln!("Skipping: {} (file exists)", entry.file_name);
            }
            return Ok(());
        }

        if !cli.overwrite {
            // In non-interactive mode, skip by default
            if !cli.is_quiet() {
                eprintln!("Skipping: {} (use -o to overwrite)", entry.file_name);
            }
            return Ok(());
        }
    }

    // Print extraction message
    if !cli.is_quiet() {
        println!("  extracting: {}", entry.file_name);
    }

    // Extract
    extractor.extract_to_file(entry, &output_path).await?;

    Ok(())
}

/// Simple glob pattern matching (supports * and ?)
fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern_chars: Vec<char> = pattern.chars().collect();
    let text_chars: Vec<char> = text.chars().collect();

    fn do_match(pattern: &[char], text: &[char]) -> bool {
        match (pattern.first(), text.first()) {
            (None, None) => true,
            (Some('*'), _) => {
                // * matches zero or more characters
                do_match(&pattern[1..], text) || (!text.is_empty() && do_match(pattern, &text[1..]))
            }
            (Some('?'), Some(_)) => {
                // ? matches exactly one character
                do_match(&pattern[1..], &text[1..])
            }
            (Some(p), Some(t)) if *p == *t => do_match(&pattern[1..], &text[1..]),
            _ => false,
        }
    }

    do_match(&pattern_chars, &text_chars)
}

/// Format byte size to human readable string
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
