#[cfg(unix)]
use std::fs::OpenOptions;
#[cfg(unix)]
use std::io::Read;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

#[cfg(unix)]
use tokio::io::unix::AsyncFd;
#[cfg(not(unix))]
use tokio::io::{self as tokio_io, AsyncBufReadExt, BufReader};

use crate::CliResult;

pub(super) fn extract_cli_input_line_from_buffer(
    buffer: &mut Vec<u8>,
) -> CliResult<Option<String>> {
    let newline_index = match buffer.iter().position(|byte| *byte == b'\n') {
        Some(index) => index,
        None => return Ok(None),
    };
    let drained_bytes: Vec<u8> = buffer.drain(..=newline_index).collect();
    let normalized_bytes = normalize_cli_input_line_bytes(drained_bytes);
    let line = String::from_utf8(normalized_bytes)
        .map_err(|error| format!("read stdin failed: {error}"))?;

    Ok(Some(line))
}

pub(super) fn finalize_cli_input_buffer(buffer: &mut Vec<u8>) -> CliResult<Option<String>> {
    if buffer.is_empty() {
        return Ok(None);
    }

    let remaining_bytes = std::mem::take(buffer);
    let normalized_bytes = normalize_cli_input_line_bytes(remaining_bytes);
    let line = String::from_utf8(normalized_bytes)
        .map_err(|error| format!("read stdin failed: {error}"))?;

    Ok(Some(line))
}

fn normalize_cli_input_line_bytes(mut bytes: Vec<u8>) -> Vec<u8> {
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }

    bytes
}

#[cfg(unix)]
enum UnixCliInputSource {
    Pollable(AsyncFd<std::fs::File>),
    Blocking(std::fs::File),
}

#[cfg(unix)]
fn open_unix_stdin_file() -> CliResult<std::fs::File> {
    let mut open_options = OpenOptions::new();
    open_options.read(true);
    open_options.custom_flags(libc::O_NONBLOCK);

    let stdin_file = open_options
        .open("/dev/stdin")
        .map_err(|error| format!("open stdin failed: {error}"))?;

    Ok(stdin_file)
}

#[cfg(unix)]
fn build_unix_cli_input_source(stdin_file: std::fs::File) -> CliResult<UnixCliInputSource> {
    let stdin_metadata = stdin_file
        .metadata()
        .map_err(|error| format!("inspect stdin failed: {error}"))?;
    let stdin_file_type = stdin_metadata.file_type();

    if stdin_file_type.is_file() {
        return Ok(UnixCliInputSource::Blocking(stdin_file));
    }

    let pollable_stdin = AsyncFd::new(stdin_file)
        .map_err(|error| format!("configure stdin polling failed: {error}"))?;

    Ok(UnixCliInputSource::Pollable(pollable_stdin))
}

#[cfg(unix)]
fn read_blocking_unix_stdin_chunk(
    stdin_file: &mut std::fs::File,
    chunk: &mut [u8],
) -> CliResult<usize> {
    let read_count = stdin_file
        .read(chunk)
        .map_err(|error| format!("read stdin failed: {error}"))?;

    Ok(read_count)
}

#[cfg(unix)]
async fn read_pollable_unix_stdin_chunk(
    stdin_file: &AsyncFd<std::fs::File>,
    chunk: &mut [u8],
) -> CliResult<usize> {
    loop {
        let mut readiness_guard = stdin_file
            .readable()
            .await
            .map_err(|error| format!("wait for stdin readiness failed: {error}"))?;
        let read_result = readiness_guard.try_io(|inner| inner.get_ref().read(chunk));
        let read_count = match read_result {
            Ok(result) => result.map_err(|error| format!("read stdin failed: {error}"))?,
            Err(_would_block) => continue,
        };

        return Ok(read_count);
    }
}

#[cfg(unix)]
async fn read_unix_stdin_chunk(
    stdin_source: &mut UnixCliInputSource,
    chunk: &mut [u8],
) -> CliResult<usize> {
    match stdin_source {
        UnixCliInputSource::Pollable(stdin_file) => {
            read_pollable_unix_stdin_chunk(stdin_file, chunk).await
        }
        UnixCliInputSource::Blocking(stdin_file) => {
            read_blocking_unix_stdin_chunk(stdin_file, chunk)
        }
    }
}

#[cfg(unix)]
pub(super) struct ConcurrentCliInputReader {
    stdin_source: UnixCliInputSource,
    buffer: Vec<u8>,
}

#[cfg(unix)]
impl ConcurrentCliInputReader {
    pub(super) fn new() -> CliResult<Self> {
        let stdin_file = open_unix_stdin_file()?;

        Self::from_unix_file(stdin_file)
    }

    fn from_unix_file(stdin_file: std::fs::File) -> CliResult<Self> {
        let stdin_source = build_unix_cli_input_source(stdin_file)?;

        Ok(Self {
            stdin_source,
            buffer: Vec::new(),
        })
    }

    pub(super) async fn next_line(&mut self) -> CliResult<Option<String>> {
        loop {
            let buffered_line = extract_cli_input_line_from_buffer(&mut self.buffer)?;
            if buffered_line.is_some() {
                return Ok(buffered_line);
            }

            let mut chunk = [0_u8; 1024];
            let read_count = read_unix_stdin_chunk(&mut self.stdin_source, &mut chunk).await?;

            if read_count == 0 {
                return finalize_cli_input_buffer(&mut self.buffer);
            }

            let chunk_slice = chunk
                .get(..read_count)
                .ok_or_else(|| "read stdin failed: invalid chunk length".to_owned())?;
            self.buffer.extend_from_slice(chunk_slice);
        }
    }
}

#[cfg(not(unix))]
pub(super) struct ConcurrentCliInputReader {
    stdin_lines: tokio_io::Lines<BufReader<tokio_io::Stdin>>,
}

#[cfg(not(unix))]
impl ConcurrentCliInputReader {
    pub(super) fn new() -> CliResult<Self> {
        let stdin_reader = BufReader::new(tokio_io::stdin());
        let stdin_lines = stdin_reader.lines();

        Ok(Self { stdin_lines })
    }

    pub(super) async fn next_line(&mut self) -> CliResult<Option<String>> {
        self.stdin_lines
            .next_line()
            .await
            .map_err(|error| format!("read stdin failed: {error}"))
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::io::Write;

    #[cfg(unix)]
    use super::ConcurrentCliInputReader;
    use super::{extract_cli_input_line_from_buffer, finalize_cli_input_buffer};

    #[test]
    fn extract_cli_input_line_from_buffer_strips_crlf() {
        let mut buffer = b"hello world\r\nnext".to_vec();
        let line = extract_cli_input_line_from_buffer(&mut buffer)
            .expect("buffered line should decode")
            .expect("buffered line should exist");

        assert_eq!(line, "hello world");
        assert_eq!(buffer, b"next");
    }

    #[test]
    fn finalize_cli_input_buffer_returns_partial_line_without_newline() {
        let mut buffer = b"tail fragment".to_vec();
        let line = finalize_cli_input_buffer(&mut buffer)
            .expect("tail line should decode")
            .expect("tail line should exist");

        assert_eq!(line, "tail fragment");
        assert!(buffer.is_empty());
    }

    #[test]
    fn finalize_cli_input_buffer_returns_none_for_empty_buffer() {
        let mut buffer = Vec::new();
        let line = finalize_cli_input_buffer(&mut buffer).expect("empty buffer should not fail");

        assert!(line.is_none());
        assert!(buffer.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn concurrent_cli_input_reader_reads_regular_file_input() {
        let mut temp_file = tempfile::NamedTempFile::new().expect("temp file should exist");
        writeln!(temp_file, "hello world").expect("first line should write");
        write!(temp_file, "tail fragment").expect("tail line should write");
        temp_file.flush().expect("temp file should flush");

        let stdin_file = temp_file.reopen().expect("temp file should reopen");
        let mut reader =
            ConcurrentCliInputReader::from_unix_file(stdin_file).expect("reader should open");
        let first_line = reader
            .next_line()
            .await
            .expect("first read should succeed")
            .expect("first line should exist");
        let second_line = reader
            .next_line()
            .await
            .expect("second read should succeed")
            .expect("second line should exist");
        let end_of_input = reader.next_line().await.expect("eof read should succeed");

        assert_eq!(first_line, "hello world");
        assert_eq!(second_line, "tail fragment");
        assert!(end_of_input.is_none());
    }
}
