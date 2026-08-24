use crate::crypto;
use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::Request;
use std::future::Future;
use std::path::PathBuf;
use tokio::sync::{mpsc, oneshot};

/// Channel capacity for file upload chunks (provides backpressure).
const UPLOAD_CHANNEL_CAPACITY: usize = 16;

/// Size of the write buffer that coalesces incoming body chunks (typically one
/// TLS record, ~16 KiB) into larger file writes.
const WRITE_BUFFER_SIZE: usize = 512 * 1024;

/// Where the content of an uploaded file should go, decided by the application.
#[derive(Debug)]
pub enum FileUploadTarget {
    /// The application consumes the binary chunks itself.
    ///
    /// The server forwards chunks into `binary_tx` and closes it at end of file.
    /// The application should compare the number of received bytes with `file.size`
    /// and report the result on the sender side of `result_rx` which determines
    /// the HTTP response (200 on `Ok`, 500 on `Err` or when the sender is dropped).
    ///
    /// Note: the checksum is only verified after the application reported its
    /// result, so a checksum mismatch is not observable by the application here
    /// (unlike [FileUploadTarget::Path] and [FileUploadTarget::Fd]).
    Stream {
        /// Channel the server sends the binary chunks of the file into.
        binary_tx: mpsc::Sender<Bytes>,

        /// Channel on which the application reports whether the file was
        /// processed successfully.
        result_rx: oneshot::Receiver<Result<(), String>>,
    },

    /// The server writes the file to this path (created or truncated)
    /// and reports the result on `result_tx`.
    ///
    /// Timestamps provided in the sender's file metadata are applied to the
    /// written file, so the application does not need to set them itself.
    Path {
        /// The path to write the file to.
        path: PathBuf,

        /// Channel on which the server reports whether the file was saved
        /// successfully, including the checksum verification if one was given.
        result_tx: oneshot::Sender<Result<(), String>>,

        /// Optional channel on which the server reports the number of bytes
        /// written so far. Events are dropped when the channel is full.
        progress_tx: Option<mpsc::Sender<u64>>,
    },

    /// The server writes the file to this raw file descriptor (Android only)
    /// and reports the result on `result_tx`.
    ///
    /// Timestamps provided in the sender's file metadata are applied through
    /// the descriptor, which also covers SAF documents that have no path.
    #[cfg(target_os = "android")]
    Fd {
        /// The raw file descriptor to write the file to.
        /// Ownership is transferred; the descriptor is closed after writing.
        fd: std::os::fd::RawFd,

        /// Channel on which the server reports whether the file was saved
        /// successfully, including the checksum verification if one was given.
        result_tx: oneshot::Sender<Result<(), String>>,

        /// Optional channel on which the server reports the number of bytes
        /// written so far. Events are dropped when the channel is full.
        progress_tx: Option<mpsc::Sender<u64>>,
    },
}

/// Timestamps extracted from the sender's file metadata.
pub(crate) struct FileTimestamps {
    pub modified: Option<std::time::SystemTime>,
    pub accessed: Option<std::time::SystemTime>,
}

impl FileTimestamps {
    fn is_empty(&self) -> bool {
        self.modified.is_none() && self.accessed.is_none()
    }
}

impl Default for FileTimestamps {
    fn default() -> Self {
        Self {
            modified: None,
            accessed: None,
        }
    }
}

/// The outcome of receiving a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SaveResult {
    /// The file has been received and, if a checksum was given, it matched.
    Success,

    /// The body could not be read or the target failed to process it.
    Failed,

    /// The received bytes do not match the expected SHA-256 checksum.
    HashMismatch,

    /// The client's `offset` does not match the size of the already-received
    /// partial file. The value is the number of bytes actually persisted, so
    /// the sender can resume from there (or 0 to start over).
    OffsetMismatch(u64),
}

/// Forwards the body of `req` to `target`.
///
/// `offset` is the number of bytes of the file that were already received in a
/// previous attempt and are persisted at the target: the server then appends
/// the body to the existing partial file instead of truncating it. The existing
/// file must be exactly `offset` bytes long, otherwise
/// [SaveResult::OffsetMismatch] is returned so the sender can adjust.
pub(crate) async fn save_req_to_target(
    req: Request<Incoming>,
    target: FileUploadTarget,
    file_size: u64,
    expected_sha256: Option<&str>,
    timestamps: FileTimestamps,
    offset: u64,
) -> SaveResult {
    use sha2::{Digest, Sha256};

    // A resumed transfer can only append to an existing partial file of exactly
    // `offset` bytes. Validate before writing so an unusable offset fails fast
    // instead of corrupting the target.
    if offset > 0 {
        let existing = match &target {
            FileUploadTarget::Path { path, .. } => match tokio::fs::metadata(path).await {
                Ok(metadata) => metadata.len(),
                Err(_) => 0,
            },
            #[cfg(target_os = "android")]
            FileUploadTarget::Fd { fd, .. } => {
                use std::os::fd::FromRawFd;
                // SAFETY: the descriptor is owned by this transfer, but we only
                // probe its size through a duplicated handle and forget it again,
                // so ownership stays with the writer below.
                let std_file = unsafe { std::fs::File::from_raw_fd(*fd) };
                let len = std_file.metadata().map(|m| m.len()).unwrap_or(0);
                std::mem::forget(std_file);
                len
            }
            // A stream target has no persisted partial file to resume into.
            FileUploadTarget::Stream { .. } => 0,
        };
        if existing != offset {
            return SaveResult::OffsetMismatch(existing);
        }
    }

    // Resolve the target into a chunk sender and a result receiver.
    // For [FileUploadTarget::Path] and [FileUploadTarget::Fd], the application's
    // result channel is answered by this function once the complete outcome
    // (including the checksum verification) is known.
    // A resumed transfer verifies the whole file afterwards; remember the path
    // here because the target is consumed below.
    let resume_path = match (&target, offset > 0, expected_sha256.is_some()) {
        (FileUploadTarget::Path { path, .. }, true, true) => Some(path.clone()),
        _ => None,
    };
    let (binary_tx, result_rx, app_result_tx) = match target {
        FileUploadTarget::Stream {
            binary_tx,
            result_rx,
        } => (binary_tx, result_rx, None),
        FileUploadTarget::Path {
            path,
            result_tx,
            progress_tx,
        } => {
            let (binary_tx, result_rx) = spawn_file_writer(
                async move {
                    if offset == 0 {
                        tokio::fs::File::create(&path)
                            .await
                            .map_err(|e| format!("Failed to create {}: {e}", path.display()))
                    } else {
                        // Resume: open without truncating and continue at offset.
                        use tokio::io::AsyncSeekExt;
                        let mut file = tokio::fs::OpenOptions::new()
                            .write(true)
                            .open(&path)
                            .await
                            .map_err(|e| format!("Failed to open {}: {e}", path.display()))?;
                        file.seek(std::io::SeekFrom::Start(offset))
                            .await
                            .map_err(|e| format!("Failed to seek {} to {offset}: {e}", path.display()))?;
                        Ok(file)
                    }
                },
                file_size,
                offset,
                progress_tx,
                timestamps,
            );
            (binary_tx, result_rx, Some(result_tx))
        }
        #[cfg(target_os = "android")]
        FileUploadTarget::Fd {
            fd,
            result_tx,
            progress_tx,
        } => {
            let (binary_tx, result_rx) = spawn_file_writer(
                async move {
                    use std::os::fd::FromRawFd;
                    use tokio::io::AsyncSeekExt;

                    // SAFETY: the descriptor is owned by this transfer; wrapping it in
                    // a File transfers that ownership so it is closed once writing finishes.
                    let std_file = unsafe { std::fs::File::from_raw_fd(fd) };
                    let mut file = tokio::fs::File::from_std(std_file);
                    if offset > 0 {
                        // The application opened the document without truncating
                        // ("rwt"), so the position must be moved to the offset.
                        file.seek(std::io::SeekFrom::Start(offset))
                            .await
                            .map_err(|e| format!("Failed to seek descriptor to {offset}: {e}"))?;
                    }
                    Ok(file)
                },
                file_size,
                offset,
                progress_tx,
                timestamps,
            );
            (binary_tx, result_rx, Some(result_tx))
        }
    };

    // Forward the request body to the target, hashing it on the way if requested
    // and this is a fresh transfer. Resumed transfers verify the complete file
    // afterwards (see below), because the prefix was not part of this body.
    let mut hasher = if offset == 0 {
        expected_sha256.map(|_| Sha256::new())
    } else {
        None
    };
    let mut body = req.into_body();
    let mut stream_error = false;
    while let Some(frame) = body.frame().await {
        match frame {
            Ok(frame) => {
                let Ok(data) = frame.into_data() else {
                    continue; // ignore non-data frames (e.g. trailers)
                };
                if data.is_empty() {
                    continue;
                }
                if let Some(hasher) = &mut hasher {
                    hasher.update(&data);
                }
                if binary_tx.send(data).await.is_err() {
                    // The receiver is gone (dropped by the application or
                    // closed by the file writer after an error).
                    stream_error = true;
                    break;
                }
            }
            Err(err) => {
                tracing::warn!("Error reading upload body of file: {err:#}");
                stream_error = true;
                break;
            }
        }
    }

    // Signal end of file to the receiving side.
    drop(binary_tx);

    // Determine the outcome; `error` carries the reason for the application.
    let (result, error): (SaveResult, Option<String>) = 'outcome: {
        if stream_error {
            // The file writer (if any) fails with a size mismatch or its own
            // write error once the channel is closed; collect that reason.
            let error = match app_result_tx.is_some() {
                true => match result_rx.await {
                    Ok(Err(err)) => Some(err),
                    _ => None,
                },
                false => None,
            };
            break 'outcome (
                SaveResult::Failed,
                error.or(Some("Upload aborted".to_string())),
            );
        }

        match result_rx.await {
            Ok(Ok(())) => (),
            Ok(Err(err)) => {
                tracing::warn!("Failed to process file: {err}");
                break 'outcome (SaveResult::Failed, Some(err));
            }
            Err(_) => break 'outcome (SaveResult::Failed, Some("Upload aborted".to_string())),
        }

        if let (Some(hasher), Some(expected)) = (hasher, expected_sha256) {
            let actual = crypto::hash::to_hex(&hasher.finalize());
            if !actual.eq_ignore_ascii_case(expected) {
                tracing::warn!("Checksum mismatch: expected {expected}, got {actual}");
                break 'outcome (
                    SaveResult::HashMismatch,
                    Some("Checksum mismatch".to_string()),
                );
            }
        }

        // Resumed transfer: the body only contained the tail, so verify the
        // complete file instead of the chunks seen in this request.
        if let Some(path) = resume_path {
            let expected = expected_sha256.expect("resume_path implies a checksum");
            match crypto::hash::sha256_file_content(
                crate::model::transfer::FileContent::Path(path),
                &tokio_util::sync::CancellationToken::new(),
                |_| {},
            )
            .await
            {
                Ok(actual) if actual.eq_ignore_ascii_case(expected) => {}
                Ok(actual) => {
                    tracing::warn!("Checksum mismatch after resume: expected {expected}, got {actual}");
                    break 'outcome (
                        SaveResult::HashMismatch,
                        Some("Checksum mismatch".to_string()),
                    );
                }
                Err(err) => {
                    tracing::warn!("Failed to verify checksum after resume: {err}");
                    break 'outcome (
                        SaveResult::Failed,
                        Some(format!("Failed to verify checksum: {err}")),
                    );
                }
            }
        }

        (SaveResult::Success, None)
    };

    if let Some(app_result_tx) = app_result_tx {
        let _ = app_result_tx.send(match error {
            None => Ok(()),
            Some(err) => Err(err),
        });
    }

    result
}

/// Spawns a task that writes incoming chunks to a file provided by `open`.
///
/// Returns the sender for the binary chunks and a receiver for the final result.
fn spawn_file_writer(
    open: impl Future<Output = Result<tokio::fs::File, String>> + Send + 'static,
    expected_size: u64,
    offset: u64,
    progress_tx: Option<mpsc::Sender<u64>>,
    timestamps: FileTimestamps,
) -> (mpsc::Sender<Bytes>, oneshot::Receiver<Result<(), String>>) {
    let (binary_tx, mut binary_rx) = mpsc::channel::<Bytes>(UPLOAD_CHANNEL_CAPACITY);
    let (internal_tx, internal_rx) = oneshot::channel::<Result<(), String>>();

    tokio::spawn(async move {
        let result = write_file_from_receiver(open, expected_size, offset, &mut binary_rx, progress_tx, timestamps)
            .await;
        // Unblock the request handler if it is still sending chunks.
        binary_rx.close();
        let _ = internal_tx.send(result);
    });

    (binary_tx, internal_rx)
}

/// Writes all chunks received on `rx` to the file provided by `open`.
///
/// `offset` is the number of bytes that were already persisted by a previous
/// attempt; the total file size must be `expected_size = offset + new bytes`.
///
/// Fails if the total number of written bytes does not match `expected_size`
/// (e.g. the sender disconnected mid-transfer).
///
/// The file is truncated to the total size, so that a target that pointed at
/// a longer, pre-existing file cannot keep a tail of the old content.
///
/// The sender-provided `timestamps` are applied to the completely written
/// file. This happens on the still-open handle: an Android file descriptor has
/// no path to address the file by afterwards.
async fn write_file_from_receiver(
    open: impl Future<Output = Result<tokio::fs::File, String>>,
    expected_size: u64,
    offset: u64,
    rx: &mut mpsc::Receiver<Bytes>,
    progress_tx: Option<mpsc::Sender<u64>>,
    timestamps: FileTimestamps,
) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;

    let mut file = tokio::io::BufWriter::with_capacity(WRITE_BUFFER_SIZE, open.await?);
    let mut written: u64 = 0;
    while let Some(chunk) = rx.recv().await {
        written += chunk.len() as u64;
        if offset + written > expected_size {
            return Err(format!(
                "Expected {expected_size} bytes, received at least {}",
                offset + written
            ));
        }
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("Failed to write file: {e}"))?;
        if let Some(progress_tx) = &progress_tx {
            // Progress is best-effort: drop the event when the consumer lags.
            // Reported as the absolute position so resumed transfers continue
            // at the correct fraction.
            let _ = progress_tx.try_send(offset + written);
        }
    }
    file.flush()
        .await
        .map_err(|e| format!("Failed to flush file: {e}"))?;
    let file = file.into_inner();

    if offset + written != expected_size {
        return Err(format!(
            "Expected {expected_size} bytes, received {}",
            offset + written
        ));
    }

    // Drops content beyond the file that was just written, in case the target
    // pointed at a longer, pre-existing file: opening truncates for paths and
    // for descriptors opened with the SAF "wt" mode, but a document provider is
    // free to ignore that mode. Retries of the same upload are covered by the
    // exact size check above either way.
    //
    // Best-effort: a provider may back the descriptor by something that cannot
    // be truncated (e.g. a pipe), which must not fail the completed transfer.
    if let Err(e) = file.set_len(offset + written).await {
        tracing::warn!("Could not truncate file to {} bytes: {e}", offset + written);
    }

    // The timestamps are applied last because the writes and the truncation
    // above update the modification time themselves.
    //
    // Best-effort: the provider backing an Android file descriptor may not
    // support changing timestamps, which must not fail the completed transfer.
    if !timestamps.is_empty() {
        let mut times = std::fs::FileTimes::new();
        if let Some(modified) = timestamps.modified {
            times = times.set_modified(modified);
        }
        if let Some(accessed) = timestamps.accessed {
            times = times.set_accessed(accessed);
        }
        let file = file.into_std().await;
        // Also closes the file, off the async runtime like tokio::fs does.
        let result = tokio::task::spawn_blocking(move || file.set_times(times)).await;
        if let Ok(Err(e)) = result {
            tracing::warn!("Could not set file timestamps: {e}");
        }
    }

    Ok(())
}
