//! Streaming, resumable download of one immutable, hash-pinned model artifact.
use crate::processing::{cancelled, interruptible, message, plain_file, ModelFile};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use uuid::Uuid;

pub async fn item(
    client: &reqwest::Client,
    url: &str,
    target: &Path,
    spec: &ModelFile,
    cancel: &AtomicBool,
    report: impl Fn(u64),
) -> Result<(), String> {
    let partial = target.join(format!("{}.part", spec.name));
    let destination = target.join(&spec.name);
    let mut bytes = 0;
    let mut hash = Sha256::new();
    if partial.exists() {
        let mut prior = plain_file(&partial)?;
        if prior.metadata().map_err(message)?.len() > spec.bytes {
            fs::rename(
                &partial,
                target.join(format!("{}.invalid-{}", spec.name, Uuid::new_v4())),
            )
            .map_err(message)?;
        } else {
            let mut buffer = [0u8; 1024 * 1024];
            loop {
                cancelled(cancel)?;
                let n = prior.read(&mut buffer).map_err(message)?;
                if n == 0 {
                    break;
                }
                hash.update(&buffer[..n]);
                bytes += n as u64;
                report(bytes);
            }
        }
    }
    if bytes < spec.bytes {
        crate::storage_space::check(target, spec.bytes.saturating_sub(bytes))?;
        let mut request = client.get(url);
        if bytes > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={bytes}-"));
        }
        let mut response = interruptible(cancel, request.send())
            .await?
            .map_err(|_| "모델을 내려받지 못했습니다. 다시 준비해 주세요.".to_string())?;
        if bytes > 0 && response.status() == reqwest::StatusCode::OK {
            // A server may ignore Range. Preserve the partial before starting a full response.
            fs::rename(
                &partial,
                target.join(format!("{}.partial-{}", spec.name, Uuid::new_v4())),
            )
            .map_err(message)?;
            bytes = 0;
            hash = Sha256::new();
        } else if bytes > 0 {
            let expected =
                format!("bytes {bytes}-{} /{}", spec.bytes - 1, spec.bytes).replace(" /", "/");
            if response.status() != reqwest::StatusCode::PARTIAL_CONTENT
                || response
                    .headers()
                    .get(reqwest::header::CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok())
                    != Some(expected.as_str())
            {
                return Err(
                    "모델 이어받기 응답이 올바르지 않습니다. 기존 파일은 보관했습니다.".into(),
                );
            }
        } else if response.status() != reqwest::StatusCode::OK {
            return Err("모델을 내려받지 못했습니다. 다시 준비해 주세요.".into());
        }
        crate::storage_space::check(target, spec.bytes.saturating_sub(bytes))?;
        let mut space_checked = std::time::Instant::now();
        let mut output = if !partial.exists() {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&partial)
                .map_err(message)?
        } else {
            plain_file(&partial)?;
            OpenOptions::new()
                .append(true)
                .open(&partial)
                .map_err(message)?
        };
        // reqwest's own text is English; the partial file stays, so preparing again resumes.
        while let Some(chunk) = interruptible(cancel, response.chunk())
            .await?
            .map_err(|_| {
                "모델을 받는 중 연결이 끊겼습니다. 다시 준비하면 이어받습니다.".to_string()
            })?
        {
            bytes += chunk.len() as u64;
            if bytes > spec.bytes {
                return Err("모델 파일의 크기가 올바르지 않습니다.".into());
            }
            if space_checked.elapsed() >= std::time::Duration::from_secs(1) {
                crate::storage_space::check(
                    target,
                    spec.bytes.saturating_sub(bytes - chunk.len() as u64),
                )?;
                space_checked = std::time::Instant::now();
            }
            output
                .write_all(&chunk)
                .map_err(crate::storage_space::write_error)?;
            hash.update(&chunk);
            report(bytes);
        }
        output
            .sync_all()
            .map_err(crate::storage_space::write_error)?;
    }
    cancelled(cancel)?;
    if bytes != spec.bytes || format!("{:x}", hash.finalize()) != spec.sha256 {
        // Bad bytes must not poison all subsequent resume attempts.
        if partial.exists() {
            fs::rename(
                &partial,
                target.join(format!("{}.invalid-{}", spec.name, Uuid::new_v4())),
            )
            .map_err(message)?;
        }
        return Err("모델의 무결성 확인에 실패했습니다. 다시 준비해 주세요.".into());
    }
    if destination.exists() {
        plain_file(&destination)?;
        fs::rename(
            &destination,
            target.join(format!("{}.invalid-{}", spec.name, Uuid::new_v4())),
        )
        .map_err(message)?;
    }
    fs::rename(&partial, &destination).map_err(message)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    #[test]
    fn range_resume_checks_full_hash_and_preserves_invalid_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let cancel = AtomicBool::new(false);
        let spec = ModelFile {
            name: "model.bin".into(),
            bytes: 5,
            sha256: format!("{:x}", Sha256::digest(b"model")),
        };
        fs::write(temp.path().join("model.bin.part"), b"mo").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0; 4096];
            let n = stream.read(&mut buf).unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).to_lowercase();
            assert!(req.contains("range: bytes=2-"));
            stream.write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nContent-Range: bytes 2-4/5\r\nConnection: close\r\n\r\ndel").unwrap();
        });
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime
            .block_on(item(
                &reqwest::Client::builder().no_proxy().build().unwrap(),
                &format!("http://{address}"),
                temp.path(),
                &spec,
                &cancel,
                |_| {},
            ))
            .unwrap();
        thread.join().unwrap();
        assert_eq!(fs::read(temp.path().join("model.bin")).unwrap(), b"model");
        fs::write(temp.path().join("model.bin.part"), b"other").unwrap();
        assert!(runtime
            .block_on(item(
                &reqwest::Client::new(),
                "http://unused",
                temp.path(),
                &spec,
                &cancel,
                |_| {}
            ))
            .is_err());
        assert_eq!(fs::read(temp.path().join("model.bin")).unwrap(), b"model");
        assert!(!temp.path().join("model.bin.part").exists());
    }
}
