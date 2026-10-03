//! Incremental reading of append-only JSONL files by byte offset.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// One complete line of a JSONL file and the byte offset just past it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// The line without its trailing newline.
    pub text: String,
    /// Byte offset right after the line's newline: where the next read resumes.
    pub end: u64,
}

/// Size of `path` in bytes, or 0 when it doesn't exist.
#[must_use]
pub fn file_size(path: &Path) -> u64 {
    path.metadata().map_or(0, |meta| meta.len())
}

/// Complete, non-blank lines of `path` past `offset`.
///
/// An unfinished tail (the writer is mid-line) is left for the next call. A missing file has no
/// lines.
///
/// # Errors
///
/// Returns the I/O error when an existing file can't be read.
pub fn new_lines(path: &Path, offset: u64) -> io::Result<Vec<Line>> {
    let size = file_size(path);
    if size <= offset {
        return Ok(Vec::new());
    }
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut chunk = Vec::new();
    file.take(size - offset).read_to_end(&mut chunk)?;
    let Some(last_newline) = chunk.iter().rposition(|&byte| byte == b'\n') else {
        return Ok(Vec::new());
    };

    let mut end = offset;
    let mut lines = Vec::new();
    for raw in chunk[..last_newline].split(|&byte| byte == b'\n') {
        end += raw.len() as u64 + 1;
        if !raw.trim_ascii().is_empty() {
            lines.push(Line {
                text: String::from_utf8_lossy(raw).into_owned(),
                end,
            });
        }
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn reads_complete_lines_and_leaves_the_unfinished_tail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.jsonl");
        fs::write(&path, "a\n\nbc\npartial").unwrap();

        let lines = new_lines(&path, 0).unwrap();
        assert_eq!(
            lines,
            vec![
                Line {
                    text: "a".into(),
                    end: 2
                },
                Line {
                    text: "bc".into(),
                    end: 6
                }
            ]
        );
        assert_eq!(new_lines(&path, 6).unwrap(), Vec::new());

        fs::write(&path, "a\n\nbc\npartial line\n").unwrap();
        assert_eq!(
            new_lines(&path, 6).unwrap(),
            vec![Line {
                text: "partial line".into(),
                end: 19
            }]
        );
    }

    #[test]
    fn missing_file_has_no_lines() {
        assert_eq!(
            new_lines(Path::new("/nonexistent/x.jsonl"), 0).unwrap(),
            Vec::new()
        );
        assert_eq!(file_size(Path::new("/nonexistent/x.jsonl")), 0);
    }
}
