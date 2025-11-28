# runzip

A Rust unzip utility with HTTP URL support using Range requests.

[![Crates.io](https://img.shields.io/crates/v/runzip.svg)](https://crates.io/crates/runzip)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

## Features

- Extract ZIP files from local filesystem
- Extract ZIP files from HTTP/HTTPS URLs using Range requests
- Only download the parts you need - perfect for large remote archives
- Support for ZIP64 format (archives larger than 4GB)
- Support for STORED and DEFLATE compression methods
- Familiar `unzip`-like command line interface
- Cross-platform (Linux, macOS, Windows)

## Installation

### From crates.io

```bash
cargo install runzip
```

### From source

```bash
git clone https://github.com/Ylarod/runzip
cd runzip
cargo install --path .
```

## Usage

### Basic usage

```bash
# Extract all files from a local ZIP
runzip archive.zip

# Extract all files from a remote ZIP (only downloads needed parts!)
runzip https://example.com/large-archive.zip

# Extract to a specific directory
runzip archive.zip -d /path/to/output

# Extract specific files
runzip archive.zip file1.txt file2.txt

# Extract files matching a pattern
runzip archive.zip "*.txt"
```

### List archive contents

```bash
# List files (simple)
runzip -l archive.zip

# List files with details (size, compression ratio, date)
runzip -v archive.zip

# List remote archive (minimal download)
runzip -l https://example.com/archive.zip
```

### Advanced options

```bash
# Extract to stdout (pipe mode)
runzip -p archive.zip file.txt | grep "pattern"

# Exclude files
runzip archive.zip -x "*.log" "*.tmp"

# Overwrite existing files without prompting
runzip -o archive.zip

# Never overwrite existing files
runzip -n archive.zip

# Junk paths (extract all files to current directory, ignore paths)
runzip -j archive.zip

# Quiet mode
runzip -q archive.zip
```

## Command Line Options

```
Usage: runzip [OPTIONS] <FILE> [FILES]...

Arguments:
  <FILE>      ZIP file path or HTTP URL
  [FILES]...  Files to extract (default: all)

Options:
  -l              List files (short format)
  -v              List verbosely/show version info
  -p              Extract files to pipe, no messages
  -d <DIR>        Extract files into directory
  -x <FILE>...    Exclude files that match patterns
  -n              Never overwrite existing files
  -o              Overwrite files WITHOUT prompting
  -j              Junk paths (do not make directories)
  -q              Quiet mode (-qq => quieter)
  -h, --help      Print help
  -V, --version   Print version
```

## How It Works

### HTTP Range Requests

When extracting from an HTTP URL, runzip uses HTTP Range requests to download only the necessary parts of the archive:

1. **HEAD request** - Get file size and verify Range support
2. **Read EOCD** - Download the last ~64KB to find the End of Central Directory
3. **Read Central Directory** - Download the file listing
4. **Extract files** - Download only the specific file data needed

This means you can extract a single 1KB file from a 10GB remote archive by downloading only a few kilobytes!

### ZIP Format Support

| Feature | Status |
|---------|--------|
| Standard ZIP | Supported |
| ZIP64 (>4GB) | Supported |
| STORED (no compression) | Supported |
| DEFLATE compression | Supported |
| Encryption | Not supported |
| BZIP2, LZMA, etc. | Not supported |
| Multi-disk archives | Not supported |

## Library Usage

runzip can also be used as a library:

```rust
use std::sync::Arc;
use runzip::{HttpRangeReader, ZipExtractor};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create a reader for a remote ZIP file
    let reader = Arc::new(
        HttpRangeReader::new("https://example.com/archive.zip".to_string()).await?
    );

    // Create an extractor
    let extractor = ZipExtractor::new(reader);

    // List all files
    for entry in extractor.list_files().await? {
        println!("{}: {} bytes", entry.file_name, entry.uncompressed_size);
    }

    Ok(())
}
```

## Performance

When working with remote archives, runzip is highly efficient:

| Operation | Data Downloaded |
|-----------|----------------|
| List files | ~64KB + Central Directory size |
| Extract one file | List + file's compressed size |
| Extract all files | Full archive |

For local files, runzip uses platform-optimized I/O:
- **Unix**: `pread()` for atomic positioned reads
- **Windows**: Handle duplication for thread-safe reads

## License

MIT License - see [LICENSE](LICENSE) for details.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## Related Projects

- [unzip](http://infozip.sourceforge.net/UnZip.html) - The original unzip utility
- [zip-rs](https://github.com/zip-rs/zip) - Rust ZIP library (full-featured, but requires full file access)
