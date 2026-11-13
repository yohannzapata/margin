//! Local model runtime: downloads (once, hash-verified) and supervises a
//! llama.cpp `llama-server` bound to 127.0.0.1. Nothing is exposed to the network.

use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct ModelInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub file: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub size: u64,
}

/// Apache-2.0 models, pinned by hash. Evaluated on tests/quality_cases.json
/// (bench/QUALITY.md): Qwen3.5-0.8B rejected (changed meaning), Qwen3.5-2B no
/// better than 1.5B while slower.
pub const MODELS: &[ModelInfo] = &[
    ModelInfo {
        id: "qwen2.5-1.5b",
        name: "Fast: Qwen2.5 1.5B (1.1 GB)",
        file: "qwen2.5-1.5b-instruct-q4_k_m.gguf",
        url: "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf",
        sha256: "6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e",
        size: 1_117_320_736,
    },
    ModelInfo {
        id: "qwen3-4b-2507",
        name: "Best: Qwen3 4B (2.4 GB)",
        file: "Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        url: "https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/main/Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        sha256: "3605803b982cb64aead44f6c1b2ae36e3acdb41d8e46c8a94c6533bc4c67e597",
        size: 2_497_281_120,
    },
];

/// Settings value meaning "pick for this PC".
pub const AUTO: &str = "auto";

/// The 4B model needs a GPU to be responsive (CPU timings in bench/QUALITY.md);
/// with ≥ 6 GB of dedicated video memory it fits alongside its cache.
pub fn recommended(use_gpu: bool) -> &'static ModelInfo {
    if use_gpu && dedicated_vram_gb() >= 6 { &MODELS[1] } else { &MODELS[0] }
}

/// Largest dedicated video memory of any hardware adapter, in GB.
pub fn dedicated_vram_gb() -> u64 {
    use windows::Win32::Graphics::Dxgi::*;
    unsafe {
        let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else { return 0 };
        let mut best = 0u64;
        let mut i = 0;
        while let Ok(adapter) = factory.EnumAdapters1(i) {
            i += 1;
            if let Ok(d) = adapter.GetDesc1() {
                if d.Flags & (DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) == 0 {
                    best = best.max(d.DedicatedVideoMemory as u64);
                }
            }
        }
        // Report e.g. a "6 GB" card (6141 MiB usable) as 6.
        (best + (512 << 20)) >> 30
    }
}

/// llama.cpp (MIT). The Vulkan build uses any GPU vendor and falls back to CPU.
const RUNTIME_URL: &str = "https://github.com/ggml-org/llama.cpp/releases/download/b12340/llama-b12340-bin-win-vulkan-x64.zip";
const RUNTIME_SHA256: &str = "a2c7a2bfaaf57b5357f84cf584603d088143a5be08c68cb623e6d9bb7db664e8";

pub static STATUS: Mutex<String> = Mutex::new(String::new());

pub fn set_status(s: impl Into<String>) {
    *STATUS.lock().unwrap() = s.into();
}

pub fn status() -> String {
    STATUS.lock().unwrap().clone()
}

pub fn data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| ".".into()).join("Margin")
}

pub fn model(id: &str, use_gpu: bool) -> &'static ModelInfo {
    MODELS.iter().find(|m| m.id == id).unwrap_or_else(|| recommended(use_gpu))
}

fn download(url: &str, dest: &Path, sha256: &str, what: &str) -> Result<(), String> {
    std::fs::create_dir_all(dest.parent().unwrap()).map_err(|e| e.to_string())?;
    let part = dest.with_extension("part");
    let agent = ureq::Agent::config_builder().https_only(true).timeout_connect(Some(Duration::from_secs(15))).build().new_agent();
    let resp = agent.get(url).call().map_err(|e| format!("download failed: {e}"))?;
    let total: u64 = resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut reader = resp.into_body().into_reader();
    let mut file = std::fs::File::create(&part).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let (mut done, mut last) = (0u64, Instant::now());
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        if last.elapsed() > Duration::from_millis(500) {
            last = Instant::now();
            let pct = if total > 0 { done * 100 / total } else { 0 };
            set_status(format!("Downloading {what}… {pct}%"));
        }
    }
    drop(file);
    let got = format!("{:x}", hasher.finalize());
    if got != sha256 {
        let _ = std::fs::remove_file(&part);
        return Err(format!("{what}: checksum mismatch, download discarded"));
    }
    std::fs::rename(&part, dest).map_err(|e| e.to_string())
}

/// Make sure runtime + model exist locally. Blocking; call from a worker thread.
pub fn ensure_installed(model: &ModelInfo) -> Result<(PathBuf, PathBuf), String> {
    let dir = data_dir();
    let server = dir.join("runtime").join("llama-server.exe");
    if !server.exists() {
        let zip = dir.join("runtime").join("llama.zip");
        download(RUNTIME_URL, &zip, RUNTIME_SHA256, "AI runtime")?;
        // Windows 10+ ships bsdtar, which extracts zip archives.
        let ok = Command::new("tar").arg("-xf").arg(&zip).arg("-C").arg(dir.join("runtime"))
            .creation_flags(0x0800_0000).status().map(|s| s.success()).unwrap_or(false);
        let _ = std::fs::remove_file(&zip);
        if !ok || !server.exists() {
            return Err("could not unpack the AI runtime".into());
        }
    }
    let model_path = dir.join("models").join(model.file);
    let size_ok = std::fs::metadata(&model_path).map(|m| m.len() == model.size).unwrap_or(false);
    if !size_ok {
        download(model.url, &model_path, model.sha256, "writing model")?;
    }
    Ok((server, model_path))
}

pub struct LocalServer {
    child: Child,
    pub port: u16,
    _job: windows::Win32::Foundation::HANDLE,
}

impl LocalServer {
    pub fn start(server: &Path, model: &Path, use_gpu: bool) -> Result<Self, String> {
        let port = free_port()?;
        let threads = (std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4) / 2).clamp(2, 8);
        let mut cmd = Command::new(server);
        cmd.arg("-m").arg(model)
            .args(["--host", "127.0.0.1", "--port", &port.to_string()])
            // One slot per request kind (predict, correct, improve, rephrase): each keeps
            // its own instructions cached, so alternating kinds doesn't re-read them.
            // 3072 tokens per slot fits instructions + examples + capped context + output.
            .args(["-c", "12288", "-np", "4", "-t", &threads.to_string()])
            .args(["--reasoning", "off"])
            .args(["-ngl", if use_gpu { "99" } else { "0" }])
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            // CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS: never compete with the foreground app.
            .creation_flags(0x0800_0000 | 0x0000_4000);
        let child = cmd.spawn().map_err(|e| format!("could not start AI runtime: {e}"))?;
        let job = unsafe { kill_on_close_job(&child) };
        let s = LocalServer { child, port, _job: job };
        let agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(1))).build().new_agent();
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(90) {
            if let Ok(mut r) = agent.get(format!("http://127.0.0.1:{port}/health")).call() {
                if r.body_mut().read_to_string().map(|b| b.contains("ok")).unwrap_or(false) {
                    return Ok(s);
                }
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        Err("AI runtime did not start".into())
    }

    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for LocalServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn free_port() -> Result<u16, String> {
    let l = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    Ok(l.local_addr().map_err(|e| e.to_string())?.port())
}

/// Tie the child to a job object so it dies with us, even on a crash.
unsafe fn kill_on_close_job(child: &Child) -> windows::Win32::Foundation::HANDLE {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::*;
    unsafe {
        let Ok(job) = CreateJobObjectW(None, None) else { return HANDLE::default() };
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let _ = SetInformationJobObject(job, JobObjectExtendedLimitInformation, &info as *const _ as _, size_of_val(&info) as u32);
        let _ = AssignProcessToJobObject(job, HANDLE(child.as_raw_handle()));
        job
    }
}
