//! The speech models: which files, where they come from, and a downloader that checks
//! every byte against a pinned SHA-256 before anything is unpacked or loaded.
//!
//! Layout under the models folder: `<name>/` holds an unpacked archive (its single top
//! folder flattened away) or, for a plain file, the file itself; `<name>.ok` is written
//! last and holds the SHA-256. Nothing here logs; only state names and sizes move.

use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

const BASE: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/";
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// One downloadable file. `url` is a full URL, so a test can point it at a local server.
#[derive(Debug, Clone)]
pub struct ModelFile {
    pub name: String,
    pub url: String,
    pub bytes: u64,
    pub sha256: String,
    /// A `.tar.bz2` to unpack into `<name>/`; otherwise the file is kept as it is.
    pub unpack: bool,
}

#[derive(Debug, Clone)]
pub struct Manifest {
    pub files: Vec<ModelFile>,
}

impl Manifest {
    /// The four models Dots uses, pinned by size and SHA-256.
    pub fn pinned() -> Self {
        let file = |name: &str, path: &str, bytes: u64, sha256: &str, unpack: bool| ModelFile {
            name: name.into(),
            url: format!("{BASE}{path}"),
            bytes,
            sha256: sha256.into(),
            unpack,
        };
        Manifest {
            files: vec![
                file(
                    "speak",
                    "tts-models/sherpa-onnx-supertonic-3-tts-int8-2026-05-11.tar.bz2",
                    128_774_318,
                    "82fa96f91c4ef8abaae3a14a3f4153facf88bed821d1f7331cec2700f432c427",
                    true,
                ),
                file(
                    "hear",
                    "asr-models/sherpa-onnx-moonshine-base-en-quantized-2026-02-27.tar.bz2",
                    111_266_225,
                    "43232c1d13013d37317163baec3135bd771a186a4356f28c889bab453bb0e891",
                    true,
                ),
                file(
                    "wake",
                    "kws-models/sherpa-onnx-kws-zipformer-gigaspeech-3.3M-2024-01-01.tar.bz2",
                    17_626_723,
                    "f170013b4716e41b62b9bfd809687c207cef798ef9bc6534d524e17af9b6561a",
                    true,
                ),
                file(
                    "vad",
                    "asr-models/silero_vad.onnx",
                    643_854,
                    "9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6",
                    false,
                ),
            ],
        }
    }

    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.bytes).sum()
    }
}

/// Where each installed model lives.
#[derive(Debug, Clone)]
pub struct ModelPaths {
    pub root: PathBuf,
}

impl ModelPaths {
    pub fn speak_dir(&self) -> PathBuf {
        self.root.join("speak")
    }
    pub fn hear_dir(&self) -> PathBuf {
        self.root.join("hear")
    }
    pub fn wake_dir(&self) -> PathBuf {
        self.root.join("wake")
    }
    pub fn vad_file(&self) -> PathBuf {
        self.root.join("vad").join("silero_vad.onnx")
    }
}

#[derive(Debug, Clone)]
pub struct Progress {
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub file: String,
}

#[derive(thiserror::Error, Debug)]
pub enum ModelError {
    #[error("the download was cancelled")]
    Cancelled,
    #[error("network: {0}")]
    Network(String),
    #[error("{file} does not match its checksum")]
    WrongHash { file: String },
    #[error("disk: {0}")]
    Disk(String),
}

fn disk(e: impl std::fmt::Display) -> ModelError {
    ModelError::Disk(e.to_string())
}

fn net(e: impl std::fmt::Display) -> ModelError {
    ModelError::Network(e.to_string())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn marker(root: &Path, file: &ModelFile) -> PathBuf {
    root.join(format!("{}.ok", file.name))
}

fn is_installed(root: &Path, file: &ModelFile) -> bool {
    root.join(&file.name).is_dir()
        && std::fs::read_to_string(marker(root, file))
            .is_ok_and(|s| s.trim().eq_ignore_ascii_case(&file.sha256))
}

/// True only when every file's `.ok` marker exists and holds the manifest's hash.
pub fn installed(root: &Path, manifest: &Manifest) -> bool {
    manifest.files.iter().all(|f| is_installed(root, f))
}

/// Delete everything this module lays down under `root` (the folder itself only if it
/// is left empty). Missing files are fine.
pub fn remove(root: &Path) -> std::io::Result<()> {
    fn gone(r: std::io::Result<()>) -> std::io::Result<()> {
        match r {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }
    for f in &Manifest::pinned().files {
        gone(std::fs::remove_dir_all(root.join(&f.name)))?;
        gone(std::fs::remove_dir_all(
            root.join(format!("{}.tmp", f.name)),
        ))?;
        gone(std::fs::remove_file(root.join(format!("{}.part", f.name))))?;
        gone(std::fs::remove_file(root.join(format!("{}.ok", f.name))))?;
    }
    let _ = std::fs::remove_dir(root);
    Ok(())
}

fn check(cancel: &CancellationToken) -> Result<(), ModelError> {
    if cancel.is_cancelled() {
        Err(ModelError::Cancelled)
    } else {
        Ok(())
    }
}

async fn forget(path: &Path) {
    let _ = tokio::fs::remove_file(path).await;
}

/// Fetch every file in `manifest` that is not already installed, check it, and put it
/// in place. A cut download resumes from its `.part` file on the next call.
pub async fn install(
    root: &Path,
    manifest: &Manifest,
    on_progress: impl Fn(Progress) + Send + Sync,
    cancel: CancellationToken,
) -> Result<(), ModelError> {
    tokio::fs::create_dir_all(root).await.map_err(disk)?;
    let total = manifest.total_bytes();
    let client = reqwest::Client::new();
    let mut finished = 0u64;
    for file in &manifest.files {
        check(&cancel)?;
        if !is_installed(root, file) {
            install_one(&client, root, file, finished, total, &on_progress, &cancel).await?;
        }
        finished += file.bytes;
        on_progress(Progress {
            done_bytes: finished,
            total_bytes: total,
            file: file.name.clone(),
        });
    }
    Ok(())
}

async fn install_one(
    client: &reqwest::Client,
    root: &Path,
    file: &ModelFile,
    base_done: u64,
    total: u64,
    on_progress: &(impl Fn(Progress) + Send + Sync),
    cancel: &CancellationToken,
) -> Result<(), ModelError> {
    let part = root.join(format!("{}.part", file.name));
    let report = Report {
        base_done,
        total,
        name: &file.name,
        on_progress,
    };
    download(client, file, &part, &report, cancel).await?;
    check(cancel)?;

    if !hash_file(&part, cancel)
        .await?
        .eq_ignore_ascii_case(&file.sha256)
    {
        forget(&part).await;
        return Err(ModelError::WrongHash {
            file: file.name.clone(),
        });
    }
    check(cancel)?;

    // Whatever was there before is replaced; until the marker is back it is not installed.
    forget(&marker(root, file)).await;
    let tmp = root.join(format!("{}.tmp", file.name));
    let dest = root.join(&file.name);
    for dir in [&tmp, &dest] {
        match tokio::fs::remove_dir_all(dir).await {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(disk(e)),
            _ => {}
        }
    }
    let keep_as = file_name_of(&file.url, &file.name);
    let unpack = file.unpack;
    let (p, t, d) = (part.clone(), tmp.clone(), dest);
    let placed = tokio::task::spawn_blocking(move || place(&p, &t, &d, unpack, &keep_as))
        .await
        .map_err(disk)?;
    if let Err(e) = placed {
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        forget(&part).await;
        return Err(e);
    }
    forget(&part).await;
    tokio::fs::write(marker(root, file), &file.sha256)
        .await
        .map_err(disk)
}

fn file_name_of(url: &str, fallback: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    match path.rsplit('/').next() {
        Some(n) if !n.is_empty() => n.to_owned(),
        _ => fallback.to_owned(),
    }
}

/// Reports progress for one file: `base_done` is what the earlier files already hold.
struct Report<'a, F: Fn(Progress)> {
    base_done: u64,
    total: u64,
    name: &'a str,
    on_progress: &'a F,
}

impl<F: Fn(Progress)> Report<'_, F> {
    fn at(&self, have: u64) {
        (self.on_progress)(Progress {
            done_bytes: self.base_done + have,
            total_bytes: self.total,
            file: self.name.to_owned(),
        });
    }
}

/// Bring `part` to `file.bytes`, resuming with a `Range` request when it already holds
/// some of the file.
async fn download<F: Fn(Progress)>(
    client: &reqwest::Client,
    file: &ModelFile,
    part: &Path,
    report: &Report<'_, F>,
    cancel: &CancellationToken,
) -> Result<(), ModelError> {
    let mut have = tokio::fs::metadata(part)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    if have > file.bytes {
        forget(part).await;
        have = 0;
    }
    if have == file.bytes {
        return Ok(());
    }

    let mut request = client.get(&file.url);
    if have > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let response = tokio::select! {
        r = request.send() => r.map_err(net)?,
        _ = cancel.cancelled() => return Err(ModelError::Cancelled),
    };
    let mut open = tokio::fs::OpenOptions::new();
    open.create(true).write(true);
    match response.status().as_u16() {
        206 => open.append(true),
        // The server sent the whole body, so the file starts over.
        200 => {
            have = 0;
            open.truncate(true)
        }
        416 => {
            forget(part).await;
            return Err(ModelError::Network(
                "the server refused to resume; try again".into(),
            ));
        }
        status => return Err(ModelError::Network(format!("the server answered {status}"))),
    };
    let mut out = open.open(part).await.map_err(disk)?;

    let result = pump(response, &mut out, have, file.bytes, report, cancel).await;
    let flushed = out.flush().await;
    drop(out);
    match result {
        Err(Overrun) => {
            forget(part).await;
            Err(ModelError::Network(
                "the server sent more than expected".into(),
            ))
        }
        Ok(r) => {
            r?;
            flushed.map_err(disk)
        }
    }
}

/// The server sent past the pinned size; the `.part` file is worthless.
struct Overrun;

async fn pump<F: Fn(Progress)>(
    response: reqwest::Response,
    out: &mut tokio::fs::File,
    mut have: u64,
    expected: u64,
    report: &Report<'_, F>,
    cancel: &CancellationToken,
) -> Result<Result<(), ModelError>, Overrun> {
    let mut stream = response.bytes_stream();
    let mut last_report: Option<Instant> = None;
    loop {
        let next = tokio::select! {
            n = stream.next() => n,
            _ = cancel.cancelled() => return Ok(Err(ModelError::Cancelled)),
        };
        let Some(chunk) = next else { break };
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => return Ok(Err(net(e))),
        };
        have += chunk.len() as u64;
        if have > expected {
            return Err(Overrun);
        }
        if let Err(e) = out.write_all(&chunk).await {
            return Ok(Err(disk(e)));
        }
        if last_report.is_none_or(|t| t.elapsed() >= PROGRESS_EVERY) {
            last_report = Some(Instant::now());
            report.at(have);
        }
    }
    if have < expected {
        return Ok(Err(ModelError::Network("the download ended early".into())));
    }
    Ok(Ok(()))
}

async fn hash_file(path: &Path, cancel: &CancellationToken) -> Result<String, ModelError> {
    let mut f = tokio::fs::File::open(path).await.map_err(disk)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        check(cancel)?;
        let n = f.read(&mut buf).await.map_err(disk)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

/// Move a checked download into place: unpack an archive into `tmp` and rename it to
/// `dest` (flattening a single top folder), or move a plain file into `dest/<keep_as>`.
fn place(
    part: &Path,
    tmp: &Path,
    dest: &Path,
    unpack: bool,
    keep_as: &str,
) -> Result<(), ModelError> {
    std::fs::create_dir_all(tmp).map_err(disk)?;
    if !unpack {
        std::fs::rename(part, tmp.join(keep_as)).map_err(disk)?;
        return std::fs::rename(tmp, dest).map_err(disk);
    }

    let file = std::fs::File::open(part).map_err(disk)?;
    let mut archive = tar::Archive::new(bzip2::read::BzDecoder::new(file));
    for entry in archive.entries().map_err(disk)? {
        let mut entry = entry.map_err(disk)?;
        let kind = entry.header().entry_type();
        if kind.is_symlink() || kind.is_hard_link() {
            return Err(ModelError::Disk("the archive contains a link".into()));
        }
        if !(kind.is_file() || kind.is_dir() || kind.is_contiguous()) {
            continue; // headers and other metadata carry no files
        }
        let escapes = entry
            .path()
            .map_err(disk)?
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir));
        if escapes || !entry.unpack_in(tmp).map_err(disk)? {
            return Err(ModelError::Disk(
                "the archive has a path outside its folder".into(),
            ));
        }
    }

    let mut top = std::fs::read_dir(tmp)
        .map_err(disk)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(disk)?;
    if top.len() == 1 && top[0].file_type().map_err(disk)?.is_dir() {
        let inner = top.remove(0).path();
        std::fs::rename(&inner, dest).map_err(disk)?;
        std::fs::remove_dir(tmp).map_err(disk)
    } else {
        std::fs::rename(tmp, dest).map_err(disk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, Bytes};
    use axum::extract::{Path as AxPath, State};
    use axum::http::{header, HeaderMap, StatusCode};
    use axum::response::Response;
    use axum::routing::get;
    use axum::Router;
    use futures_util::StreamExt;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Copy, PartialEq)]
    enum Mode {
        Normal,
        CutFirst,
        IgnoreRange,
    }

    struct Server {
        files: HashMap<String, Vec<u8>>,
        mode: Mode,
        cut_done: AtomicBool,
        ranges: Mutex<Vec<Option<String>>>,
    }

    async fn serve_file(
        State(s): State<Arc<Server>>,
        AxPath(name): AxPath<String>,
        headers: HeaderMap,
    ) -> Response {
        let range = headers
            .get(header::RANGE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        s.ranges.lock().unwrap().push(range.clone());
        let Some(data) = s.files.get(&name) else {
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::empty())
                .unwrap();
        };
        let total = data.len();
        let start = match (&range, s.mode) {
            (Some(r), m) if m != Mode::IgnoreRange => r
                .strip_prefix("bytes=")
                .and_then(|r| r.strip_suffix('-'))
                .and_then(|n| n.parse::<usize>().ok())
                .unwrap_or(0),
            _ => 0,
        };
        let status = if start > 0 {
            StatusCode::PARTIAL_CONTENT
        } else {
            StatusCode::OK
        };
        let rest = Bytes::copy_from_slice(&data[start..]);
        let mut builder = Response::builder()
            .status(status)
            .header(header::CONTENT_LENGTH, rest.len());
        if start > 0 {
            builder = builder.header(
                header::CONTENT_RANGE,
                format!("bytes {start}-{}/{total}", total - 1),
            );
        }
        if s.mode == Mode::CutFirst && !s.cut_done.swap(true, Ordering::SeqCst) {
            let half = rest.slice(..rest.len() / 2);
            // Pause before dropping so the first half is on the wire.
            let stream = futures_util::stream::once(async move { Ok(half) }).chain(
                futures_util::stream::once(async {
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    Err(std::io::Error::other("connection dropped"))
                }),
            );
            return builder.body(Body::from_stream(stream)).unwrap();
        }
        builder.body(Body::from(rest)).unwrap()
    }

    async fn start(files: HashMap<String, Vec<u8>>, mode: Mode) -> (String, Arc<Server>) {
        let state = Arc::new(Server {
            files,
            mode,
            cut_done: AtomicBool::new(false),
            ranges: Mutex::new(Vec::new()),
        });
        let app = Router::new()
            .route("/{name}", get(serve_file))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (base, state)
    }

    fn sha_hex(data: &[u8]) -> String {
        hex(&Sha256::digest(data))
    }

    /// A `.tar.bz2` with one top folder and the given files.
    fn archive(top: &str, files: &[(&str, &[u8])]) -> Vec<u8> {
        let enc = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
        let mut b = tar::Builder::new(enc);
        for (name, data) in files {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, format!("{top}/{name}"), *data)
                .unwrap();
        }
        b.into_inner().unwrap().finish().unwrap()
    }

    /// A `.tar.bz2` with one raw entry, whose name and type are written as given
    /// (the tar builder itself refuses `..`).
    fn raw_archive(name: &str, kind: tar::EntryType, link: Option<&str>) -> Vec<u8> {
        let enc = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
        let mut b = tar::Builder::new(enc);
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(kind);
        h.set_size(if link.is_some() { 0 } else { 4 });
        h.set_mode(0o644);
        h.as_old_mut().name[..name.len()].copy_from_slice(name.as_bytes());
        if let Some(l) = link {
            h.set_link_name(l).unwrap();
        }
        h.set_cksum();
        let body: &[u8] = if link.is_some() { b"" } else { b"evil" };
        b.append(&h, body).unwrap();
        b.into_inner().unwrap().finish().unwrap()
    }

    fn entry(name: &str, base: &str, file: &str, data: &[u8], unpack: bool) -> ModelFile {
        ModelFile {
            name: name.into(),
            url: format!("{base}/{file}"),
            bytes: data.len() as u64,
            sha256: sha_hex(data),
            unpack,
        }
    }

    fn nothing(_: Progress) {}

    fn leftovers(root: &Path) -> Vec<String> {
        std::fs::read_dir(root)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".part") || n.ends_with(".tmp"))
            .collect()
    }

    #[tokio::test]
    async fn a_good_download_installs_and_reports_progress() {
        let tts = archive(
            "speak",
            &[("tts.json", b"{\"a\":1}"), ("model.onnx", b"onnx!")],
        );
        let vad = b"silero-bytes".to_vec();
        let (base, _s) = start(
            HashMap::from([
                ("speak.tar.bz2".into(), tts.clone()),
                ("silero_vad.onnx".into(), vad.clone()),
            ]),
            Mode::Normal,
        )
        .await;
        let manifest = Manifest {
            files: vec![
                entry("speak", &base, "speak.tar.bz2", &tts, true),
                entry("vad", &base, "silero_vad.onnx", &vad, false),
            ],
        };
        let dir = tempfile::tempdir().unwrap();
        assert!(!installed(dir.path(), &manifest));

        let seen = Mutex::new(Vec::<Progress>::new());
        install(
            dir.path(),
            &manifest,
            |p| seen.lock().unwrap().push(p),
            CancellationToken::new(),
        )
        .await
        .unwrap();

        assert!(installed(dir.path(), &manifest));
        let seen = seen.into_inner().unwrap();
        let last = seen.last().unwrap();
        assert_eq!(last.done_bytes, last.total_bytes);
        assert_eq!(last.total_bytes, manifest.total_bytes());
        assert!(seen.windows(2).all(|w| w[0].done_bytes <= w[1].done_bytes));
        let paths = ModelPaths {
            root: dir.path().to_path_buf(),
        };
        assert_eq!(
            std::fs::read(paths.speak_dir().join("tts.json")).unwrap(),
            b"{\"a\":1}"
        );
        assert_eq!(
            std::fs::read(paths.speak_dir().join("model.onnx")).unwrap(),
            b"onnx!"
        );
        assert_eq!(
            std::fs::read(dir.path().join("vad").join("silero_vad.onnx")).unwrap(),
            vad
        );
        assert!(leftovers(dir.path()).is_empty());

        // A second run changes nothing and still ends on the full count.
        let again = Mutex::new(Vec::<Progress>::new());
        install(
            dir.path(),
            &manifest,
            |p| again.lock().unwrap().push(p),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        let again = again.into_inner().unwrap();
        assert_eq!(again.last().unwrap().done_bytes, manifest.total_bytes());
    }

    #[tokio::test]
    async fn a_wrong_hash_installs_nothing() {
        let tts = archive("speak", &[("tts.json", b"{}")]);
        let (base, _s) = start(
            HashMap::from([("speak.tar.bz2".into(), tts.clone())]),
            Mode::Normal,
        )
        .await;
        let mut f = entry("speak", &base, "speak.tar.bz2", &tts, true);
        f.sha256 = sha_hex(b"something else");
        let manifest = Manifest { files: vec![f] };
        let dir = tempfile::tempdir().unwrap();

        let err = install(dir.path(), &manifest, nothing, CancellationToken::new())
            .await
            .unwrap_err();
        assert!(matches!(err, ModelError::WrongHash { ref file } if file == "speak"));
        assert!(!installed(dir.path(), &manifest));
        assert!(!dir.path().join("speak").exists());
        assert!(!dir.path().join("speak.ok").exists());
        assert!(leftovers(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn a_cut_download_resumes() {
        // Noise, so the archive does not compress to a few bytes.
        let noise: Vec<u8> = (0..200_000u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
            .collect();
        let tts = archive("speak", &[("big.bin", &noise)]);
        let (base, server) = start(
            HashMap::from([("speak.tar.bz2".into(), tts.clone())]),
            Mode::CutFirst,
        )
        .await;
        let manifest = Manifest {
            files: vec![entry("speak", &base, "speak.tar.bz2", &tts, true)],
        };
        let dir = tempfile::tempdir().unwrap();

        let first = install(dir.path(), &manifest, nothing, CancellationToken::new()).await;
        assert!(matches!(first, Err(ModelError::Network(_))));
        assert!(!installed(dir.path(), &manifest));
        let part = std::fs::metadata(dir.path().join("speak.part"))
            .unwrap()
            .len();
        assert!(part > 0 && part < tts.len() as u64);

        install(dir.path(), &manifest, nothing, CancellationToken::new())
            .await
            .unwrap();
        assert!(installed(dir.path(), &manifest));
        let ranges = server.ranges.lock().unwrap();
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0], None);
        assert_eq!(ranges[1], Some(format!("bytes={part}-")));
        assert!(dir.path().join("speak").join("big.bin").is_file());
    }

    #[tokio::test]
    async fn a_server_that_ignores_range_still_ends_with_a_good_file() {
        let tts = archive("speak", &[("tts.json", b"{\"ok\":true}")]);
        let (base, server) = start(
            HashMap::from([("speak.tar.bz2".into(), tts.clone())]),
            Mode::IgnoreRange,
        )
        .await;
        let manifest = Manifest {
            files: vec![entry("speak", &base, "speak.tar.bz2", &tts, true)],
        };
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("speak.part"), &tts[..tts.len() / 2]).unwrap();

        install(dir.path(), &manifest, nothing, CancellationToken::new())
            .await
            .unwrap();
        assert!(installed(dir.path(), &manifest));
        assert!(server.ranges.lock().unwrap()[0].is_some());
        assert_eq!(
            std::fs::read(dir.path().join("speak").join("tts.json")).unwrap(),
            b"{\"ok\":true}"
        );
    }

    #[tokio::test]
    async fn a_part_file_larger_than_expected_is_started_again() {
        let tts = archive("speak", &[("tts.json", b"{}")]);
        let (base, _s) = start(
            HashMap::from([("speak.tar.bz2".into(), tts.clone())]),
            Mode::Normal,
        )
        .await;
        let manifest = Manifest {
            files: vec![entry("speak", &base, "speak.tar.bz2", &tts, true)],
        };
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("speak.part"), vec![1u8; tts.len() + 50]).unwrap();

        install(dir.path(), &manifest, nothing, CancellationToken::new())
            .await
            .unwrap();
        assert!(installed(dir.path(), &manifest));
    }

    #[tokio::test]
    async fn cancel_stops_and_leaves_no_install() {
        let tts = archive("speak", &[("big.bin", &vec![3u8; 100_000])]);
        let (base, _s) = start(
            HashMap::from([("speak.tar.bz2".into(), tts.clone())]),
            Mode::Normal,
        )
        .await;
        let manifest = Manifest {
            files: vec![entry("speak", &base, "speak.tar.bz2", &tts, true)],
        };
        let dir = tempfile::tempdir().unwrap();
        let token = CancellationToken::new();
        let t = token.clone();

        let err = install(dir.path(), &manifest, move |_| t.cancel(), token)
            .await
            .unwrap_err();
        assert!(matches!(err, ModelError::Cancelled));
        assert!(!installed(dir.path(), &manifest));
        assert!(!dir.path().join("speak").exists());
    }

    #[tokio::test]
    async fn an_archive_that_escapes_its_folder_is_refused() {
        let evil = raw_archive("../evil", tar::EntryType::Regular, None);
        let (base, _s) = start(
            HashMap::from([("speak.tar.bz2".into(), evil.clone())]),
            Mode::Normal,
        )
        .await;
        let manifest = Manifest {
            files: vec![entry("speak", &base, "speak.tar.bz2", &evil, true)],
        };
        let outer = tempfile::tempdir().unwrap();
        let root = outer.path().join("models");

        let err = install(&root, &manifest, nothing, CancellationToken::new())
            .await
            .unwrap_err();
        assert!(matches!(err, ModelError::Disk(_)), "{err:?}");
        assert!(!installed(&root, &manifest));
        assert!(!outer.path().join("evil").exists());
        assert!(!root.join("evil").exists());
        assert!(!root.join("speak").exists());
    }

    #[tokio::test]
    async fn an_archive_with_a_link_is_refused() {
        for kind in [tar::EntryType::Symlink, tar::EntryType::Link] {
            let bad = raw_archive("speak/link", kind, Some("/etc/passwd"));
            let (base, _s) = start(
                HashMap::from([("speak.tar.bz2".into(), bad.clone())]),
                Mode::Normal,
            )
            .await;
            let manifest = Manifest {
                files: vec![entry("speak", &base, "speak.tar.bz2", &bad, true)],
            };
            let dir = tempfile::tempdir().unwrap();

            let err = install(dir.path(), &manifest, nothing, CancellationToken::new())
                .await
                .unwrap_err();
            assert!(matches!(err, ModelError::Disk(_)), "{err:?}");
            assert!(!installed(dir.path(), &manifest));
            assert!(!dir.path().join("speak").exists());
        }
    }

    #[tokio::test]
    async fn an_archive_without_one_top_folder_unpacks_directly() {
        let enc = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
        let mut b = tar::Builder::new(enc);
        for (n, d) in [("a.txt", &b"A"[..]), ("b.txt", &b"B"[..])] {
            let mut h = tar::Header::new_gnu();
            h.set_size(1);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, n, d).unwrap();
        }
        let flat = b.into_inner().unwrap().finish().unwrap();
        let (base, _s) = start(
            HashMap::from([("w.tar.bz2".into(), flat.clone())]),
            Mode::Normal,
        )
        .await;
        let manifest = Manifest {
            files: vec![entry("wake", &base, "w.tar.bz2", &flat, true)],
        };
        let dir = tempfile::tempdir().unwrap();

        install(dir.path(), &manifest, nothing, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(dir.path().join("wake").join("a.txt")).unwrap(),
            b"A"
        );
        assert_eq!(
            std::fs::read(dir.path().join("wake").join("b.txt")).unwrap(),
            b"B"
        );
    }

    #[tokio::test]
    async fn remove_deletes_what_install_laid_down() {
        let vad = b"v".to_vec();
        let (base, _s) = start(
            HashMap::from([("silero_vad.onnx".into(), vad.clone())]),
            Mode::Normal,
        )
        .await;
        let manifest = Manifest {
            files: vec![entry("vad", &base, "silero_vad.onnx", &vad, false)],
        };
        let dir = tempfile::tempdir().unwrap();
        install(dir.path(), &manifest, nothing, CancellationToken::new())
            .await
            .unwrap();
        remove(dir.path()).unwrap();
        assert!(!installed(dir.path(), &manifest));
        assert!(!dir.path().join("vad").exists());
        remove(dir.path()).unwrap();
    }

    #[test]
    fn the_pinned_manifest_is_whole() {
        let m = Manifest::pinned();
        assert_eq!(m.files.len(), 4);
        for f in &m.files {
            assert_eq!(f.sha256.len(), 64, "{}", f.name);
            assert!(
                f.sha256.chars().all(|c| c.is_ascii_hexdigit()),
                "{}",
                f.name
            );
            assert!(f
                .url
                .starts_with("https://github.com/k2-fsa/sherpa-onnx/releases/download/"));
        }
        assert_eq!(m.total_bytes(), 258_311_120);
    }
}
